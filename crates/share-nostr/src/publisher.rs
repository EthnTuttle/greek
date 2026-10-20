//! Publishing: an OWN [`nostr_sdk::Client`] pool (Appendix R network-identity
//! separation — this client never shares connections with the sync client),
//! per-relay publish attempts, and NIP-20 `OK`-message prefix
//! classification.
//!
//! `nostr_sdk`'s `send_event` lumps a relay's `OK false` into that relay's
//! error string, so a duplicate resend arrives as an *error* containing
//! `duplicate: …`. [`classify_ok_message`] is the single place that decides
//! what each prefix means:
//!
//! | prefix            | class        | effect                              |
//! |-------------------|--------------|-------------------------------------|
//! | `duplicate:`      | `Duplicate`  | treated as delivered                |
//! | `rate-limited:`   | `Retry`      | row stays queued; flush backs off   |
//! | `blocked:`/`invalid:`/`pow:`/`restricted:` | `Permanent` | relay stops retrying this event |
//! | `auth-required:`  | `UnsupportedAuth` | NIP-42 unsupported in v1; permanent |
//! | anything else     | `Retry`      | conservative: transient             |

use std::time::Duration;

use nostr_sdk::prelude::*;

use crate::errors::{Result, ShareError};
use crate::identity::ShareKeys;

/// Timeout for a single publish fan-out (per-relay OK wait lives inside
/// the pool; this caps the whole call).
pub const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for the pre-publish read-back REQ.
pub const READ_BACK_TIMEOUT: Duration = Duration::from_secs(8);
/// Connect deadline: at least one relay must report connected.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Maximum accepted relay URLs for the sharing set (Appendix R).
pub const MAX_SHARE_RELAYS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OkClass {
    /// OK true.
    Accepted,
    /// OK false but `duplicate:` — the relay ALREADY HAS the event, which
    /// for delivery purposes is success (Appendix R).
    Duplicate,
    /// Transient: try again later (`rate-limited:`, unknown prefixes).
    Retry,
    /// Hard stop for that relay (`blocked:`/`invalid:`/`pow:`/`restricted:`,
    /// size violations).
    Permanent,
    /// `auth-required:` — NIP-42 auth is out of scope for v1; permanent.
    UnsupportedAuth,
}

impl OkClass {
    /// Delivered (one way or the other) by this relay.
    pub fn delivered(&self) -> bool {
        matches!(self, Self::Accepted | Self::Duplicate)
    }

    /// Worth retrying on a later flush.
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Retry)
    }
}

/// Machine-readable classification of a relay's OK-false message.
pub fn classify_ok_message(message: &str) -> OkClass {
    let msg = message.trim_start().to_lowercase();
    // Strip a repeated "relay message: " wrapper some pool versions add.
    let msg = msg.strip_prefix("relay message: ").unwrap_or(&msg);
    if msg.starts_with("duplicate:") {
        OkClass::Duplicate
    } else if msg.starts_with("rate-limited:") {
        OkClass::Retry
    } else if msg.starts_with("auth-required:") {
        OkClass::UnsupportedAuth
    } else if msg.starts_with("blocked:")
        || msg.starts_with("invalid:")
        || msg.starts_with("pow:")
        || msg.starts_with("restricted:")
        || msg.contains("message too large")
        || msg.contains("too large")
    {
        OkClass::Permanent
    } else {
        // Unknown prefixes (including bare "error: …") are transient.
        OkClass::Retry
    }
}

/// Per-relay result of one publish attempt.
#[derive(Debug, Clone)]
pub struct RelayOutcome {
    pub relay: String,
    pub class: OkClass,
    /// The relay's raw OK message ("" for plain acceptance).
    pub message: String,
}

impl RelayOutcome {
    /// The serialized `per_relay` map value: "accepted" or "failed:<msg>".
    pub fn per_relay_value(&self) -> String {
        if self.class.delivered() {
            "accepted".to_string()
        } else {
            format!("failed:{}", self.message)
        }
    }
}

/// Aggregate result of one publish attempt across the relay set.
#[derive(Debug, Clone)]
pub struct PublishResult {
    pub event_id: String,
    pub outcomes: Vec<RelayOutcome>,
}

impl PublishResult {
    pub fn delivered_relays(&self) -> Vec<String> {
        self.outcomes
            .iter()
            .filter(|o| o.class.delivered())
            .map(|o| o.relay.clone())
            .collect()
    }

    pub fn any_delivered(&self) -> bool {
        self.outcomes.iter().any(|o| o.class.delivered())
    }

    /// Every relay has reached a terminal state for this event — posture:
    /// no further retry will change anything (all permanent/unsupported, or
    /// all delivered, or a mix of the two).
    pub fn converged(&self) -> bool {
        !self.outcomes.is_empty() && self.outcomes.iter().all(|o| !o.class.retryable())
    }

    /// Relays that outright failed this attempt, with their reason strings
    /// — the `failed` entries of [`crate::ops::FlushReport`].
    pub fn failures(&self) -> Vec<(String, String)> {
        self.outcomes
            .iter()
            .filter(|o| !o.class.delivered())
            .map(|o| (o.relay.clone(), o.message.clone()))
            .collect()
    }
}

/// Validate a user-supplied sharing relay list: `wss://` only, at most
/// [`MAX_SHARE_RELAYS`], deduplicated preserving order. The empty list is
/// valid (Appendix R: the enable flow's relay picker starts EMPTY).
pub fn validate_relay_list(urls: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::with_capacity(urls.len());
    for raw in urls {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !trimmed.starts_with("wss://") {
            return Err(ShareError::RelayUrl(format!(
                "only wss:// relay URLs are allowed, got {trimmed:?}"
            )));
        }
        RelayUrl::parse(trimmed)
            .map_err(|e| ShareError::RelayUrl(format!("bad relay URL {trimmed:?}: {e}")))?;
        let normalized = trimmed.to_string();
        if !out.contains(&normalized) {
            out.push(normalized);
        }
        if out.len() > MAX_SHARE_RELAYS {
            return Err(ShareError::RelayUrl(format!(
                "at most {MAX_SHARE_RELAYS} sharing relays"
            )));
        }
    }
    Ok(out)
}

/// The sharing publisher. Owns its own `Client`; create one per flush batch
/// (or one per app lifetime) — never share it with the sync transport.
pub struct Publisher {
    client: Client,
    pubkey: PublicKey,
}

impl Publisher {
    /// Connect to `relay_urls`. Callers that enforce the user-facing relay
    /// policy pass URLs already vetted by [`validate_relay_list`]; the test
    /// harness may pass local `ws://` addresses directly, which the client
    /// accepts.
    pub async fn connect(keys: &ShareKeys, relay_urls: &[String]) -> Result<Self> {
        // NIP-42 relay auth is out of scope for v1: an `auth-required:`
        // OK-false is a PERMANENT per-relay stop (class UnsupportedAuth),
        // so the client must not hang waiting for AUTH challenges.
        let options = ClientOptions::new().automatic_authentication(false);
        let client = Client::builder().signer(keys.keys().clone()).opts(options).build();
        for url in relay_urls {
            client
                .add_relay(url.as_str())
                .await
                .map_err(|e| ShareError::Nostr(format!("{url}: {e}")))?;
        }
        client.connect().await;
        let deadline = std::time::Instant::now() + CONNECT_TIMEOUT;
        loop {
            let relays = client.relays().await;
            if relays.values().any(|r| r.is_connected()) {
                break;
            }
            if std::time::Instant::now() >= deadline {
                client.disconnect().await;
                return Err(ShareError::Nostr(
                    "no sharing relay connected within 10s".to_string(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok(Self {
            client,
            pubkey: keys.public_key(),
        })
    }

    pub fn public_key(&self) -> PublicKey {
        self.pubkey
    }

    /// Disconnect all relays of this publisher's pool.
    pub async fn disconnect(&self) {
        self.client.disconnect().await;
    }

    /// Publish `event` to every connected write relay in parallel and
    /// classify each relay's answer. The event bytes are whatever the outbox
    /// persisted — this function never mutates or re-signs.
    pub async fn publish(&self, event: &Event) -> Result<PublishResult> {
        if event.kind != Kind::TextNote {
            return Err(ShareError::Nostr(
                "share-nostr publishes kind 1 only".to_string(),
            ));
        }
        let out = tokio::time::timeout(SEND_TIMEOUT, self.client.send_event(event))
            .await
            .map_err(|_| ShareError::Nostr("publish timed out".to_string()))?
            .map_err(|e| ShareError::Nostr(e.to_string()))?;

        let mut outcomes: Vec<RelayOutcome> = Vec::new();
        for url in &out.success {
            outcomes.push(RelayOutcome {
                relay: url.to_string(),
                class: OkClass::Accepted,
                message: String::new(),
            });
        }
        for (url, msg) in &out.failed {
            outcomes.push(RelayOutcome {
                relay: url.to_string(),
                class: classify_ok_message(msg),
                message: msg.clone(),
            });
        }
        outcomes.sort_by(|a, b| a.relay.cmp(&b.relay));
        Ok(PublishResult {
            event_id: event.id.to_hex(),
            outcomes,
        })
    }

    /// Pre-publish read-back (Appendix R self-healing for the multi-device /
    /// crash race): REQ `{authors:[share_pk], kinds:[1], limit 50}` and
    /// content-match after NFC sanitization on both sides. `true` ⇒ at least
    /// one connected relay already serves this exact content ⇒ treat the
    /// milestone as delivered WITHOUT re-sending.
    pub async fn content_already_posted(&self, content: &str) -> Result<bool> {
        let filter = Filter::new()
            .author(self.pubkey)
            .kinds([Kind::TextNote])
            .limit(50);
        let events = tokio::time::timeout(
            READ_BACK_TIMEOUT,
            self.client.fetch_events(filter, READ_BACK_TIMEOUT),
        )
        .await
        .map_err(|_| ShareError::Nostr("read-back fetch timed out".to_string()))?
        .map_err(|e| ShareError::Nostr(format!("read-back fetch failed: {e}")))?;
        let target = crate::templates::sanitize(content);
        let found = events
            .iter()
            .any(|ev| crate::templates::sanitize(&ev.content) == target);
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_ok_prefixes() {
        assert_eq!(classify_ok_message("duplicate: already have this event"), OkClass::Duplicate);
        assert_eq!(classify_ok_message("Duplicate: seen"), OkClass::Duplicate);
        assert_eq!(
            classify_ok_message("relay message: duplicate: foo"),
            OkClass::Duplicate
        );
        assert_eq!(classify_ok_message("rate-limited: slow down"), OkClass::Retry);
        assert_eq!(classify_ok_message("auth-required: we only serve authed users"), OkClass::UnsupportedAuth);
        assert_eq!(classify_ok_message("blocked: nice try"), OkClass::Permanent);
        assert_eq!(classify_ok_message("invalid: message too large"), OkClass::Permanent);
        assert_eq!(classify_ok_message("blocked: message too large"), OkClass::Permanent);
        assert_eq!(classify_ok_message("pow: need difficulty 24"), OkClass::Permanent);
        assert_eq!(classify_ok_message("restricted: not in the allowlist"), OkClass::Permanent);
        assert_eq!(classify_ok_message("error: unspecified relay hiccup"), OkClass::Retry);
        assert_eq!(classify_ok_message("not connected"), OkClass::Retry);
    }

    #[test]
    fn validate_relay_list_rules() {
        assert!(validate_relay_list(&["wss://relay.damus.io".into()]).is_ok());
        assert!(validate_relay_list(&[]).is_ok(), "empty list is valid");
        // Dedupes.
        assert_eq!(
            validate_relay_list(&["wss://a.example".into(), "wss://a.example".into()]).unwrap(),
            vec!["wss://a.example".to_string()]
        );
        // ws:// rejected for user-supplied config (tests bypass via connect).
        assert!(matches!(
            validate_relay_list(&["ws://127.0.0.1:7777".into()]),
            Err(ShareError::RelayUrl(_))
        ));
        assert!(validate_relay_list(&["http://x".into()]).is_err());
        // Cap at 8.
        let many: Vec<String> = (0..9).map(|i| format!("wss://r{i}.example")).collect();
        assert!(validate_relay_list(&many).is_err());
        let eight: Vec<String> = (0..8).map(|i| format!("wss://r{i}.example")).collect();
        assert_eq!(validate_relay_list(&eight).unwrap().len(), 8);
    }
}
