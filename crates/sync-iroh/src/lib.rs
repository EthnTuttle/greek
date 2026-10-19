//! iroh-docs sync transport for the Greek learning app (SYNC-WIRE.md §3).
//!
//! One iroh-docs document == one user's notebook: each device holds its own
//! author key, review events are unique-keyed entries under `reviews/…`, and
//! settings ride `settings/<key>` entries as `{value, hlc}` JSON (LWW applied
//! downstream by `greek_core::sync`). Sharing is a `ShareMode::Write` ticket
//! (embeds the namespace secret — treat as a credential, QR-only).
//!
//! Everything lives under `<dir_of_greek_app.db>/iroh-sync/`: the endpoint
//! secret (`endpoint.key`), the blobs FsStore (`blobs/`), the docs redb
//! (`docs/`), and the known-peer list (`peers.json`). Async API on tokio.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use greek_core::sync::ReviewEvent;
use iroh::endpoint::presets;
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointAddr, PublicKey, SecretKey};
use iroh_blobs::api::Store as BlobStore;
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::BlobsProtocol;
use iroh_docs::api::protocol::{AddrInfoOptions, ShareMode};
use iroh_docs::api::Doc;
use iroh_docs::engine::LiveEvent;
use iroh_docs::protocol::Docs;
use iroh_docs::store::Query;
use iroh_docs::{AuthorId, DocTicket};
use iroh_gossip::net::Gossip;
use n0_future::StreamExt;

/// QR payload scheme (SYNC-WIRE.md §3.3); joining also accepts the raw ticket.
pub const TICKET_SCHEME_PREFIX: &str = "greeksync://ticket/";

/// Upper bound for waiting on live convergence inside [`SyncIrohTransport::sync_once`].
const SYNC_WAIT_TIMEOUT: Duration = Duration::from_secs(20);

/// Live iroh sync node for one device: endpoint + router + docs engine +
/// the open notebook doc (once created or joined).
pub struct SyncIrohTransport {
    _router: Router,
    endpoint: Endpoint,
    docs: Docs,
    blobs: BlobStore,
    _gossip: Gossip,
    data_dir: Option<PathBuf>, // None in memory-mode tests
    state: Mutex<DocState>,
}

struct DocState {
    doc: Option<Doc>,
    author: Option<AuthorId>,
    /// Peers learned from tickets. All `EndpointAddr` (as received) in-session;
    /// on-disk persistence stores node ids only (`peers.json`).
    peers: Vec<EndpointAddr>,
}

/// Which preset cluster to bind with: production uses n0 (relay + DNS
/// discovery, WAN-capable); loopback tests use `Empty` like the spike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetMode {
    N0,
    /// Hermetic loopback mode: `presets::Empty` — no relay, no DNS
    /// discovery; peers dial only the direct addresses carried in the
    /// ticket. Works host↔host on one machine or across two devices on
    /// the same LAN/VPN. Used by cross-process E2E harnesses; Android
    /// production stays on N0.
    Loopback,
}

impl SyncIrohTransport {
    /// Open (or create) the persistent sync node rooted at `data_dir`
    /// (the app passes `<dir_of_greek_app.db>/iroh-sync/`).
    pub async fn open(data_dir: &Path) -> Result<Self> {
        Self::spawn(Some(data_dir), NetMode::N0).await
    }

    /// Like [`open`], but in hermetic loopback mode (see [`NetMode::Loopback`]).
    /// Selected by callers via the `GREEK_IROH_NET=loopback` env var.
    pub async fn open_loopback(data_dir: &Path) -> Result<Self> {
        Self::spawn(Some(data_dir), NetMode::Loopback).await
    }

    async fn spawn(data_dir: Option<&Path>, net: NetMode) -> Result<Self> {
        // Endpoint identity: persisted so the device keeps one node id across
        // restarts (SYNC-WIRE.md §3.4 — tickets/peers reference it).
        let secret_key = match data_dir {
            Some(dir) => load_or_create_endpoint_key(dir)?,
            None => SecretKey::generate(),
        };
        // ring crypto provider must be passed explicitly (spike gotcha;
        // presets::Empty sets nothing mandatory).
        let crypto = std::sync::Arc::new(rustls::crypto::ring::default_provider());
        let endpoint = match net {
            NetMode::N0 => Endpoint::builder(presets::N0),
            NetMode::Loopback => Endpoint::builder(presets::Empty),
        }
        .crypto_provider(crypto)
        .secret_key(secret_key)
        .bind()
        .await?;

        let gossip = Gossip::builder().spawn(endpoint.clone());
        let blobs: BlobStore = match data_dir {
            Some(dir) => {
                let blobs_dir = dir.join("blobs");
                std::fs::create_dir_all(&blobs_dir)?;
                let fs = FsStore::load(&blobs_dir).await?;
                fs.into()
            }
            None => iroh_blobs::store::mem::MemStore::new().into(),
        };
        let docs = match data_dir {
            Some(dir) => {
                let docs_dir = dir.join("docs");
                std::fs::create_dir_all(&docs_dir)?;
                Docs::persistent(docs_dir)
            }
            None => Docs::memory(),
        }
        .spawn(endpoint.clone(), blobs.clone(), gossip.clone())
        .await?;

        let router = Router::builder(endpoint.clone())
            .accept(iroh_blobs::ALPN, BlobsProtocol::new(&blobs, None))
            .accept(iroh_gossip::net::GOSSIP_ALPN, gossip.clone())
            .accept(iroh_docs::ALPN, docs.clone())
            .spawn();

        // Recover the notebook doc from a previous run (v1: the first doc).
        let doc = first_or_no_doc(&docs).await?;
        // Recover known peers (node ids only; dialing goes through discovery).
        let peers = data_dir.map(load_peers_file).transpose()?.unwrap_or_default();

        Ok(Self {
            _router: router,
            endpoint,
            docs,
            blobs,
            _gossip: gossip,
            data_dir: data_dir.map(|d| d.to_path_buf()),
            state: Mutex::new(DocState {
                doc,
                author: None,
                peers,
            }),
        })
    }
}

/// Open the first known doc, if any (v1 uses exactly one notebook doc).
async fn first_or_no_doc(docs: &Docs) -> Result<Option<Doc>> {
    let list = docs.list().await?;
    tokio::pin!(list);
    while let Some(item) = list.next().await {
        let (id, _cap) = item?;
        return Ok(docs.open(id).await?);
    }
    Ok(None)
}

const ENDPOINT_KEY_FILE: &str = "endpoint.key";
const PEERS_FILE: &str = "peers.json";

fn load_or_create_endpoint_key(dir: &Path) -> Result<SecretKey> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(ENDPOINT_KEY_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let arr: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .context("endpoint.key corrupt (want 32 bytes)")?;
            Ok(SecretKey::from_bytes(&arr))
        }
        Err(_) => {
            let key = SecretKey::generate();
            write_private_file(&path, &key.to_bytes())?;
            Ok(key)
        }
    }
}

/// Write a file with owner-only permissions where the OS supports modes —
/// endpoint keys and peer lists shouldn't be world-readable.
fn write_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(bytes)?;
    }
    #[cfg(not(unix))]
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Stored peer list: hex node ids (the sharer's iroh public keys).
fn load_peers_file(dir: &Path) -> Result<Vec<EndpointAddr>> {
    let path = dir.join(PEERS_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Ok(Vec::new()),
    };
    let ids: Vec<String> = serde_json::from_str(&text).context("peers.json corrupt")?;
    let mut out = Vec::new();
    for id in ids {
        let pk: PublicKey = id.parse().with_context(|| format!("bad peer id {id:?}"))?;
        out.push(EndpointAddr::from_parts(pk, []));
    }
    Ok(out)
}

fn save_peers_file(dir: &Path, peers: &[EndpointAddr]) -> Result<()> {
    let ids: Vec<String> = peers.iter().map(|p| p.id.to_string()).collect();
    let text = serde_json::to_string(&ids)?;
    write_private_file(&dir.join(PEERS_FILE), text.as_bytes())
}

/// Doc entry key for a review event (SYNC-WIRE.md §3.2):
/// `reviews/<hlc-ms:013>-<device_id>-<uuid-first-8-hex>` — lexicographic key
/// order == chronological order; uniqueness via device id + uuid prefix.
pub fn review_entry_key(event: &ReviewEvent) -> Vec<u8> {
    let ms: i64 = event
        .hlc
        .parse::<greek_core::sync::Hlc>()
        .map(|h| h.ms)
        .unwrap_or(0);
    let uuid8: String = event
        .uuid
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(8)
        .collect();
    format!("reviews/{:013}-{}-{}", ms, event.device_id, uuid8).into_bytes()
}

impl SyncIrohTransport {
    /// The open notebook doc and this device's author id (lazily obtained:
    /// creates the doc / default author on first use).
    async fn doc_and_author(&self) -> Result<(Doc, AuthorId)> {
        let author = match self.docs.author_default().await {
            Ok(a) => a,
            Err(_) => {
                let a = self.docs.author_create().await?;
                self.docs.author_set_default(a).await?;
                a
            }
        };
        let doc = {
            let st = self.state.lock().expect("doc state mutex poisoned");
            st.doc.clone()
        };
        let doc = match doc {
            Some(d) => d,
            None => {
                let fresh = match first_or_no_doc(&self.docs).await? {
                    Some(d) => d,
                    None => self.docs.create().await?,
                };
                let mut st = self.state.lock().expect("doc state mutex poisoned");
                st.doc.get_or_insert(fresh).clone()
            }
        };
        self.state
            .lock()
            .expect("doc state mutex poisoned")
            .author = Some(author);
        Ok((doc, author))
    }

    /// Create the notebook doc if needed and return a ShareMode::Write ticket
    /// (QR payload). The ticket embeds the namespace secret: it is a
    /// credential and must only travel over the QR channel.
    pub async fn create_or_get_ticket(&self) -> Result<String> {
        let (doc, _author) = self.doc_and_author().await?;
        let ticket = doc
            .share(ShareMode::Write, AddrInfoOptions::RelayAndAddresses)
            .await?;
        Ok(ticket.to_string())
    }

    /// Import a notebook from a ticket (`greeksync://ticket/…` URL or the raw
    /// ticket string) and remember its nodes as sync peers. Attempts an
    /// immediate sync but tolerates the sharer being unreachable — the next
    /// [`Self::sync_once`] retries.
    pub async fn join_from_ticket(&self, ticket: &str) -> Result<()> {
        let raw = ticket.strip_prefix(TICKET_SCHEME_PREFIX).unwrap_or(ticket);
        let parsed: DocTicket = raw.parse().map_err(|e| {
            anyhow::anyhow!("bad sync ticket (expected {TICKET_SCHEME_PREFIX}<ticket>): {e}")
        })?;
        let nodes = parsed.nodes.clone();
        if nodes.is_empty() {
            anyhow::bail!("sync ticket carries no peer addresses");
        }
        let doc = self.docs.import(parsed).await?;
        {
            let mut st = self.state.lock().expect("doc state mutex poisoned");
            st.peers = nodes.clone();
            st.doc = Some(doc.clone());
        }
        if let Some(dir) = &self.data_dir {
            save_peers_file(dir, &nodes)?;
        }
        // Best-effort immediate sync; failure here is not fatal.
        let _ = doc.start_sync(nodes).await;
        Ok(())
    }

    /// Write local review events and settings into the doc. Returns the
    /// number of entries attempted (set_bytes is upsert — replays of the
    /// same keys are no-ops).
    pub async fn push_local(
        &self,
        reviews: &[ReviewEvent],
        settings: &[(String, String, String)],
    ) -> Result<u32> {
        let (doc, author) = self.doc_and_author().await?;
        let mut n = 0u32;
        for ev in reviews {
            let key = review_entry_key(ev);
            let content = serde_json::to_vec(ev)?;
            doc.set_bytes(author, key, content).await?;
            n += 1;
        }
        for (key, value, hlc) in settings {
            let k = format!("settings/{key}").into_bytes();
            let content =
                serde_json::json!({ "value": value, "hlc": hlc }).to_string();
            doc.set_bytes(author, k, content.into_bytes()).await?;
            n += 1;
        }
        Ok(n)
    }

    /// Read ALL doc entries back out as review events + settings triples.
    /// Dedup/merge correctness is downstream (`apply_review_events` is
    /// uuid-dedup, `apply_settings` is HLC-LWW); settings take the max-hlc
    /// entry per key across authors.
    pub async fn pull_remote(
        &self,
    ) -> Result<(Vec<ReviewEvent>, Vec<(String, String, String)>)> {
        let doc = {
            let st = self.state.lock().expect("doc state mutex poisoned");
            st.doc.clone()
        };
        let Some(doc) = doc else {
            bail!("no notebook doc yet (create or join first)");
        };
        let stream = doc.get_many(Query::all()).await?;
        tokio::pin!(stream);
        let mut reviews = Vec::new();
        let mut settings: std::collections::HashMap<String, (String, String)> =
            std::collections::HashMap::new();
        while let Some(item) = stream.next().await {
            let entry = item?;
            let key = entry.key();
            let bytes = self
                .blobs
                .blobs()
                .get_bytes(entry.content_hash())
                .await
                .with_context(|| {
                    format!("missing blob for key {:?}", String::from_utf8_lossy(key))
                })?;
            if let Some(rest) = key.strip_prefix(b"reviews/") {
                let _ = rest;
                let ev: ReviewEvent = serde_json::from_slice(&bytes)
                    .with_context(|| format!("bad review entry {key:?}"))?;
                reviews.push(ev);
            } else if let Some(kname) = key.strip_prefix(b"settings/") {
                let parsed: serde_json::Value = serde_json::from_slice(&bytes)?;
                let value = parsed
                    .get("value")
                    .and_then(|v| v.as_str())
                    .context("settings entry missing value")?
                    .to_string();
                let hlc = parsed
                    .get("hlc")
                    .and_then(|v| v.as_str())
                    .context("settings entry missing hlc")?
                    .to_string();
                let kname = String::from_utf8(kname.to_vec())?;
                settings
                    .entry(kname)
                    .and_modify(|cur| {
                        if hlc > cur.1 {
                            *cur = (value.clone(), hlc.clone());
                        }
                    })
                    .or_insert((value, hlc));
            }
        }
        let settings: Vec<(String, String, String)> = settings
            .into_iter()
            .map(|(k, (v, h))| (k, v, h))
            .collect();
        Ok((reviews, settings))
    }

    /// Trigger one live sync round against all known peers and wait (bounded)
    /// for convergence events. An empty peer list (or unreachable peers) is
    /// tolerated: this is a no-op Ok — sync is opportunistic.
    pub async fn sync_once(&self) -> Result<()> {
        let (doc, peers) = {
            let st = self.state.lock().expect("doc state mutex poisoned");
            (st.doc.clone(), st.peers.clone())
        };
        let Some(doc) = doc else {
            return Ok(());
        };
        if peers.is_empty() {
            return Ok(());
        }
        let mut events = doc.subscribe().await?;
        doc.start_sync(peers.clone()).await?;
        let mut finished = 0usize;
        let deadline = std::time::Instant::now() + SYNC_WAIT_TIMEOUT;
        while finished < peers.len() {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, events.next()).await {
                Ok(Some(Ok(LiveEvent::SyncFinished(_)))) => finished += 1,
                Ok(Some(Ok(_))) => {}
                Ok(Some(Err(e))) => bail!("live sync error: {e}"),
                Ok(None) => break,
                Err(_) => break, // timed out: peers presumably offline
            }
        }
        Ok(())
    }

    /// This device's iroh node id (hex public key) — handy for diagnostics.
    pub fn node_id(&self) -> String {
        self.endpoint.id().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use greek_core::sync::Hlc;

    fn fake_event(i: u32, device: &str) -> ReviewEvent {
        ReviewEvent {
            uuid: uuid::Uuid::new_v4().to_string(),
            card_id: format!("card-{i:04}"),
            timestamp: format!("2026-10-17T10:{:02}:00Z", i % 60),
            rating: (i % 4) + 1,
            elapsed_days: 1.0,
            scheduled_days: 2.0,
            source: "flashcard".into(),
            device_id: device.into(),
            hlc: Hlc {
                ms: 1_760_000_000_000 + i as i64 * 1000,
                counter: 0,
                node: device.into(),
            }
            .to_string(),
        }
    }

    async fn loopback_transport() -> Result<SyncIrohTransport> {
        SyncIrohTransport::spawn(None, NetMode::Loopback).await
    }

    #[test]
    fn key_layout_matches_wire_spec() {
        let ev = fake_event(7, "dev-A");
        let key = review_entry_key(&ev);
        let s = String::from_utf8(key).unwrap();
        assert!(s.starts_with("reviews/"), "key: {s}");
        // reviews/ + 13-digit ms + "-" + device + "-" + 8 hex
        let body = &s["reviews/".len()..];
        // device_id is a uuid (contains dashes), so split structurally:
        // [0..13] zero-padded hlc ms, '-' separator, … , last 8 = uuid hex.
        assert!(body[..13].chars().all(|c| c.is_ascii_digit()), "key: {s}");
        assert_eq!(&body[13..14], "-");
        let uuid8 = &body[body.len() - 8..];
        assert!(uuid8.chars().all(|c| c.is_ascii_hexdigit()), "key: {s}");
        assert!(body.contains("dev-A"));
        // chronological: higher hlc.ms sorts later
        let later = Hlc { ms: 1_760_000_009_000, counter: 0, node: "dev-A".into() }.to_string();
        let mut ev2 = ev.clone();
        ev2.hlc = later;
        assert!(review_entry_key(&ev2) > review_entry_key(&ev));
    }

    #[tokio::test]
    async fn ticket_parse_errors_are_clear() {
        let t = loopback_transport().await.unwrap();
        let err = t.join_from_ticket("greeksync://ticket/not-a-ticket").await;
        assert!(err.is_err(), "garbage ticket must fail");
        let msg = format!("{:#}", err.unwrap_err());
        assert!(msg.contains("bad sync ticket"), "msg: {msg}");
        let err2 = t.join_from_ticket("").await;
        assert!(err2.is_err());
    }

    #[tokio::test]
    async fn two_endpoints_converge_both_ways() -> Result<()> {
        let x = loopback_transport().await?;
        let y = loopback_transport().await?;

        // X pushes 3 reviews + a setting, Y joins, syncs, and sees them.
        let reviews_x: Vec<_> = (0..3).map(|i| fake_event(i, "dev-X")).collect();
        let n = x
            .push_local(&reviews_x, &[("theme".into(), "dark".into(), "0001760000000000-0000-devX".into())])
            .await?;
        assert_eq!(n, 4);
        let ticket = format!("{TICKET_SCHEME_PREFIX}{}", x.create_or_get_ticket().await?);
        y.join_from_ticket(&ticket).await?;
        y.sync_once().await?;
        let (reviews, settings) = y.pull_remote().await?;
        assert_eq!(reviews.len(), 3, "Y must see X's 3 reviews");
        assert_eq!(settings.len(), 1);
        assert_eq!(settings[0], ("theme".to_string(), "dark".to_string(), "0001760000000000-0000-devX".to_string()));

        // Y pushes 2 of its own; X syncs via remembered peers and sees 5.
        let reviews_y: Vec<_> = (3..5).map(|i| fake_event(i, "dev-Y")).collect();
        y.push_local(&reviews_y, &[]).await?;
        // X doesn't know Y's address from a ticket; in-loopback it receives
        // Y's entries when Y starts the sync instead. Do that.
        y.sync_once().await?;
        // Let the accepter side finish committing before we read.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let (reviews_back, _) = x.pull_remote().await?;
        assert_eq!(reviews_back.len(), 5, "X must see its own 3 + Y's 2");
        Ok(())
    }

    #[tokio::test]
    async fn push_is_idempotent_same_key() -> Result<()> {
        let x = loopback_transport().await?;
        let evs: Vec<_> = (0..2).map(|i| fake_event(i, "dev-X")).collect();
        assert_eq!(x.push_local(&evs, &[]).await?, 2);
        // Replay the same events: keys unchanged, entry count unchanged.
        assert_eq!(x.push_local(&evs, &[]).await?, 2);
        let (reviews, _) = x.pull_remote().await?;
        assert_eq!(reviews.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn sync_once_without_peers_or_doc_is_ok() -> Result<()> {
        let x = loopback_transport().await?;
        x.sync_once().await?; // no doc, no peers: no-op
        x.create_or_get_ticket().await?; // now a doc exists, still no peers
        x.sync_once().await?;
        Ok(())
    }

    /// Loopback-only here; live relay + WAN traversal is a spike follow-up
    /// per SYNC-WIRE.md §3.5 / SPIKE-IROH.md OPEN RISKS (not CI-gated).
    #[tokio::test]
    #[ignore]
    async fn live_wan_roundtrip_via_relay() {
        // Requires two hosts and a fresh ticket exchanged out of band.
    }
}
