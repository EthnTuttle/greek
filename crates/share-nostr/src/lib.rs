//! Progress sharing over nostr (DESIGN-progress-sharing.md, Appendix R).
//!
//! Coarse *measured-attainment* milestones ("7-day streak", "alphabet check
//! passed") are posted as NIP-01 kind-1 notes from a keypair HKDF-derived
//! from the sync secret — never the sync identity itself — under hard rate
//! caps, with an outbox that persists fully-signed events verbatim so retries
//! re-send identical bytes.
//!
//! ## Module map
//!
//! - [`identity`]: `ShareKeys` — HKDF-SHA256(sync_sk, "greek/share-nostr/v1").
//! - [`templates`]: shipped EN/EL presets + content validation (NFC, byte and
//!   grapheme caps, control-char stripping, URL/mention pattern rejection).
//! - [`outbox`]: `share_outbox` sqlite table; rows own the signed event json.
//! - [`caps`]: daily post budget (2/day solo, 1/day/device when multi-device)
//!   and ≥6h spacing, derived from `share.posted.*` markers + the outbox.
//! - [`publisher`]: own `nostr_sdk::Client` pool; per-relay OK-prefix
//!   classification; pre-publish content read-back.
//! - [`detect`]: pure detectors — gate latch (lane-aware), chapter mastered,
//!   due-cleared streak (FSRS replay cut at end-of-day, cached).
//! - [`ops`]: `enable()` (zero-retroactivity seeding) and `flush()`
//!   (`FlushReport`).

pub mod caps;
pub mod detect;
pub mod identity;
pub mod ops;
pub mod outbox;
pub mod publisher;
pub mod templates;

/// Flat FFI-facing surface (REQ-B contract with crates/uniffi-bindings;
/// re-exported at the crate root so binding call sites read
/// `share_nostr::is_enabled(...)` etc.).
pub mod facade;
pub use facade::*;

pub use crate::errors::{Result, ShareError};

mod errors {
    /// Error type shared by every share-nostr module.
    #[derive(Debug, thiserror::Error)]
    pub enum ShareError {
        #[error("database error: {0}")]
        Db(#[from] rusqlite::Error),
        #[error("core error: {0}")]
        Core(#[from] anyhow::Error),
        #[error("nostr error: {0}")]
        Nostr(String),
        #[error("template error: {0}")]
        Template(#[from] crate::templates::TemplateError),
        #[error("identity error: {0}")]
        Identity(String),
        #[error("relay URL rejected: {0}")]
        RelayUrl(String),
        #[error("serialization error: {0}")]
        Json(#[from] serde_json::Error),
    }

    pub type Result<T> = std::result::Result<T, ShareError>;
}
