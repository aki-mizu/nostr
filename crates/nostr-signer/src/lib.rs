// Copyright (c) 2022-2023 Yuki Kishimoto
// Copyright (c) 2023-2024 Rust Nostr Developers
// Distributed under the MIT software license

//! Nostr Signer

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::bare_urls)]

use std::fmt;

use nostr::key;
use nostr::prelude::*;
use thiserror::Error;

#[cfg(feature = "nip46")]
pub mod nip46;
pub mod prelude;

#[cfg(feature = "nip46")]
pub use self::nip46::Nip46Signer;

/// Nostr Signer error
#[derive(Debug, Error)]
pub enum Error {
    /// Keys error
    #[error(transparent)]
    Keys(#[from] key::Error),
    /// Unsigned event error
    #[error(transparent)]
    Unsigned(#[from] unsigned::Error),
    /// NIP04 error
    #[cfg(feature = "nip04")]
    #[error(transparent)]
    NIP04(#[from] nip04::Error),
    /// NIP07 error
    #[cfg(all(feature = "nip07", target_arch = "wasm32"))]
    #[error(transparent)]
    NIP07(#[from] nip07::Error),
    /// NIP44 error
    #[cfg(feature = "nip44")]
    #[error(transparent)]
    NIP44(#[from] nip44::Error),
    /// NIP46 error
    #[cfg(feature = "nip46")]
    #[error(transparent)]
    NIP46(#[from] nip46::Error),
    /// Response not match to the request
    #[cfg(feature = "nip46")]
    #[error("response not match to the request")]
    ResponseNotMatchRequest,
    /// Not supported yet
    #[error("{0}")]
    Unsupported(String),
}

#[cfg(feature = "nip44")]
impl Error {
    fn unsupported<S>(message: S) -> Self
    where
        S: Into<String>,
    {
        Self::Unsupported(message.into())
    }
}

/// Nostr Signer Type
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NostrSignerType {
    /// Keys
    Keys,
    /// NIP07
    #[cfg(all(feature = "nip07", target_arch = "wasm32"))]
    NIP07,
    /// NIP46
    #[cfg(feature = "nip46")]
    NIP46,
}

// TODO: better display
impl fmt::Display for NostrSignerType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Keys => write!(f, "Keys"),
            #[cfg(all(feature = "nip07", target_arch = "wasm32"))]
            Self::NIP07 => write!(f, "Nostr Browser Extension"),
            #[cfg(feature = "nip46")]
            Self::NIP46 => write!(f, "Nostr Connect"),
        }
    }
}

/// Nostr signer
#[derive(Debug, Clone)]
pub enum NostrSigner {
    /// Private Keys
    Keys(Keys),
    /// NIP07 signer
    #[cfg(all(feature = "nip07", target_arch = "wasm32"))]
    NIP07(Nip07Signer),
    /// NIP46 signer
    #[cfg(feature = "nip46")]
    NIP46(Box<Nip46Signer>),
}

impl NostrSigner {
    /// Create a new [NIP07] instance and compose [NostrSigner]
    #[cfg(all(feature = "nip07", target_arch = "wasm32"))]
    pub fn nip07() -> Result<Self, Error> {
        let instance = Nip07Signer::new()?;
        Ok(Self::NIP07(instance))
    }

    /// Compose [NostrSigner] with [Nip46Signer]
    #[cfg(feature = "nip46")]
    pub fn nip46(signer: Nip46Signer) -> Self {
        Self::NIP46(Box::new(signer))
    }

    /// Get Nostr Signer Type
    pub fn r#type(&self) -> NostrSignerType {
        match self {
            Self::Keys(..) => NostrSignerType::Keys,
            #[cfg(all(feature = "nip07", target_arch = "wasm32"))]
            Self::NIP07(..) => NostrSignerType::NIP07,
            #[cfg(feature = "nip46")]
            Self::NIP46(..) => NostrSignerType::NIP46,
        }
    }

    /// Get signer public key
    pub async fn public_key(&self) -> Result<PublicKey, Error> {
        match self {
            Self::Keys(keys) => Ok(keys.public_key()),
            #[cfg(all(feature = "nip07", target_arch = "wasm32"))]
            Self::NIP07(s) => Ok(s.get_public_key().await?),
            #[cfg(feature = "nip46")]
            Self::NIP46(s) => Ok(s.signer_public_key().await?),
        }
    }

    /// Sign an [EventBuilder]
    pub async fn sign_event_builder(&self, builder: EventBuilder) -> Result<Event, Error> {
        let public_key: PublicKey = self.public_key().await?;
        let unsigned: UnsignedEvent = builder.to_unsigned_event(public_key);
        self.sign_event(unsigned).await
    }

    /// Sign an [UnsignedEvent]
    pub async fn sign_event(&self, unsigned: UnsignedEvent) -> Result<Event, Error> {
        match self {
            Self::Keys(keys) => Ok(unsigned.sign(keys)?),
            #[cfg(all(feature = "nip07", target_arch = "wasm32"))]
            Self::NIP07(nip07) => Ok(nip07.sign_event(unsigned).await?),
            #[cfg(feature = "nip46")]
            Self::NIP46(nip46) => {
                let res = nip46
                    .send_req_to_signer(nostr::nips::nip46::Request::SignEvent(unsigned), None)
                    .await?;
                if let nostr::nips::nip46::Response::SignEvent(event) = res {
                    Ok(event)
                } else {
                    Err(Error::ResponseNotMatchRequest)
                }
            }
        }
    }

    /// NIP04 encrypt
    #[cfg(feature = "nip04")]
    pub async fn nip04_encrypt<T>(&self, public_key: PublicKey, content: T) -> Result<String, Error>
    where
        T: AsRef<[u8]>,
    {
        let content: &[u8] = content.as_ref();
        match self {
            Self::Keys(keys) => Ok(nip04::encrypt(keys.secret_key()?, &public_key, content)?),
            #[cfg(all(feature = "nip07", target_arch = "wasm32"))]
            Self::NIP07(signer) => Ok(signer.nip04_encrypt(public_key, content).await?),
            #[cfg(feature = "nip46")]
            Self::NIP46(signer) => {
                let req = nostr::nips::nip46::Request::Nip04Encrypt {
                    public_key,
                    text: String::from_utf8_lossy(content).to_string(),
                };
                let res: nostr::nips::nip46::Response =
                    signer.send_req_to_signer(req, None).await?;
                if let nostr::nips::nip46::Response::Nip04Encrypt(ciphertext) = res {
                    Ok(ciphertext)
                } else {
                    Err(Error::ResponseNotMatchRequest)
                }
            }
        }
    }

    /// NIP04 decrypt
    #[cfg(feature = "nip04")]
    pub async fn nip04_decrypt<T>(
        &self,
        public_key: PublicKey,
        encrypted_content: T,
    ) -> Result<String, Error>
    where
        T: AsRef<str>,
    {
        let encrypted_content: &str = encrypted_content.as_ref();
        match self {
            Self::Keys(keys) => Ok(nip04::decrypt(
                keys.secret_key()?,
                &public_key,
                encrypted_content,
            )?),
            #[cfg(all(feature = "nip07", target_arch = "wasm32"))]
            Self::NIP07(signer) => Ok(signer.nip04_decrypt(public_key, encrypted_content).await?),
            #[cfg(feature = "nip46")]
            Self::NIP46(signer) => {
                let req = nostr::nips::nip46::Request::Nip04Decrypt {
                    public_key,
                    text: encrypted_content.to_string(),
                };
                let res: nostr::nips::nip46::Response =
                    signer.send_req_to_signer(req, None).await?;
                if let nostr::nips::nip46::Response::Nip04Decrypt(content) = res {
                    Ok(content)
                } else {
                    Err(Error::ResponseNotMatchRequest)
                }
            }
        }
    }

    /// NIP44 encryption with [NostrSigner]
    #[cfg(feature = "nip44")]
    pub async fn nip44_encrypt<T>(&self, public_key: PublicKey, content: T) -> Result<String, Error>
    where
        T: AsRef<[u8]>,
    {
        let content: &[u8] = content.as_ref();
        match self {
            Self::Keys(keys) => Ok(nip44::encrypt(
                keys.secret_key()?,
                &public_key,
                content,
                nip44::Version::default(),
            )?),
            #[cfg(all(feature = "nip07", target_arch = "wasm32"))]
            Self::NIP07(signer) => Ok(signer.nip44_encrypt(public_key, content).await?),
            #[cfg(feature = "nip46")]
            Self::NIP46(..) => Err(Error::unsupported(
                "NIP44 encryption not supported with NIP46 signer yet!",
            )),
        }
    }

    /// NIP44 decryption with [NostrSigner]
    #[cfg(feature = "nip44")]
    pub async fn nip44_decrypt<T>(&self, public_key: PublicKey, payload: T) -> Result<String, Error>
    where
        T: AsRef<[u8]>,
    {
        let payload: &[u8] = payload.as_ref();
        match self {
            Self::Keys(keys) => Ok(nip44::decrypt(keys.secret_key()?, &public_key, payload)?),
            #[cfg(all(feature = "nip07", target_arch = "wasm32"))]
            Self::NIP07(signer) => Ok(signer.nip44_decrypt(public_key, payload).await?),
            #[cfg(feature = "nip46")]
            Self::NIP46(..) => Err(Error::unsupported(
                "NIP44 decryption not supported with NIP46 signer yet!",
            )),
        }
    }
}

impl From<Keys> for NostrSigner {
    fn from(keys: Keys) -> Self {
        Self::Keys(keys)
    }
}

impl From<&Keys> for NostrSigner {
    fn from(keys: &Keys) -> Self {
        Self::Keys(keys.clone())
    }
}

#[cfg(all(feature = "nip07", target_arch = "wasm32"))]
impl From<Nip07Signer> for NostrSigner {
    fn from(nip07: Nip07Signer) -> Self {
        Self::NIP07(nip07)
    }
}

#[cfg(feature = "nip46")]
impl From<Nip46Signer> for NostrSigner {
    fn from(nip46: Nip46Signer) -> Self {
        Self::nip46(nip46)
    }
}
