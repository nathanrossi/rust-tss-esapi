// Copyright 2019 Contributors to the Parsec project.
// SPDX-License-Identifier: Apache-2.0
//! Utility module
//!
//! This module mostly contains helper elements meant to act as either wrappers around FFI-level
//! structs or builders for them, along with other convenience elements.
//! The naming structure usually takes the names inherited from the TSS spec and applies Rust
//! guidelines to them. Structures that are meant to act as builders have `Builder` appended to
//! type name. Unions are converted to Rust `enum`s by dropping the `TPMU` qualifier and appending
//! `Union`.
pub mod algorithm_specifiers;

use crate::constants::*;
use crate::response_code::{Error, Result, WrapperErrorKind};
use crate::tss2_esys::*;
use algorithm_specifiers::{Cipher, HashingAlgorithm};
use bitfield::bitfield;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::convert::{TryFrom, TryInto};

/// Helper for building `TPM2B_PUBLIC` values out of its subcomponents.
///
/// Currently the implementation is incomplete, focusing on creating objects of RSA type.
// Most of the field types are from bindgen which does not implement Debug on them.
#[allow(missing_debug_implementations)]
pub struct Tpm2BPublicBuilder {
    type_: Option<TPMI_ALG_PUBLIC>,
    name_alg: TPMI_ALG_HASH,
    object_attributes: ObjectAttributes,
    auth_policy: TPM2B_DIGEST,
    parameters: Option<PublicParmsUnion>,
    unique: Option<PublicIdUnion>,
}

impl Tpm2BPublicBuilder {
    /// Create a new builder with default (i.e. empty or null) placeholder values.
    pub fn new() -> Self {
        Tpm2BPublicBuilder {
            type_: None,
            name_alg: TPM2_ALG_NULL,
            object_attributes: ObjectAttributes(0),
            auth_policy: Default::default(),
            parameters: None,
            unique: None,
        }
    }

    /// Set the type of the object to be built.
    pub fn with_type(mut self, type_: TPMI_ALG_PUBLIC) -> Self {
        self.type_ = Some(type_);
        self
    }

    /// Set the algorithm used to derive the object name.
    pub fn with_name_alg(mut self, name_alg: TPMI_ALG_HASH) -> Self {
        self.name_alg = name_alg;
        self
    }

    /// Set the object attributes.
    pub fn with_object_attributes(mut self, obj_attr: ObjectAttributes) -> Self {
        self.object_attributes = obj_attr;
        self
    }

    /// Set the authentication policy hash for the object.
    pub fn with_auth_policy(mut self, size: u16, buffer: [u8; 64]) -> Self {
        self.auth_policy = TPM2B_DIGEST { size, buffer };
        self
    }

    /// Set the public parameters of the object.
    pub fn with_parms(mut self, parameters: PublicParmsUnion) -> Self {
        self.parameters = Some(parameters);
        self
    }

    /// Set the unique value for the object.
    pub fn with_unique(mut self, unique: PublicIdUnion) -> Self {
        self.unique = Some(unique);
        self
    }

    /// Build an object with the previously provided parameters.
    ///
    /// The paramters are checked for consistency based on the TSS specifications for the
    /// `TPM2B_PUBLIC` structure and for the structures nested within it.
    ///
    /// Currently only objects of type `TPM2_ALG_RSA` are supported.
    ///
    /// # Errors
    /// * if no public parameters are provided, `ParamsMissing` wrapper error is returned
    /// * if a public parameter type or public ID type is provided that is incosistent with the
    /// object type provided, `InconsistentParams` wrapper error is returned
    ///
    /// # Panics
    /// * will panic on unsupported platforms (i.e. on 8 bit processors)
    pub fn build(self) -> Result<TPM2B_PUBLIC> {
        match self.type_ {
            Some(TPM2_ALG_RSA) => {
                // RSA key
                let parameters;
                let unique;
                if let Some(PublicParmsUnion::RsaDetail(parms)) = self.parameters {
                    parameters = TPMU_PUBLIC_PARMS { rsaDetail: parms };
                } else if self.parameters.is_none() {
                    return Err(Error::local_error(WrapperErrorKind::ParamsMissing));
                } else {
                    return Err(Error::local_error(WrapperErrorKind::InconsistentParams));
                }

                if let Some(PublicIdUnion::Rsa(rsa_unique)) = self.unique {
                    unique = TPMU_PUBLIC_ID { rsa: *rsa_unique };
                } else if self.unique.is_none() {
                    unique = Default::default();
                } else {
                    return Err(Error::local_error(WrapperErrorKind::InconsistentParams));
                }

                Ok(TPM2B_PUBLIC {
                    size: std::mem::size_of::<TPMT_PUBLIC>()
                        .try_into()
                        .expect("Failed to convert usize to u16"), // should not fail on valid targets
                    publicArea: TPMT_PUBLIC {
                        type_: self.type_.unwrap(), // cannot fail given that this is inside a match on `type_`
                        nameAlg: self.name_alg,
                        objectAttributes: self.object_attributes.0,
                        authPolicy: self.auth_policy,
                        parameters,
                        unique,
                    },
                })
            }
            _ => Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
        }
    }
}

impl Default for Tpm2BPublicBuilder {
    fn default() -> Self {
        Tpm2BPublicBuilder::new()
    }
}

/// Builder for `TPMS_RSA_PARMS` values.
// Most of the field types are from bindgen which does not implement Debug on them.
#[allow(missing_debug_implementations)]
#[derive(Copy, Clone, Default)]
pub struct TpmsRsaParmsBuilder {
    /// Symmetric cipher to be used in conjuction with the key
    pub symmetric: Option<TPMT_SYM_DEF_OBJECT>,
    /// Asymmetric scheme to be used for key operations
    pub scheme: Option<AsymSchemeUnion>,
    /// Size of key, in bits
    pub key_bits: TPMI_RSA_KEY_BITS,
    /// Public exponent of the key. A value of 0 defaults to 2 ^ 16 + 1
    pub exponent: u32,
    /// Flag indicating whether the key shall be used for signing
    pub for_signing: bool,
    /// Flag indicating whether the key shall be used for decryption
    pub for_decryption: bool,
    /// Flag indicating whether the key is restricted
    pub restricted: bool,
}

impl TpmsRsaParmsBuilder {
    /// Create parameters for a restricted decryption key
    pub fn new_restricted_decryption_key(
        symmetric: TPMT_SYM_DEF_OBJECT,
        key_bits: TPMI_RSA_KEY_BITS,
        exponent: u32,
    ) -> Self {
        TpmsRsaParmsBuilder {
            symmetric: Some(symmetric),
            scheme: Some(AsymSchemeUnion::AnySig(TPM2_ALG_NULL)),
            key_bits,
            exponent,
            for_signing: false,
            for_decryption: true,
            restricted: true,
        }
    }

    /// Create parameters for an unrestricted signing key
    pub fn new_unrestricted_signing_key(
        scheme: AsymSchemeUnion,
        key_bits: TPMI_RSA_KEY_BITS,
        exponent: u32,
    ) -> Self {
        TpmsRsaParmsBuilder {
            symmetric: None,
            scheme: Some(scheme),
            key_bits,
            exponent,
            for_signing: true,
            for_decryption: false,
            restricted: false,
        }
    }

    /// Build an object given the previously provded parameters.
    ///
    /// The only mandatory parameter is the asymmetric scheme.
    ///
    /// # Errors
    /// * if no asymmetric scheme is set, `ParamsMissing` wrapper error is returned.
    /// * if the `for_signing`, `for_decryption` and `restricted` parameters are
    /// inconsistent with the rest of the parameters, `InconsistentParams` wrapper
    /// error is returned
    pub fn build(self) -> Result<TPMS_RSA_PARMS> {
        if self.restricted && self.for_decryption {
            if self.symmetric.is_none() {
                return Err(Error::local_error(WrapperErrorKind::ParamsMissing));
            }
        } else if self.symmetric.is_some() {
            return Err(Error::local_error(WrapperErrorKind::InconsistentParams));
        }
        let symmetric = self.symmetric.unwrap_or_else(|| {
            let mut def: TPMT_SYM_DEF_OBJECT = Default::default();
            def.algorithm = TPM2_ALG_NULL;

            def
        });

        let scheme = self
            .scheme
            .ok_or_else(|| Error::local_error(WrapperErrorKind::ParamsMissing))?
            .get_rsa_scheme();
        if self.restricted {
            if self.for_signing
                && scheme.scheme != TPM2_ALG_RSAPSS
                && scheme.scheme != TPM2_ALG_RSASSA
            {
                return Err(Error::local_error(WrapperErrorKind::InconsistentParams));
            }

            if self.for_decryption && scheme.scheme != TPM2_ALG_NULL {
                return Err(Error::local_error(WrapperErrorKind::InconsistentParams));
            }
        } else {
            if self.for_decryption && self.for_signing && scheme.scheme != TPM2_ALG_NULL {
                return Err(Error::local_error(WrapperErrorKind::InconsistentParams));
            }
            if self.for_signing
                && scheme.scheme != TPM2_ALG_RSAPSS
                && scheme.scheme != TPM2_ALG_RSASSA
                && scheme.scheme != TPM2_ALG_NULL
            {
                return Err(Error::local_error(WrapperErrorKind::InconsistentParams));
            }

            if self.for_decryption
                && scheme.scheme != TPM2_ALG_RSAES
                && scheme.scheme != TPM2_ALG_OAEP
                && scheme.scheme != TPM2_ALG_NULL
            {
                return Err(Error::local_error(WrapperErrorKind::InconsistentParams));
            }
        }
        Ok(TPMS_RSA_PARMS {
            symmetric,
            scheme,
            keyBits: self.key_bits,
            exponent: self.exponent,
        })
    }
}

/// Supported sizes for RSA key modulus
pub const RSA_KEY_SIZES: [u16; 4] = [1024, 2048, 3072, 4096];

/// Builder for `TPMT_SYM_DEF` objects.
#[derive(Copy, Clone, Debug)]
pub struct TpmtSymDefBuilder {
    algorithm: Option<TPM2_ALG_ID>,
    key_bits: u16,
    mode: TPM2_ALG_ID,
}

impl TpmtSymDefBuilder {
    /// Create a new builder with default (i.e. empty or null) placeholder values.
    pub fn new() -> Self {
        TpmtSymDefBuilder {
            algorithm: None,
            key_bits: 0,
            mode: TPM2_ALG_NULL,
        }
    }

    /// Set the symmetric algorithm.
    pub fn with_algorithm(mut self, algorithm: TPM2_ALG_ID) -> Self {
        self.algorithm = Some(algorithm);
        self
    }

    /// Set the key length.
    pub fn with_key_bits(mut self, key_bits: TPM2_KEY_BITS) -> Self {
        self.key_bits = key_bits;
        self
    }

    /// Set the hash algorithm (applies when the symmetric algorithm is XOR).
    pub fn with_hash(mut self, hash: TPM2_ALG_ID) -> Self {
        self.key_bits = hash;
        self
    }

    /// Set the mode of the symmetric algorithm.
    pub fn with_mode(mut self, mode: TPM2_ALG_ID) -> Self {
        self.mode = mode;
        self
    }

    /// Build a TPMT_SYM_DEF given the previously provided parameters.
    ///
    /// # Errors
    /// * if an unrecognized symmetric algorithm type was set, `UnsupportedParam` wrapper error
    /// is returned.
    /// * if an algorithm is not explicitly set, `ParamsMissing` is returned
    pub fn build(self) -> Result<TPMT_SYM_DEF> {
        let (key_bits, mode) = self.bits_and_mode()?;

        Ok(TPMT_SYM_DEF {
            algorithm: self.algorithm.unwrap(), // bits_and_mode would return an Err if algorithm was missing
            keyBits: key_bits,
            mode,
        })
    }

    /// Build a TPMT_SYM_DEF_OBJECT given the previously provided parameters.
    ///
    /// # Errors
    /// * if an unrecognized symmetric algorithm type was set, `UnsupportedParam` wrapper error
    /// is returned.
    /// * if an algorithm is not explicitly set, `ParamsMissing` is returned
    pub fn build_object(self) -> Result<TPMT_SYM_DEF_OBJECT> {
        let (key_bits, mode) = self.bits_and_mode()?;

        Ok(TPMT_SYM_DEF_OBJECT {
            algorithm: self.algorithm.unwrap(), // bits_and_mode would return an Err if algorithm was missing
            keyBits: key_bits,
            mode,
        })
    }

    fn bits_and_mode(self) -> Result<(TPMU_SYM_KEY_BITS, TPMU_SYM_MODE)> {
        let key_bits;
        let mode;
        match self.algorithm {
            Some(TPM2_ALG_XOR) => {
                // Exclusive OR
                key_bits = TPMU_SYM_KEY_BITS {
                    exclusiveOr: self.key_bits,
                };
                mode = Default::default(); // NULL
            }
            Some(TPM2_ALG_AES) => {
                // AES
                key_bits = TPMU_SYM_KEY_BITS { aes: self.key_bits };
                mode = TPMU_SYM_MODE { aes: self.mode };
            }
            Some(TPM2_ALG_SM4) => {
                // SM4
                key_bits = TPMU_SYM_KEY_BITS { sm4: self.key_bits };
                mode = TPMU_SYM_MODE { sm4: self.mode };
            }
            Some(TPM2_ALG_CAMELLIA) => {
                // CAMELLIA
                key_bits = TPMU_SYM_KEY_BITS {
                    camellia: self.key_bits,
                };
                mode = TPMU_SYM_MODE {
                    camellia: self.mode,
                };
            }
            Some(TPM2_ALG_NULL) => {
                // NULL
                key_bits = Default::default();
                mode = Default::default();
            }
            None => return Err(Error::local_error(WrapperErrorKind::ParamsMissing)),
            _ => return Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
        }

        Ok((key_bits, mode))
    }

    /// Generate a `TPMT_SYM_DEF` object defining 256 bit AES in CFB mode.
    pub fn aes_256_cfb() -> TPMT_SYM_DEF {
        TPMT_SYM_DEF {
            algorithm: TPM2_ALG_AES,
            keyBits: TPMU_SYM_KEY_BITS { aes: 256 },
            mode: TPMU_SYM_MODE { aes: TPM2_ALG_CFB },
        }
    }

    /// Generate a `TPMT_SYM_DEF_OBJECT` object defining 256 bit AES in CFB mode.
    pub fn aes_256_cfb_object() -> TPMT_SYM_DEF_OBJECT {
        TPMT_SYM_DEF_OBJECT {
            algorithm: TPM2_ALG_AES,
            keyBits: TPMU_SYM_KEY_BITS { aes: 256 },
            mode: TPMU_SYM_MODE { aes: TPM2_ALG_CFB },
        }
    }
}

impl Default for TpmtSymDefBuilder {
    fn default() -> Self {
        TpmtSymDefBuilder::new()
    }
}

bitfield! {
    pub struct ObjectAttributes(TPMA_OBJECT);
    impl Debug;
    // Object attribute flags
    pub fixed_tpm, set_fixed_tpm: 1;
    pub st_clear, set_st_clear: 2;
    pub fixed_parent, set_fixed_parent: 4;
    pub sensitive_data_origin, set_sensitive_data_origin: 5;
    pub user_with_auth, set_user_with_auth: 6;
    pub admin_with_policy, set_admin_with_policy: 7;
    pub no_da, set_no_da: 10;
    pub encrypted_duplication, set_encrypted_duplication: 11;
    pub restricted, set_restricted: 16;
    pub decrypt, set_decrypt: 17;
    pub sign_encrypt, set_sign_encrypt: 18;
}

impl ObjectAttributes {
    pub fn new_fixed_parent_key() -> Self {
        let mut attrs = ObjectAttributes(0);
        attrs.set_fixed_tpm(true);
        attrs.set_fixed_parent(true);
        attrs.set_sensitive_data_origin(true);
        attrs.set_user_with_auth(true);
        attrs.set_decrypt(true);
        attrs.set_restricted(true);
        attrs
    }

    pub fn new_fixed_signing_key() -> Self {
        let mut attrs = ObjectAttributes(0);
        attrs.set_fixed_tpm(true);
        attrs.set_fixed_parent(true);
        attrs.set_sensitive_data_origin(true);
        attrs.set_user_with_auth(true);
        attrs.set_sign_encrypt(true);

        attrs
    }
}

/// Rust enum representation of `TPMU_PUBLIC_ID`.
// Most of the field types are from bindgen which does not implement Debug on them.
#[allow(missing_debug_implementations)]
pub enum PublicIdUnion {
    KeyedHash(TPM2B_DIGEST),
    Sym(TPM2B_DIGEST),
    Rsa(Box<TPM2B_PUBLIC_KEY_RSA>),
    Ecc(Box<TPMS_ECC_POINT>),
}

impl PublicIdUnion {
    /// Extract a `PublicIdUnion` from a `TPM2B_PUBLIC` object.
    ///
    /// # Constraints
    /// * the value of `public.publicArea.type_` *MUST* be consistent with the union field used in
    /// `public.publicArea.unique`.
    ///
    /// # Safety
    ///
    /// Check "Notes on code safety" section in the crate-level documentation.
    pub unsafe fn from_public(public: &TPM2B_PUBLIC) -> Result<Self> {
        match public.publicArea.type_ {
            TPM2_ALG_RSA => Ok(PublicIdUnion::Rsa(Box::from(public.publicArea.unique.rsa))),
            TPM2_ALG_ECC => Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
            TPM2_ALG_SYMCIPHER => Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
            TPM2_ALG_KEYEDHASH => Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
            _ => Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
        }
    }
}

/// Rust enum representation of `TPMU_PUBLIC_PARMS`.
// Most of the field types are from bindgen which does not implement Debug on them.
#[allow(missing_debug_implementations)]
#[allow(clippy::pub_enum_variant_names)]
#[derive(Copy, Clone)]
pub enum PublicParmsUnion {
    KeyedHashDetail(TPMS_KEYEDHASH_PARMS),
    SymDetail(Cipher),
    RsaDetail(TPMS_RSA_PARMS),
    EccDetail(TPMS_ECC_PARMS),
    AsymDetail(TPMS_ASYM_PARMS),
}

impl PublicParmsUnion {
    /// Get the object type corresponding to the value's variant.
    pub fn object_type(&self) -> TPMI_ALG_PUBLIC {
        match self {
            PublicParmsUnion::AsymDetail(..) => TPM2_ALG_NULL,
            PublicParmsUnion::EccDetail(..) => TPM2_ALG_ECC,
            PublicParmsUnion::RsaDetail(..) => TPM2_ALG_RSA,
            PublicParmsUnion::SymDetail(..) => TPM2_ALG_SYMCIPHER,
            PublicParmsUnion::KeyedHashDetail(..) => TPM2_ALG_KEYEDHASH,
        }
    }
}

impl From<PublicParmsUnion> for TPMU_PUBLIC_PARMS {
    fn from(parms: PublicParmsUnion) -> Self {
        match parms {
            PublicParmsUnion::AsymDetail(tss_parms) => TPMU_PUBLIC_PARMS {
                asymDetail: tss_parms,
            },
            PublicParmsUnion::EccDetail(tss_parms) => TPMU_PUBLIC_PARMS {
                eccDetail: tss_parms,
            },
            PublicParmsUnion::RsaDetail(tss_parms) => TPMU_PUBLIC_PARMS {
                rsaDetail: tss_parms,
            },
            PublicParmsUnion::SymDetail(cipher) => TPMU_PUBLIC_PARMS {
                symDetail: cipher.into(),
            },
            PublicParmsUnion::KeyedHashDetail(tss_parms) => TPMU_PUBLIC_PARMS {
                keyedHashDetail: tss_parms,
            },
        }
    }
}

/// Rust enum representation of `TPMU_ASYM_SCHEME`.
#[derive(Copy, Clone, Debug)]
pub enum AsymSchemeUnion {
    ECDH(TPMI_ALG_HASH),
    ECMQV(TPMI_ALG_HASH),
    RSASSA(TPMI_ALG_HASH),
    RSAPSS(TPMI_ALG_HASH),
    ECDSA(TPMI_ALG_HASH),
    ECDAA(TPMI_ALG_HASH, u16),
    SM2(TPMI_ALG_HASH),
    ECSchnorr(TPMI_ALG_HASH),
    RSAES,
    RSAOAEP(TPMI_ALG_HASH),
    AnySig(TPMI_ALG_HASH),
}

impl AsymSchemeUnion {
    /// Get scheme ID.
    pub fn scheme_id(self) -> TPM2_ALG_ID {
        match self {
            AsymSchemeUnion::ECDH(_) => TPM2_ALG_ECDH,
            AsymSchemeUnion::ECMQV(_) => TPM2_ALG_ECMQV,
            AsymSchemeUnion::RSASSA(_) => TPM2_ALG_RSASSA,
            AsymSchemeUnion::RSAPSS(_) => TPM2_ALG_RSAPSS,
            AsymSchemeUnion::ECDSA(_) => TPM2_ALG_ECDSA,
            AsymSchemeUnion::ECDAA(_, _) => TPM2_ALG_ECDAA,
            AsymSchemeUnion::SM2(_) => TPM2_ALG_SM2,
            AsymSchemeUnion::ECSchnorr(_) => TPM2_ALG_ECSCHNORR,
            AsymSchemeUnion::RSAES => TPM2_ALG_RSAES,
            AsymSchemeUnion::RSAOAEP(_) => TPM2_ALG_OAEP,
            AsymSchemeUnion::AnySig(_) => TPM2_ALG_NULL,
        }
    }

    /// Convert scheme object to `TPMT_RSA_SCHEME`.
    fn get_rsa_scheme(self) -> TPMT_RSA_SCHEME {
        let scheme = self.scheme_id();
        let details = match self {
            AsymSchemeUnion::ECDH(hash_alg) => TPMU_ASYM_SCHEME {
                ecdh: TPMS_SCHEME_HASH { hashAlg: hash_alg },
            },
            AsymSchemeUnion::ECMQV(hash_alg) => TPMU_ASYM_SCHEME {
                ecmqv: TPMS_SCHEME_HASH { hashAlg: hash_alg },
            },
            AsymSchemeUnion::RSASSA(hash_alg) => TPMU_ASYM_SCHEME {
                rsassa: TPMS_SCHEME_HASH { hashAlg: hash_alg },
            },
            AsymSchemeUnion::RSAPSS(hash_alg) => TPMU_ASYM_SCHEME {
                rsapss: TPMS_SCHEME_HASH { hashAlg: hash_alg },
            },
            AsymSchemeUnion::ECDSA(hash_alg) => TPMU_ASYM_SCHEME {
                ecdsa: TPMS_SCHEME_HASH { hashAlg: hash_alg },
            },
            AsymSchemeUnion::ECDAA(hash_alg, count) => TPMU_ASYM_SCHEME {
                ecdaa: TPMS_SCHEME_ECDAA {
                    hashAlg: hash_alg,
                    count,
                },
            },
            AsymSchemeUnion::SM2(hash_alg) => TPMU_ASYM_SCHEME {
                sm2: TPMS_SCHEME_HASH { hashAlg: hash_alg },
            },
            AsymSchemeUnion::ECSchnorr(hash_alg) => TPMU_ASYM_SCHEME {
                ecschnorr: TPMS_SCHEME_HASH { hashAlg: hash_alg },
            },
            AsymSchemeUnion::RSAES => TPMU_ASYM_SCHEME {
                rsaes: Default::default(),
            },
            AsymSchemeUnion::RSAOAEP(hash_alg) => TPMU_ASYM_SCHEME {
                oaep: TPMS_SCHEME_HASH { hashAlg: hash_alg },
            },
            AsymSchemeUnion::AnySig(hash_alg) => TPMU_ASYM_SCHEME {
                anySig: TPMS_SCHEME_HASH { hashAlg: hash_alg },
            },
        };

        TPMT_RSA_SCHEME { scheme, details }
    }
}

/// Rust native representation of an asymmetric signature.
///
/// The structure contains the signature as a byte vector and the scheme with which the signature
/// was created.
#[derive(Debug)]
pub struct Signature {
    pub scheme: AsymSchemeUnion,
    pub signature: Vec<u8>,
}

impl Signature {
    /// Attempt to parse a signature from a `TPMT_SIGNATURE` object.
    ///
    /// # Constraints
    /// * the value of `tss_signature.sigAlg` *MUST* be consistent with the union field used in
    /// `tss_signature.signature`
    ///
    /// # Safety
    ///
    /// Check "Notes on code safety" section in the crate-level documentation.
    pub unsafe fn try_from(tss_signature: TPMT_SIGNATURE) -> Result<Self> {
        match tss_signature.sigAlg {
            TPM2_ALG_RSASSA => {
                let hash_alg = tss_signature.signature.rsassa.hash;
                let scheme = AsymSchemeUnion::RSASSA(hash_alg);
                let signature_buf = tss_signature.signature.rsassa.sig;
                let mut signature = signature_buf.buffer.to_vec();
                let buf_size = signature_buf.size.into();
                if buf_size > signature.len() {
                    return Err(Error::local_error(WrapperErrorKind::InconsistentParams));
                }
                signature.truncate(buf_size);

                Ok(Signature { scheme, signature })
            }
            TPM2_ALG_ECDH => Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
            TPM2_ALG_ECDSA => Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
            TPM2_ALG_OAEP => Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
            TPM2_ALG_RSAPSS => Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
            TPM2_ALG_RSAES => Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
            TPM2_ALG_ECMQV => Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
            TPM2_ALG_SM2 => Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
            TPM2_ALG_ECSCHNORR => Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
            TPM2_ALG_ECDAA => Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
            _ => Err(Error::local_error(WrapperErrorKind::InconsistentParams)),
        }
    }
}

impl TryFrom<Signature> for TPMT_SIGNATURE {
    type Error = Error;
    fn try_from(sig: Signature) -> Result<Self> {
        let len = sig.signature.len();
        if len > 512 {
            return Err(Error::local_error(WrapperErrorKind::WrongParamSize));
        }

        let mut buffer = [0_u8; 512];
        buffer[..len].clone_from_slice(&sig.signature[..len]);

        match sig.scheme {
            AsymSchemeUnion::ECDH(_) => Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
            AsymSchemeUnion::ECMQV(_) => {
                Err(Error::local_error(WrapperErrorKind::UnsupportedParam))
            }
            AsymSchemeUnion::RSASSA(hash_alg) => Ok(TPMT_SIGNATURE {
                sigAlg: TPM2_ALG_RSASSA,
                signature: TPMU_SIGNATURE {
                    rsassa: TPMS_SIGNATURE_RSA {
                        hash: hash_alg,
                        sig: TPM2B_PUBLIC_KEY_RSA {
                            size: len.try_into().expect("Failed to convert length to u16"), // Should never panic as per the check above
                            buffer,
                        },
                    },
                },
            }),
            AsymSchemeUnion::RSAPSS(_) => {
                Err(Error::local_error(WrapperErrorKind::UnsupportedParam))
            }
            AsymSchemeUnion::ECDSA(_) => {
                Err(Error::local_error(WrapperErrorKind::UnsupportedParam))
            }
            AsymSchemeUnion::ECDAA(_, _) => {
                Err(Error::local_error(WrapperErrorKind::UnsupportedParam))
            }
            AsymSchemeUnion::SM2(_) => Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
            AsymSchemeUnion::ECSchnorr(_) => {
                Err(Error::local_error(WrapperErrorKind::UnsupportedParam))
            }
            AsymSchemeUnion::RSAES => Err(Error::local_error(WrapperErrorKind::UnsupportedParam)),
            AsymSchemeUnion::RSAOAEP(_) => {
                Err(Error::local_error(WrapperErrorKind::UnsupportedParam))
            }
            AsymSchemeUnion::AnySig(_) => {
                Err(Error::local_error(WrapperErrorKind::UnsupportedParam))
            }
        }
    }
}

/// Rust native wrapper for session attributes objects.
#[derive(Copy, Clone, Debug, Default)]
pub struct TpmaSession {
    flags: TPMA_SESSION,
    mask: TPMA_SESSION,
}

impl TpmaSession {
    // Clones the TpmaSession but adds a new mask.
    pub fn clone_with_new_mask(self, new_mask: TPMA_SESSION) -> TpmaSession {
        TpmaSession {
            flags: self.flags,
            mask: new_mask,
        }
    }

    /// Function to retrieve the masks
    /// that have been set.
    pub fn mask(self) -> TPMA_SESSION {
        self.mask
    }

    /// Function to retrive the flags that
    /// gave been set.
    pub fn flags(self) -> TPMA_SESSION {
        self.flags
    }
}

/// A builder for TpmaSession.
///
/// If no mask have been added then the mask
/// will be set till all flags.
#[derive(Copy, Clone, Debug, Default)]
pub struct TpmaSessionBuilder {
    flags: TPMA_SESSION,
    mask: Option<TPMA_SESSION>,
}

impl TpmaSessionBuilder {
    pub fn new() -> TpmaSessionBuilder {
        TpmaSessionBuilder {
            flags: 0,
            mask: None,
        }
    }

    /// Function used to add flags.
    pub fn with_flag(mut self, flag: TPMA_SESSION) -> Self {
        self.flags |= flag;
        self
    }

    /// Function used to add masks.
    pub fn with_mask(mut self, mask: TPMA_SESSION) -> Self {
        self.mask = Some(self.mask.unwrap_or(0) | mask);
        self
    }

    /// Function used to build a TpmaSession
    /// from the parameters that have been
    /// added.
    pub fn build(self) -> TpmaSession {
        TpmaSession {
            flags: self.flags,
            mask: self.mask.unwrap_or(self.flags),
        }
    }
}

/// Rust native wrapper for `TPMS_CONTEXT` objects.
///
/// This structure is intended to help with persisting object contexts. As the main reason for
/// saving the context of an object is to be able to re-use it later, on demand, a serializable
/// structure is most commonly needed. `TpmsContext` implements the `Serialize` and `Deserialize`
/// defined by `serde`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TpmsContext {
    sequence: u64,
    saved_handle: TPMI_DH_CONTEXT,
    hierarchy: TPMI_RH_HIERARCHY,
    context_blob: Vec<u8>,
}

// TODO: Replace with `From`
impl TryFrom<TPMS_CONTEXT> for TpmsContext {
    type Error = Error;

    fn try_from(tss2_context: TPMS_CONTEXT) -> Result<Self> {
        let mut context = TpmsContext {
            sequence: tss2_context.sequence,
            saved_handle: tss2_context.savedHandle,
            hierarchy: tss2_context.hierarchy,
            context_blob: tss2_context.contextBlob.buffer.to_vec(),
        };
        context.context_blob.truncate(
            tss2_context
                .contextBlob
                .size
                .try_into()
                .or_else(|_| Err(Error::local_error(WrapperErrorKind::WrongParamSize)))?,
        );
        Ok(context)
    }
}

#[allow(clippy::needless_update)]
impl TryFrom<TpmsContext> for TPMS_CONTEXT {
    type Error = Error;

    fn try_from(context: TpmsContext) -> Result<Self> {
        let buffer_size = context.context_blob.len();
        if buffer_size > 5188 {
            return Err(Error::local_error(WrapperErrorKind::WrongParamSize));
        }
        let mut buffer = [0_u8; 5188];
        for (i, val) in context.context_blob.into_iter().enumerate() {
            buffer[i] = val;
        }
        Ok(TPMS_CONTEXT {
            sequence: context.sequence,
            savedHandle: context.saved_handle,
            hierarchy: context.hierarchy,
            contextBlob: TPM2B_CONTEXT_DATA {
                size: buffer_size.try_into().unwrap(), // should not panic given the check above
                buffer,
            },
            ..Default::default()
        })
    }
}

/// Enum describing the object hierarchies in a TPM 2.0.
#[derive(Debug, Clone, Copy)]
pub enum Hierarchy {
    Null,
    Owner,
    Platform,
    Endorsement,
}

impl Hierarchy {
    /// Get the ESYS resource handle for the hierarchy.
    pub fn esys_rh(self) -> TPMI_RH_HIERARCHY {
        match self {
            Hierarchy::Null => ESYS_TR_RH_NULL,
            Hierarchy::Owner => ESYS_TR_RH_OWNER,
            Hierarchy::Platform => ESYS_TR_RH_PLATFORM,
            Hierarchy::Endorsement => ESYS_TR_RH_ENDORSEMENT,
        }
    }

    /// Get the TPM resource handle for the hierarchy.
    pub fn rh(self) -> TPM2_RH {
        match self {
            Hierarchy::Null => TPM2_RH_NULL,
            Hierarchy::Owner => TPM2_RH_OWNER,
            Hierarchy::Platform => TPM2_RH_PLATFORM,
            Hierarchy::Endorsement => TPM2_RH_ENDORSEMENT,
        }
    }
}

impl TryFrom<TPM2_HANDLE> for Hierarchy {
    type Error = Error;

    fn try_from(handle: TPM2_HANDLE) -> Result<Self> {
        match handle {
            TPM2_RH_NULL | ESYS_TR_RH_NULL => Ok(Hierarchy::Null),
            TPM2_RH_OWNER | ESYS_TR_RH_OWNER => Ok(Hierarchy::Owner),
            TPM2_RH_PLATFORM | ESYS_TR_RH_PLATFORM => Ok(Hierarchy::Platform),
            TPM2_RH_ENDORSEMENT | ESYS_TR_RH_ENDORSEMENT => Ok(Hierarchy::Endorsement),
            _ => Err(Error::local_error(WrapperErrorKind::InconsistentParams)),
        }
    }
}

/// Rust native wrapper for `TPMT_TK_VERIFIED` objects.
#[derive(Debug)]
pub struct TpmtTkVerified {
    hierarchy: Hierarchy,
    digest: Vec<u8>,
}

impl TpmtTkVerified {
    /// Get the hierarchy associated with the verification ticket.
    pub fn hierarchy(&self) -> Hierarchy {
        self.hierarchy
    }

    /// Get the digest associated with the verification ticket.
    pub fn digest(&self) -> &[u8] {
        &self.digest
    }
}

impl TryFrom<TPMT_TK_VERIFIED> for TpmtTkVerified {
    type Error = Error;

    fn try_from(tss_verif: TPMT_TK_VERIFIED) -> Result<Self> {
        if tss_verif.tag != TPM2_ST_VERIFIED {
            return Err(Error::local_error(WrapperErrorKind::InconsistentParams));
        }

        let len = tss_verif.digest.size.into();
        let mut digest = tss_verif.digest.buffer.to_vec();
        digest.truncate(len);

        let hierarchy = tss_verif.hierarchy.try_into()?;

        Ok(TpmtTkVerified { hierarchy, digest })
    }
}

impl TryFrom<TpmtTkVerified> for TPMT_TK_VERIFIED {
    type Error = Error;

    fn try_from(verif: TpmtTkVerified) -> Result<Self> {
        let digest = verif.digest;
        if digest.len() > 64 {
            return Err(Error::local_error(WrapperErrorKind::WrongParamSize));
        }

        let mut buffer = [0; 64];
        buffer[..digest.len()].clone_from_slice(&digest[..digest.len()]);

        Ok(TPMT_TK_VERIFIED {
            tag: TPM2_ST_VERIFIED,
            hierarchy: verif.hierarchy.rh(),
            digest: TPM2B_DIGEST {
                size: digest.len().try_into().unwrap(), // should not fail based on the checks done above
                buffer,
            },
        })
    }
}

/// Create the TPM2B_PUBLIC structure for a restricted decryption key.
///
/// * `symmetric` - Cipher to be used for decrypting children of the key
/// * `key_bits` - Size in bits of the decryption key
/// * `pub_exponent` - Public exponent of the RSA key. A value of 0 defaults to 2^16 + 1
pub fn create_restricted_decryption_rsa_public(
    symmetric: Cipher,
    key_bits: u16,
    pub_exponent: u32,
) -> Result<TPM2B_PUBLIC> {
    let rsa_parms = TpmsRsaParmsBuilder::new_restricted_decryption_key(
        symmetric.into(),
        key_bits,
        pub_exponent,
    )
    .build()?;
    let mut object_attributes = ObjectAttributes(0);
    object_attributes.set_fixed_tpm(true);
    object_attributes.set_fixed_parent(true);
    object_attributes.set_sensitive_data_origin(true);
    object_attributes.set_user_with_auth(true);
    object_attributes.set_decrypt(true);
    object_attributes.set_sign_encrypt(false);
    object_attributes.set_restricted(true);

    Tpm2BPublicBuilder::new()
        .with_type(TPM2_ALG_RSA)
        .with_name_alg(TPM2_ALG_SHA256)
        .with_object_attributes(object_attributes)
        .with_parms(PublicParmsUnion::RsaDetail(rsa_parms))
        .build()
}

/// Create the TPM2B_PUBLIC structure for an unrestricted signing key.
///
/// * `scheme` - Asymmetric key to be used for signing
/// * `key_bits` - Size in bits of the decryption key
/// * `pub_exponent` - Public exponent of the RSA key. A value of 0 defaults to 2^16 + 1
pub fn create_unrestricted_signing_rsa_public(
    scheme: AsymSchemeUnion,
    key_bits: u16,
    pub_exponent: u32,
) -> Result<TPM2B_PUBLIC> {
    let rsa_parms =
        TpmsRsaParmsBuilder::new_unrestricted_signing_key(scheme, key_bits, pub_exponent)
            .build()?; // should not fail as we control the params
    let mut object_attributes = ObjectAttributes(0);
    object_attributes.set_fixed_tpm(true);
    object_attributes.set_fixed_parent(true);
    object_attributes.set_sensitive_data_origin(true);
    object_attributes.set_user_with_auth(true);
    object_attributes.set_decrypt(false);
    object_attributes.set_sign_encrypt(true);
    object_attributes.set_restricted(false);

    Tpm2BPublicBuilder::new()
        .with_type(TPM2_ALG_RSA)
        .with_name_alg(TPM2_ALG_SHA256)
        .with_object_attributes(object_attributes)
        .with_parms(PublicParmsUnion::RsaDetail(rsa_parms))
        .build() // should not fail as we control the params
}

#[derive(Hash, Eq, PartialEq, Copy, Clone, Debug)]
pub enum PcrSlot {
    Slot0 = 0,
    Slot1,
    Slot2,
    Slot3,
    Slot4,
    Slot5,
    Slot6,
    Slot7,
    Slot8,
    Slot9,
    Slot10,
    Slot11,
    Slot12,
    Slot13,
    Slot14,
    Slot15,
    Slot16,
    Slot17,
    Slot18,
    Slot19,
    Slot20,
    Slot21,
    Slot22,
    Slot23,
}

impl TryFrom<u32> for PcrSlot {
    type Error = Error;
    fn try_from(pcr_slot_number: u32) -> Result<Self> {
        match pcr_slot_number {
            0 => Ok(PcrSlot::Slot0),
            1 => Ok(PcrSlot::Slot1),
            2 => Ok(PcrSlot::Slot2),
            3 => Ok(PcrSlot::Slot3),
            4 => Ok(PcrSlot::Slot4),
            5 => Ok(PcrSlot::Slot5),
            6 => Ok(PcrSlot::Slot6),
            7 => Ok(PcrSlot::Slot7),
            8 => Ok(PcrSlot::Slot8),
            9 => Ok(PcrSlot::Slot9),
            10 => Ok(PcrSlot::Slot10),
            11 => Ok(PcrSlot::Slot11),
            12 => Ok(PcrSlot::Slot12),
            13 => Ok(PcrSlot::Slot13),
            14 => Ok(PcrSlot::Slot14),
            15 => Ok(PcrSlot::Slot15),
            16 => Ok(PcrSlot::Slot16),
            17 => Ok(PcrSlot::Slot17),
            18 => Ok(PcrSlot::Slot18),
            19 => Ok(PcrSlot::Slot19),
            20 => Ok(PcrSlot::Slot20),
            21 => Ok(PcrSlot::Slot21),
            22 => Ok(PcrSlot::Slot22),
            23 => Ok(PcrSlot::Slot23),
            _ => Err(Error::local_error(WrapperErrorKind::InvalidParam)),
        }
    }
}

/// TPML_PCR_SELECTION is a structure that can contain up to
/// 16 TPMS_PCR_SELECTION
///
/// The TPMS_PCR_SELECTION is a variable size bitmask
/// where the position of each bit is the selected index.
///
/// The minimum number of octets in a TPMS_PCR_SELECT.sizeOfSelect
/// is not determined by the number of PCR implemented but by the
/// number of PCR required by the platform-specific
/// specification with which the TPM is compliant or by the implementer if
/// not adhering to a platform-specific specification.
///
/// So to make things simple 3 octets are
/// used which means that size of selections always will be
/// 3.
// For example, selecting PCRs 3 and 9 looks like:
// size(3)  mask     mask     mask     mask
// 00000011 00000000 00000000 00000001 00000100
#[derive(Debug, Default, Clone)]
pub struct PcrSelections {
    size_of_select: u8,
    items: HashMap<HashingAlgorithm, HashSet<PcrSlot>>,
}

impl From<PcrSelections> for TPML_PCR_SELECTION {
    fn from(pcr_selections: PcrSelections) -> TPML_PCR_SELECTION {
        let mut ret: TPML_PCR_SELECTION = Default::default();
        for (hash_algorithm, pcr_slots) in &pcr_selections.items {
            ret.pcrSelections[ret.count as usize].hash = hash_algorithm.clone().into();
            ret.pcrSelections[ret.count as usize].sizeofSelect = pcr_selections.size_of_select;
            for &pcr_slot in pcr_slots {
                let index: usize = (pcr_slot as usize) / 8;
                let value: u8 = 1 << ((pcr_slot as u8) % 8);
                ret.pcrSelections[ret.count as usize].pcrSelect[index] |= value;
            }
            ret.count += 1;
        }
        ret
    }
}

impl TryFrom<TPML_PCR_SELECTION> for PcrSelections {
    type Error = Error;
    fn try_from(tpml_pcr_selection: TPML_PCR_SELECTION) -> Result<PcrSelections> {
        let mut ret: PcrSelections = Default::default();
        let mut size_of_select: Option<u8> = None;
        // Loop over available selections
        for selection_index in 0..(tpml_pcr_selection.count as usize) {
            let selection = &tpml_pcr_selection.pcrSelections[selection_index];
            let mut pcr_slots: HashSet<PcrSlot> = HashSet::<PcrSlot>::new();
            // Check for variations in sizeofSelect.
            // Something that currently is not supported.
            if selection.sizeofSelect != size_of_select.unwrap_or(selection.sizeofSelect) {
                return Err(Error::local_error(WrapperErrorKind::InconsistentParams));
            }
            size_of_select = Some(selection.sizeofSelect);
            // Loop over pcr slots to find the selected ones.
            for slot_nr in 0..((selection.sizeofSelect * 8) - 1) {
                let index: usize = (slot_nr / 8) as usize;
                let mask: u8 = 1 << (slot_nr % 8);
                let is_set = (selection.pcrSelect[index] & mask) == mask;
                if is_set {
                    let _ = pcr_slots.insert(PcrSlot::try_from(slot_nr as u32).unwrap());
                }
            }
            let _ = ret.items.insert(
                HashingAlgorithm::try_from(selection.hash).unwrap(),
                pcr_slots,
            );
        }
        ret.size_of_select = size_of_select.unwrap();
        Ok(ret)
    }
}

#[derive(Debug, Default)]
pub struct PcrSelectionsBuilder {
    size_of_select: Option<u8>,
    items: HashMap<HashingAlgorithm, HashSet<PcrSlot>>,
}

/// A builder for the PcrSelection struct.
impl PcrSelectionsBuilder {
    pub fn new() -> Self {
        PcrSelectionsBuilder {
            size_of_select: None,
            items: Default::default(),
        }
    }

    /// Set the size of the pcr selection(sizeofSelect)
    ///
    /// # Arguments
    /// size_of_select -- The size that will be used for all selections(sizeofSelect).
    pub fn with_size_of_select(mut self, size_of_select: u8) -> Self {
        self.size_of_select = Some(size_of_select);
        self
    }

    /// Adds a selection associated with a specific HashingAlgorithm.
    ///
    /// This function will not overwrite the values already associated
    /// with a specific HashingAlgorithm only update.
    ///
    /// # Arguments
    /// hash_algorithm -- The HashingAlgorithm associated with the pcr selection
    /// pcr_slots -- The PCR slots in the selection.
    pub fn with_selection(
        mut self,
        hash_algorithm: HashingAlgorithm,
        pcr_slots: &[PcrSlot],
    ) -> Self {
        let selected_pcr_slots: HashSet<PcrSlot> = pcr_slots.iter().cloned().collect();
        match self.items.get_mut(&hash_algorithm) {
            Some(previously_selected_pcr_slots) => {
                *previously_selected_pcr_slots = previously_selected_pcr_slots
                    .union(&selected_pcr_slots)
                    .cloned()
                    .collect();
            }
            None => {
                let _ = self.items.insert(hash_algorithm, selected_pcr_slots);
            }
        }
        self
    }

    /// Builds a PcrSelections with the values that have been
    /// provided.
    ///
    /// If no size of select have been provided then it will
    /// be defaulted to 3. This may not be the correct size for
    /// the current platform. The correct values can be obtained
    /// by quering the tpm for its capabilities.
    pub fn build(self) -> PcrSelections {
        let select_size = match self.size_of_select {
            Some(value) => value,
            None => 3,
        };
        PcrSelections {
            size_of_select: select_size,
            items: self.items,
        }
    }
}
