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

// Canonical wire types live in greek-core (SYNC-WIRE.md §2.1); re-exported
// here so existing sync-nostr call sites keep working unchanged.
pub use greek_core::sync::{ReviewEvent, SyncBatch};

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
