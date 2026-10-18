//! Nostr "spool" sync transport for the Greek learning app.
//!
//! One human identity = one nostr keypair. Relays are untrusted storage:
//! every batch is NIP-44-encrypted to self and published as kind-1078
//! events (chunked at 48 KiB), plus a kind-30078 blinded checkpoint.
//! See SYNC-WIRE.md §2 for the wire format.

pub mod codec;
pub mod transport;

pub use codec::{chunk_batch, reassemble_chunks, CHUNK_SIZE};
pub use transport::NostrSpoolTransport;

// SYNC-CORE-MIRROR: canonicalized to greek_core::sync at integration
use serde::{Deserialize, Serialize};

/// A single spaced-repetition review event.
// SYNC-CORE-MIRROR: canonicalized to greek_core::sync at integration
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewEvent {
    pub uuid: String,
    pub card_id: String,
    pub timestamp: String,
    pub rating: u32,
    pub elapsed_days: f64,
    pub scheduled_days: f64,
    pub source: String,
    pub device_id: String,
    pub hlc: String,
}

/// A sync batch: v1 of the sync wire format.
// SYNC-CORE-MIRROR: canonicalized to greek_core::sync at integration
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncBatch {
    pub v: u32, // 1
    pub device_id: String,
    pub from_hlc: String,
    pub to_hlc: String,
    pub reviews: Vec<ReviewEvent>,
    /// (key, value, hlc) settings triples.
    pub settings: Vec<(String, String, String)>,
}

/// Errors surfaced by the nostr spool transport.
#[derive(Debug, thiserror::Error)]
pub enum SyncNostrError {
    #[error("relay ack/response timed out")]
    RelayTimeout,
    #[error("relay connection failed: {0}")]
    Connect(String),
    #[error("event publish failed: {0}")]
    Publish(String),
    #[error("failed to decrypt/parse {count} event(s)")]
    DecryptFailed { count: usize },
    #[error("incomplete batch: {missing} chunk(s) missing")]
    PartialBatch { missing: usize },
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("nostr error: {0}")]
    Nostr(String),
}

pub type Result<T> = std::result::Result<T, SyncNostrError>;
