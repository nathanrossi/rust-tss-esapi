// Copyright 2020 Contributors to the Parsec project.
// SPDX-License-Identifier: Apache-2.0
mod handle_manager;
use crate::{
    abstraction::cipher::Cipher,
    attributes::SessionAttributesBuilder,
    constants::{CapabilityType, PropertyTag, SessionType},
    handles::ObjectHandle,
    interface_types::algorithm::HashingAlgorithm,
    session::Session,
    structures::CapabilityData,
    tss2_esys::*,
    Error, Result, Tcti, WrapperErrorKind as ErrorKind,
};
use handle_manager::HandleManager;
use log::{error, info};
use mbox::MBox;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::ffi::CString;
use std::ptr::null_mut;

/// Safe abstraction over an ESYS_CONTEXT.
///
/// Serves as a low-level abstraction interface to the TPM, providing a thin wrapper around the
/// `unsafe` FFI calls. It is meant for more advanced uses of the TSS where control over all
/// parameters is necessary or important.
///
/// The methods it exposes take the parameters advertised by the specification, with some of the
/// parameters being passed as generated by `bindgen` and others in a more convenient/Rust-efficient
/// way.
///
/// The context also keeps track of all object allocated and deallocated through it and, before
/// being dropped, will attempt to close all outstanding handles. However, care must be taken by
/// the client to not exceed the maximum number of slots available from the RM.
///
/// Code safety-wise, the methods should cover the two kinds of problems that might arise:
/// * in terms of memory safety, all parameters passed down to the TSS are verified and the library
/// stack is then trusted to provide back valid outputs
/// * in terms of thread safety, all methods require a mutable reference to the context object,
/// ensuring that no two threads can use the context at the same time for an operation (barring use
/// of `unsafe` constructs on the client side)
/// More testing and verification will be added to ensure this.
///
/// For most methods, if the wrapped TSS call fails and returns a non-zero `TPM2_RC`, a
/// corresponding `Tss2ResponseCode` will be created and returned as an `Error`. Wherever this is
/// not the case or additional error types can be returned, the method definition should mention
/// it.
#[derive(Debug)]
pub struct Context {
    /// Handle for the ESYS context object owned through an Mbox.
    /// Wrapping the handle in an optional Mbox is done to allow the `Context` to be closed properly when the `Context` structure is dropped.
    esys_context: Option<MBox<ESYS_CONTEXT>>,
    sessions: (Option<Session>, Option<Session>, Option<Session>),
    /// TCTI context handle associated with the ESYS context.
    /// As with the ESYS context, an optional Mbox wrapper allows the context to be deallocated.
    tcti_context: Option<MBox<TSS2_TCTI_CONTEXT>>,
    /// Handle manager that keep tracks of the state of the handles and how they are to be
    /// disposed.
    handle_manager: HandleManager,
    /// A cache of determined TPM limits
    cached_tpm_properties: HashMap<PropertyTag, u32>,
}

// Implementation of the TPM commands
mod tpm_commands;
// Implementation of the ESAPI session administration
// functions.
mod session_administration;
// Implementation of the general ESAPI ESYS_TR functions
mod general_esys_tr;

impl Context {
    /// Create a new ESYS context based on the desired TCTI
    ///
    /// # Safety
    /// * the client is responsible for ensuring that the context can be initialized safely,
    /// threading-wise
    ///
    /// # Errors
    /// * if either `Tss2_TctiLdr_Initiialize` or `Esys_Initialize` fail, a corresponding
    /// Tss2ResponseCode will be returned
    pub unsafe fn new(tcti: Tcti) -> Result<Self> {
        let mut esys_context = null_mut();
        let mut tcti_context = null_mut();

        let tcti_name_conf = CString::try_from(tcti)?; // should never panic

        let ret = Tss2_TctiLdr_Initialize(tcti_name_conf.as_ptr(), &mut tcti_context);
        let ret = Error::from_tss_rc(ret);
        if !ret.is_success() {
            error!("Error when creating a TCTI context: {}", ret);
            return Err(ret);
        }
        let mut tcti_context = Some(MBox::from_raw(tcti_context));

        let ret = Esys_Initialize(
            &mut esys_context,
            tcti_context.as_mut().unwrap().as_mut_ptr(), // will not panic as per how tcti_context is initialised
            null_mut(),
        );
        let ret = Error::from_tss_rc(ret);

        if ret.is_success() {
            let esys_context = Some(MBox::from_raw(esys_context));
            let context = Context {
                esys_context,
                sessions: (None, None, None),
                tcti_context,
                handle_manager: HandleManager::new(),
                cached_tpm_properties: HashMap::new(),
            };
            Ok(context)
        } else {
            error!("Error when creating a new context: {}", ret);
            Err(ret)
        }
    }

    /// Set the sessions to be used in calls to ESAPI.
    ///
    /// # Details
    /// In some calls these sessions are optional and in others
    /// they are required.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use tss_esapi::{Context, Tcti,
    /// #     constants::SessionType,
    /// #     interface_types::algorithm::HashingAlgorithm,
    /// #     abstraction::cipher::Cipher,
    /// # };
    /// # // Create context
    /// # let mut context = unsafe {
    /// #     Context::new(
    /// #        Tcti::from_environment_variable().expect("Failed to get TCTI"),
    /// #     ).expect("Failed to create Context")
    /// # };
    /// // Create auth session without key_handle, bind_handle
    /// // and Nonce
    /// let auth_session = context
    ///     .start_auth_session(
    ///         None,
    ///         None,
    ///         None,
    ///         SessionType::Hmac,
    ///         Cipher::aes_256_cfb(),
    ///         HashingAlgorithm::Sha256,
    ///     )
    ///     .expect("Failed to create session");
    ///
    /// // Set auth_session as the first handle to be
    /// // used in calls to ESAPI no matter if it None
    /// // or not.
    /// context.set_sessions((auth_session, None, None));
    /// ```
    pub fn set_sessions(
        &mut self,
        session_handles: (Option<Session>, Option<Session>, Option<Session>),
    ) {
        self.sessions = session_handles;
    }

    /// Clears any sessions that have been set
    ///
    /// This will result in the None handle being
    /// used in all calls to ESAPI.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use tss_esapi::{Context, Tcti, session::Session};
    /// # // Create context
    /// # let mut context = unsafe {
    /// #     Context::new(
    /// #         Tcti::from_environment_variable().expect("Failed to get TCTI"),
    /// #     ).expect("Failed to create Context")
    /// # };
    /// // Use password session for auth
    /// context.set_sessions((Some(Session::Password), None, None));
    ///
    /// // Clear auth sessions
    /// context.clear_sessions();
    /// ```
    pub fn clear_sessions(&mut self) {
        self.sessions = (None, None, None)
    }

    /// Returns the sessions that are currently set.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use tss_esapi::{Context, Tcti, session::Session};
    /// # // Create context
    /// # let mut context = unsafe {
    /// #     Context::new(
    /// #         Tcti::from_environment_variable().expect("Failed to get TCTI"),
    /// #     ).expect("Failed to create Context")
    /// # };
    /// // Use password session for auth
    /// context.set_sessions((Some(Session::Password), None, None));
    ///
    /// // Retreive sessions in use
    /// let (session_1, session_2, session_3) = context.sessions();
    /// assert_eq!(Some(Session::Password), session_1);
    /// assert_eq!(None, session_2);
    /// assert_eq!(None, session_3);
    /// ```
    pub fn sessions(&self) -> (Option<Session>, Option<Session>, Option<Session>) {
        self.sessions
    }

    /// Execute the closure in f with the specified set of sessions, and sets the original sessions back afterwards
    pub fn execute_with_sessions<F, T>(
        &mut self,
        session_handles: (Option<Session>, Option<Session>, Option<Session>),
        f: F,
    ) -> T
    where
        // We only need to call f once, so it can be FnOnce
        F: FnOnce(&mut Context) -> T,
    {
        let oldses = self.sessions();
        self.set_sessions(session_handles);

        let res = f(self);

        self.set_sessions(oldses);

        res
    }

    /// Executes the closure with a single session set, and the others set to None
    pub fn execute_with_session<F, T>(&mut self, session_handle: Option<Session>, f: F) -> T
    where
        // We only need to call f once, so it can be FnOnce
        F: FnOnce(&mut Context) -> T,
    {
        self.execute_with_sessions((session_handle, None, None), f)
    }

    /// Executes the closure without any sessions,
    pub fn execute_without_session<F, T>(&mut self, f: F) -> T
    where
        // We only need to call f once, so it can be FnOnce
        F: FnOnce(&mut Context) -> T,
    {
        self.execute_with_sessions((None, None, None), f)
    }

    /// Executes the closure with a newly generated empty session
    ///
    /// # Details
    /// The session attributes for the generated empty session that
    /// is used to execute closure will have the attributes decrypt
    /// and encrypt set.
    pub fn execute_with_nullauth_session<F, T>(&mut self, f: F) -> Result<T>
    where
        // We only need to call f once, so it can be FnOnce
        F: FnOnce(&mut Context) -> Result<T>,
    {
        let session = match self.start_auth_session(
            None,
            None,
            None,
            SessionType::Hmac,
            Cipher::aes_128_cfb(),
            HashingAlgorithm::Sha256,
        )? {
            Some(ses) => ses,
            None => return Err(Error::local_error(ErrorKind::WrongValueFromTpm)),
        };

        let (session_attributes, session_attributes_mask) = SessionAttributesBuilder::new()
            .with_decrypt(true)
            .with_encrypt(true)
            .build();
        self.tr_sess_set_attributes(session, session_attributes, session_attributes_mask)?;

        let res = self.execute_with_session(Some(session), f);

        self.flush_context(session.handle().into())?;

        res
    }

    /// Execute the closure in f, and clear up the object after it's done before returning the result
    /// This is a convenience function that ensures object is always closed, even if an error occurs
    pub fn execute_with_temporary_object<F, T>(&mut self, object: ObjectHandle, f: F) -> Result<T>
    where
        F: FnOnce(&mut Context, ObjectHandle) -> Result<T>,
    {
        let res = f(self, object);

        self.flush_context(object)?;

        res
    }

    /// Determine a TPM property
    ///
    /// # Details
    /// Returns the value of the provided `TpmProperty` if
    /// the TPM has a value for it else None will be returned.
    /// If None is returned then use default from specification.
    ///
    /// # Errors
    /// If the TPM returns a value that is wrong when
    /// its capabilities is being retrieved then a
    /// `WrongValueFromTpm` is returned.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use tss_esapi::{Context, Tcti, constants::PropertyTag};
    /// # use std::str::FromStr;
    /// # // Create context
    /// # let mut context = unsafe {
    /// #     Context::new(
    /// #         Tcti::from_environment_variable().expect("Failed to get TCTI"),
    /// #     ).expect("Failed to create Context")
    /// # };
    /// let rev = context
    ///     .get_tpm_property(PropertyTag::Revision)
    ///     .expect("Wrong value from TPM")
    ///     .expect("Value is not supported");
    /// ```
    pub fn get_tpm_property(&mut self, property: PropertyTag) -> Result<Option<u32>> {
        // Return cahced value if it exists
        if let Some(&val) = self.cached_tpm_properties.get(&property) {
            return Ok(Some(val));
        }

        let (capabs, _) = self.execute_without_session(|ctx| {
            ctx.get_capability(CapabilityType::TPMProperties, property.into(), 4)
        })?;
        let props = match capabs {
            CapabilityData::TPMProperties(props) => props,
            _ => return Err(Error::WrapperError(ErrorKind::WrongValueFromTpm)),
        };
        for (key, val) in props.iter() {
            if let Ok(key) = PropertyTag::try_from(*key) {
                // If we are returned a property we don't know, just ignore it
                let _ = self.cached_tpm_properties.insert(key, *val);
            }
        }

        if let Some(val) = self.cached_tpm_properties.get(&property) {
            return Ok(Some(*val));
        }
        Ok(None)
    }

    // ////////////////////////////////////////////////////////////////////////
    //  Private Methods Section
    // ////////////////////////////////////////////////////////////////////////

    /// Returns a mutable reference to the native ESYS context handle.
    fn mut_context(&mut self) -> *mut ESYS_CONTEXT {
        self.esys_context.as_mut().unwrap().as_mut_ptr() // will only fail if called from Drop after .take()
    }

    /// Internal function for retrieving the ESYS session handle for
    /// the optional session 1.
    fn optional_session_1(&self) -> ESYS_TR {
        Session::handle_from_option(self.sessions.0).into()
    }

    /// Internal function for retrieving the ESYS session handle for
    /// the optional session 2.
    fn optional_session_2(&self) -> ESYS_TR {
        Session::handle_from_option(self.sessions.1).into()
    }

    /// Internal function for retrieving the ESYS session handle for
    /// the optional session 3.
    fn optional_session_3(&self) -> ESYS_TR {
        Session::handle_from_option(self.sessions.2).into()
    }

    /// Function that returns the required
    /// session handle 1 if it is available else
    /// returns an error.
    fn required_session_1(&self) -> Result<ESYS_TR> {
        self.sessions.0.map(|v| v.handle().into()).ok_or_else(|| {
            error!("Missing session handle for authorization (authSession1 = None)");
            Error::local_error(ErrorKind::MissingAuthSession)
        })
    }

    /// Function that returns the required
    /// session handle 2 if it is available else
    /// returns an error.
    fn required_session_2(&self) -> Result<ESYS_TR> {
        self.sessions.1.map(|v| v.handle().into()).ok_or_else(|| {
            error!("Missing session handle for authorization (authSession2 = None)");
            Error::local_error(ErrorKind::MissingAuthSession)
        })
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        info!("Closing context.");

        // Flush handles
        for handle in self.handle_manager.handles_to_flush() {
            info!("Flushing handle {}", ESYS_TR::from(handle));
            if let Err(e) = self.flush_context(handle) {
                error!("Error when dropping the context: {}", e);
            }
        }

        // Close handles
        for handle in self.handle_manager.handles_to_close().iter_mut() {
            info!("Closing handle {}", ESYS_TR::from(*handle));
            if let Err(e) = self.tr_close(handle) {
                error!("Error when dropping context: {}.", e);
            }
        }

        // Check if all handles have been cleaned up proeprly.
        if self.handle_manager.has_open_handles() {
            error!("Not all handles have had their resources successfully released");
        }

        let esys_context = self.esys_context.take().unwrap(); // should not fail based on how the context is initialised/used
        let tcti_context = self.tcti_context.take().unwrap(); // should not fail based on how the context is initialised/used

        // Close the TCTI context.
        unsafe { Tss2_TctiLdr_Finalize(&mut tcti_context.into_raw()) };

        // Close the context.
        unsafe { Esys_Finalize(&mut esys_context.into_raw()) };
        info!("Context closed.");
    }
}
