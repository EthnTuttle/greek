//! Sharing identity: HKDF-SHA256 derivation from the sync secret, so every
//! paired device derives the *same* sharing keypair with no new secret at
//! rest. Tradeoff (documented in Appendix R): sync-secret compromise
//! deanonymizes past shares.

use hkdf::Hkdf;
use nostr_sdk::prelude::{Event, EventBuilder, Keys, PublicKey, SecretKey};
use sha2::Sha256;

use crate::errors::{Result, ShareError};

/// HKDF `info` string for the base derivation. A retry counter is appended
/// (`"greek/share-nostr/v1#<n>"`) in the astronomically unlikely case the
/// expanded 32 bytes are not a valid secp256k1 scalar.
pub const HKDF_INFO: &str = "greek/share-nostr/v1";

/// Hard stop for the negligible-scalar retry loop. An invalid expansion has
/// probability ~2^-128 per try; reaching this bound is unreachable in
/// practice.
const MAX_DERIVE_RETRIES: u32 = 1000;

/// HKDF-SHA256(salt=None, ikm=`sync_sk_bytes`, info="greek/share-nostr/v1"),
/// retrying with `"…v1#<counter>"` until the output is a valid (non-zero)
/// secp256k1 secret scalar. Pure and deterministic.
pub fn derive_share_secret(sync_sk_bytes: &[u8; 32]) -> [u8; 32] {
    let hk: Hkdf<Sha256> = Hkdf::new(None, sync_sk_bytes);
    for counter in 0..=MAX_DERIVE_RETRIES {
        let info: String = match counter {
            0 => HKDF_INFO.to_string(),
            n => format!("{HKDF_INFO}#{n}"),
        };
        let mut okm = [0u8; 32];
        // 32 bytes is a valid HKDF-SHA256 output length; expand cannot fail.
        hk.expand(info.as_bytes(), &mut okm)
            .expect("HKDF-SHA256 32-byte expand is infallible");
        if SecretKey::from_slice(&okm).is_ok() {
            return okm;
        }
    }
    // Practically unreachable (see MAX_DERIVE_RETRIES) but never panic-able:
    // the loop above returns on the first valid scalar.
    unreachable!("no valid secp256k1 scalar found within {MAX_DERIVE_RETRIES} HKDF retries");
}

/// The nostr keypair used for sharing notes. Wraps [`nostr_sdk::Keys`]; the
/// secret is never exposed through [`std::fmt::Debug`] (redacted in both
/// debug and release builds).
pub struct ShareKeys(Keys);

impl ShareKeys {
    /// Wrap an existing keypair. Prefer [`ShareKeys::derive_from_sync`] or
    /// [`ShareKeys::from_db`] — this constructor exists mainly for tests.
    pub fn from_keys(keys: Keys) -> Self {
        Self(keys)
    }

    /// Derive the sharing keypair from the raw sync secret key bytes.
    pub fn derive_from_sync(sync_sk_bytes: &[u8; 32]) -> Result<Self> {
        let secret = derive_share_secret(sync_sk_bytes);
        let sk = SecretKey::from_slice(&secret)
            .map_err(|e| ShareError::Identity(format!("derived share secret invalid: {e}")))?;
        Ok(Self(Keys::new(sk)))
    }

    /// Load the sync secret (`nostr_nsec`, hex) from the database, derive the
    /// sharing keypair from it, and assert `share_pk != sync_pk` (Appendix R
    /// separation invariant — an equality here would identify a derivation
    /// bug, not something to paper over).
    pub fn from_db(db: &greek_core::Database) -> Result<Self> {
        let hex = db
            .conn()
            .query_row(
                "SELECT value FROM user_settings WHERE key = 'nostr_nsec'",
                [],
                |row| row.get::<_, String>(0),
            )
            .map_err(|_| {
                ShareError::Identity(
                    "nostr sync identity not configured; sharing identity derives \
                     from it — call sync_generate_nostr_identity first"
                        .to_string(),
                )
            })?;
        let sync_keys = Keys::parse(hex.trim()).map_err(|e| {
            ShareError::Identity(format!("stored nostr key is invalid: {e}"))
        })?;
        let mut sk_raw = [0u8; 32];
        let sk_hex = sync_keys.secret_key().to_secret_hex();
        // Keys::parse guarantees a valid 64-hex-char secret.
        for i in 0..32 {
            sk_raw[i] = u8::from_str_radix(&sk_hex[i * 2..i * 2 + 2], 16)
                .map_err(|e| ShareError::Identity(format!("secret hex decode: {e}")))?;
        }
        let share = Self::derive_from_sync(&sk_raw)?;
        if share.public_key() == sync_keys.public_key() {
            return Err(ShareError::Identity(
                "derived sharing pubkey equals the sync pubkey; refusing to post \
                 under the sync identity"
                    .to_string(),
            ));
        }
        Ok(share)
    }

    /// The sharing public key (what the world sees as the milestone poster).
    pub fn public_key(&self) -> PublicKey {
        self.0.public_key()
    }

    /// The sharing public key in lowercase hex, for settings/JSON output.
    pub fn public_key_hex(&self) -> String {
        self.0.public_key().to_hex()
    }

    /// Borrow the underlying [`Keys`] — e.g. for `Client::new`. Never produce
    /// a bech32/hex dump from this; the secret must not leave the process.
    pub fn keys(&self) -> &Keys {
        &self.0
    }

    /// Sign a kind-1 text note. Appendix R: this identity ONLY ever signs
    /// kind 1 — the assertion is enforced here (the single signing entry
    /// point).
    pub fn sign_note(&self, content: &str) -> Result<Event> {
        let event = EventBuilder::text_note(content)
            .sign_with_keys(&self.0)
            .map_err(|e| ShareError::Nostr(format!("event signing failed: {e}")))?;
        debug_assert_eq!(event.kind, nostr_sdk::prelude::Kind::TextNote);
        if event.kind != nostr_sdk::prelude::Kind::TextNote {
            return Err(ShareError::Identity(
                "share-nostr signs kind 1 only".to_string(),
            ));
        }
        Ok(event)
    }
}

impl Clone for ShareKeys {
    fn clone(&self) -> Self {
        // Keys derives Clone via its keypair; copying is in-process only.
        Self(self.0.clone())
    }
}

impl std::fmt::Debug for ShareKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Redact the secret in ALL build profiles: only the public key,
        // which is by construction shareable, is ever printable.
        write!(f, "ShareKeys(pubkey={})", self.0.public_key().to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SK_A: [u8; 32] = [0x11; 32];
    const SK_B: [u8; 32] = [0x22; 32];

    /// Golden vector for HKDF-SHA256(salt=32 zero bytes, ikm=0x11*32,
    /// info="greek/share-nostr/v1"), computed with Python's hmac/hashlib
    /// (RFC 5869 single-block expand). Pinned so a future dependency bump or
    /// refactor that silently changed the derivation fails loudly: the
    /// sharing identity's public key depends on every bit.
    const HKDF_GOLDEN_HEX: &str =
        "439a5273423249feed6a6a172b8719c2c85d182ae18f2ecac15a7a76230b81d7";

    #[test]
    fn hkdf_derivation_is_deterministic_and_matches_golden_vector() {
        let a1 = derive_share_secret(&SK_A);
        let a2 = derive_share_secret(&SK_A);
        assert_eq!(a1, a2, "HKDF must be deterministic");
        let golden: [u8; 32] = {
            let mut b = [0u8; 32];
            for i in 0..32 {
                b[i] = u8::from_str_radix(&HKDF_GOLDEN_HEX[i * 2..i * 2 + 2], 16).unwrap();
            }
            b
        };
        assert_eq!(a1, golden, "HKDF-SHA256(salt=None, ikm, info) golden vector");
        let b = derive_share_secret(&SK_B);
        assert_ne!(a1, b, "different sync secrets must yield different share secrets");
    }

    #[test]
    fn share_keys_derives_valid_distinct_keypair() {
        let sync = Keys::new(SecretKey::from_slice(&SK_A).unwrap());
        let share_a = ShareKeys::derive_from_sync(&SK_A).unwrap();
        let share_a2 = ShareKeys::derive_from_sync(&SK_A).unwrap();
        assert_eq!(
            share_a.public_key(),
            share_a2.public_key(),
            "derivation must be stable across calls"
        );
        assert_ne!(
            share_a.public_key(),
            sync.public_key(),
            "sharing pubkey must differ from the sync pubkey"
        );
        assert_ne!(
            ShareKeys::derive_from_sync(&SK_B).unwrap().public_key(),
            share_a.public_key(),
            "per-secret uniqueness"
        );
    }

    #[test]
    fn share_keys_debug_redacts_secret_in_both_profiles() {
        let share = ShareKeys::derive_from_sync(&SK_A).unwrap();
        let dbg = format!("{share:?}");
        assert!(dbg.starts_with("ShareKeys(pubkey="));
        let secret_hex = SecretKey::from_slice(&derive_share_secret(&SK_A))
            .unwrap()
            .to_secret_hex();
        assert!(
            !dbg.contains(&secret_hex),
            "Debug output must never contain the secret hex"
        );
        assert!(
            !format!("{:?}", share.keys()).is_empty(),
            "inner Keys debug exists (throwaway sign-only object)"
        );
    }

    #[test]
    fn from_db_requires_sync_identity_and_derives() {
        let db = greek_core::Database::open(":memory:", None).unwrap();
        assert!(
            matches!(ShareKeys::from_db(&db), Err(ShareError::Identity(_))),
            "missing nostr_nsec must be an explicit identity error"
        );
        let sync = Keys::generate();
        db.conn()
            .execute(
                "INSERT INTO user_settings (key, value) VALUES ('nostr_nsec', ?1)",
                [sync.secret_key().to_secret_hex()],
            )
            .unwrap();
        let share = ShareKeys::from_db(&db).unwrap();
        assert_ne!(share.public_key(), sync.public_key());
    }

    #[test]
    fn sign_note_produces_kind1_valid_event() {
        let share = ShareKeys::derive_from_sync(&SK_A).unwrap();
        let event = share.sign_note("hello Greek world").unwrap();
        assert_eq!(event.kind, nostr_sdk::prelude::Kind::TextNote);
        assert_eq!(event.pubkey, share.public_key());
        assert_eq!(event.content, "hello Greek world");
        assert!(event.verify().is_ok(), "signature must verify");
    }
}
