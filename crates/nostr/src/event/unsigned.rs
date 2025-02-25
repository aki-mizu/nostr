// Copyright (c) 2022-2023 Yuki Kishimoto
// Copyright (c) 2023-2024 Rust Nostr Developers
// Distributed under the MIT software license

//! Unsigned Event

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

#[cfg(feature = "std")]
use bitcoin::secp256k1::rand;
use bitcoin::secp256k1::rand::{CryptoRng, Rng};
use bitcoin::secp256k1::schnorr::Signature;
use bitcoin::secp256k1::{self, Message, Secp256k1, Signing, Verification};

#[cfg(feature = "std")]
use crate::SECP256K1;
use crate::{Event, EventId, JsonUtil, Keys, Kind, PublicKey, Tag, Timestamp};

/// [`UnsignedEvent`] error
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// Key error
    Key(crate::key::Error),
    /// Error serializing or deserializing JSON data
    Json(String),
    /// Secp256k1 error
    Secp256k1(secp256k1::Error),
    /// Event error
    Event(super::Error),
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(e) => write!(f, "Key: {e}"),
            Self::Json(e) => write!(f, "Json: {e}"),
            Self::Secp256k1(e) => write!(f, "Secp256k1: {e}"),
            Self::Event(e) => write!(f, "Event: {e}"),
        }
    }
}

impl From<crate::key::Error> for Error {
    fn from(e: crate::key::Error) -> Self {
        Self::Key(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e.to_string())
    }
}

impl From<secp256k1::Error> for Error {
    fn from(e: secp256k1::Error) -> Self {
        Self::Secp256k1(e)
    }
}

impl From<super::Error> for Error {
    fn from(e: super::Error) -> Self {
        Self::Event(e)
    }
}

/// [`UnsignedEvent`] struct
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct UnsignedEvent {
    /// Id
    pub id: EventId,
    /// Author
    pub pubkey: PublicKey,
    /// Timestamp (seconds)
    pub created_at: Timestamp,
    /// Kind
    pub kind: Kind,
    /// Vector of [`Tag`]
    pub tags: Vec<Tag>,
    /// Content
    pub content: String,
}

impl UnsignedEvent {
    /// Verify if the [`EventId`] it's composed correctly
    pub fn verify_id(&self) -> Result<(), Error> {
        let id: EventId = EventId::new(
            &self.pubkey,
            self.created_at,
            &self.kind,
            &self.tags,
            &self.content,
        );
        if id == self.id {
            Ok(())
        } else {
            Err(Error::Event(super::Error::InvalidId))
        }
    }

    /// Sign an [`UnsignedEvent`]
    #[cfg(feature = "std")]
    pub fn sign(self, keys: &Keys) -> Result<Event, Error> {
        self.sign_with_ctx(&SECP256K1, &mut rand::thread_rng(), keys)
    }

    /// Sign an [`UnsignedEvent`]
    pub fn sign_with_ctx<C, R>(
        self,
        secp: &Secp256k1<C>,
        rng: &mut R,
        keys: &Keys,
    ) -> Result<Event, Error>
    where
        C: Signing,
        R: Rng + CryptoRng,
    {
        let message = Message::from_slice(self.id.as_bytes())?;
        Ok(Event::new(
            self.id,
            self.pubkey,
            self.created_at,
            self.kind,
            self.tags,
            self.content,
            keys.sign_schnorr_with_ctx(secp, &message, rng)?,
        ))
    }

    /// Add signature to [`UnsignedEvent`]
    #[cfg(feature = "std")]
    pub fn add_signature(self, sig: Signature) -> Result<Event, Error> {
        self.add_signature_with_ctx(&SECP256K1, sig)
    }

    /// Add signature to [`UnsignedEvent`]
    pub fn add_signature_with_ctx<C>(
        self,
        secp: &Secp256k1<C>,
        sig: Signature,
    ) -> Result<Event, Error>
    where
        C: Verification,
    {
        let event = Event::new(
            self.id,
            self.pubkey,
            self.created_at,
            self.kind,
            self.tags,
            self.content,
            sig,
        );
        event.verify_with_ctx(secp)?;
        Ok(event)
    }
}

impl JsonUtil for UnsignedEvent {
    type Err = Error;
}

impl From<Event> for UnsignedEvent {
    fn from(event: Event) -> Self {
        Self {
            id: event.id,
            pubkey: event.pubkey,
            created_at: event.created_at,
            kind: event.kind,
            tags: event.tags.clone(),
            content: event.content.clone(),
        }
    }
}
