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
//! (`docs/`), the known-peer list (`peers.json`; node ids + last-observed
//! transport addrs), and the ACTIVE namespace id (`active_doc`, S9 — reopen
//! must not fall back to the first stored doc after a join). Async API on
//! tokio.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use greek_core::sync::ReviewEvent;
use iroh::endpoint::presets;
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointAddr, PublicKey, SecretKey, TransportAddr};
use iroh_blobs::api::Store as BlobStore;
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::BlobsProtocol;
use iroh_docs::api::protocol::{AddrInfoOptions, ShareMode};
use iroh_docs::api::Doc;
use iroh_docs::engine::LiveEvent;
use iroh_docs::protocol::Docs;
use iroh_docs::store::Query;
use iroh_docs::{AuthorId, DocTicket, NamespaceId};
use iroh_gossip::net::Gossip;
use n0_future::StreamExt;

/// QR payload scheme (SYNC-WIRE.md §3.3); joining also accepts the raw ticket.
pub const TICKET_SCHEME_PREFIX: &str = "greeksync://ticket/";

/// Upper bound for waiting on live convergence inside [`SyncIrohTransport::sync_once`].
const SYNC_WAIT_TIMEOUT: Duration = Duration::from_secs(20);

/// Live iroh sync node for one device: endpoint + router + docs engine +
/// the open notebook doc (once created or joined).
pub struct SyncIrohTransport {
    router: Router,
    endpoint: Endpoint,
    docs: Docs,
    blobs: BlobStore,
    _gossip: Gossip,
    data_dir: Option<PathBuf>, // None in memory-mode tests
    state: Arc<Mutex<DocState>>,
    /// Background subscriber on the active doc that records observed peers
    /// (S8); rotated when the active doc changes.
    watch: Mutex<Option<PeerWatcher>>,
}

struct PeerWatcher {
    doc: NamespaceId,
    task: tokio::task::JoinHandle<()>,
}

struct DocState {
    doc: Option<Doc>,
    author: Option<AuthorId>,
    /// Peers learned from tickets, plus peers OBSERVED during sync (S8).
    /// In-session they carry the best-known `EndpointAddr`; on disk
    /// (`peers.json`) node ids + the last-observed transport addrs are kept
    /// so a restarted endpoint can re-dial without a discovery service.
    peers: Vec<EndpointAddr>,
    /// True once a join happened or a peer was observed (S8c): gates whether
    /// an all-empty peer list is a clean no-op (fresh device) or a degraded
    /// "paired, waiting for the peer" round.
    ever_paired: bool,
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

        // Recover the notebook doc from a previous run: the PERSISTED ACTIVE
        // namespace wins (S9) — a device that created doc A then joined doc B
        // reopens B, not the first stored namespace.
        let doc = resolve_or_no_doc(&docs, data_dir).await?;
        // Recover known peers; an existing peers.json (even an empty one)
        // means the device paired before (S8c degraded-round gate).
        let (peers, ever_paired) = match data_dir {
            Some(dir) => load_peers_file(dir)?,
            None => (Vec::new(), false),
        };

        let transport = Self {
            router,
            endpoint,
            docs,
            blobs,
            _gossip: gossip,
            data_dir: data_dir.map(|d| d.to_path_buf()),
            state: Arc::new(Mutex::new(DocState {
                doc: doc.clone(),
                author: None,
                peers,
                ever_paired,
            })),
            watch: Mutex::new(None),
        };
        if let Some(doc) = &doc {
            // S8: resume live sync on the active doc right away so a peer can
            // dial us back after a restart (no new ticket needed), and record
            // whoever shows up.
            transport.ensure_live(doc).await;
        }
        Ok(transport)
    }
}

impl Drop for SyncIrohTransport {
    fn drop(&mut self) {
        if let Ok(mut w) = self.watch.lock() {
            if let Some(w) = w.take() {
                w.task.abort();
            }
        }
    }
}

/// Pick the doc to use: the persisted ACTIVE namespace id wins; if it is not
/// among the stored namespaces (or none was ever persisted), fall back to the
/// first stored doc — logging the mismatch (S9) — and persist the pick so the
/// namespace the S7 push cursor fingerprint derives from stays stable.
async fn resolve_or_no_doc(docs: &Docs, data_dir: Option<&Path>) -> Result<Option<Doc>> {
    let list = docs.list().await?;
    tokio::pin!(list);
    let mut stored = Vec::new();
    while let Some(item) = list.next().await {
        stored.push(item?.0);
    }
    let persisted = data_dir.and_then(load_active_doc);
    let chosen = match persisted {
        Some(want) if stored.contains(&want) => Some(want),
        Some(want) => {
            tracing::warn!(
                wanted = %want,
                stored = stored.len(),
                "persisted active_doc id is not among the stored namespaces; \
                 falling back to the first stored doc"
            );
            stored.first().copied()
        }
        None => stored.first().copied(),
    };
    let Some(id) = chosen else {
        return Ok(None);
    };
    if let Some(dir) = data_dir {
        if persisted != Some(id) {
            save_active_doc(dir, &id)?;
        }
    }
    docs.open(id).await
}

const ENDPOINT_KEY_FILE: &str = "endpoint.key";
const PEERS_FILE: &str = "peers.json";
const ACTIVE_DOC_FILE: &str = "active_doc";

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

/// Persisted ACTIVE notebook namespace id (S9). 0600 like the other
/// secrets-adjacent files in this dir (it embeds in no credential, but it's
/// the pairing surface's identity).
fn save_active_doc(dir: &Path, id: &NamespaceId) -> Result<()> {
    write_private_file(&dir.join(ACTIVE_DOC_FILE), id.to_string().as_bytes())
}

fn load_active_doc(dir: &Path) -> Option<NamespaceId> {
    let text = std::fs::read_to_string(dir.join(ACTIVE_DOC_FILE)).ok()?;
    text.trim().parse().ok()
}

/// Stored peer entry. v2 (`Full`) remembers the transport addrs last observed
/// while the peer was online, so a restarted endpoint can re-dial it in
/// discovery-less setups (hermetic loopback has no relay/DNS lookup); v1
/// entries (bare hex node id string) still load, addr-less.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
enum PeerEntry {
    Full { id: String, addrs: Vec<TransportAddr> },
    IdOnly(String),
}

/// Stored peer list: hex node ids plus (since S8) last-observed addrs.
/// Returns the loaded peers and whether a peers file existed at all.
fn load_peers_file(dir: &Path) -> Result<(Vec<EndpointAddr>, bool)> {
    let path = dir.join(PEERS_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Ok((Vec::new(), false)),
    };
    let entries: Vec<PeerEntry> = serde_json::from_str(&text).context("peers.json corrupt")?;
    let mut out = Vec::new();
    for entry in entries {
        let (id, addrs) = match entry {
            PeerEntry::IdOnly(id) => (id, Vec::new()),
            PeerEntry::Full { id, addrs } => (id, addrs),
        };
        let pk: PublicKey = id.parse().with_context(|| format!("bad peer id {id:?}"))?;
        out.push(EndpointAddr::from_parts(pk, addrs));
    }
    Ok((out, true))
}

fn save_peers_file(dir: &Path, peers: &[EndpointAddr]) -> Result<()> {
    let entries: Vec<PeerEntry> = peers
        .iter()
        .map(|p| PeerEntry::Full {
            id: p.id.to_string(),
            addrs: p.addrs.iter().cloned().collect(),
        })
        .collect();
    let text = serde_json::to_string(&entries)?;
    write_private_file(&dir.join(PEERS_FILE), text.as_bytes())
}

/// Fold `addr` into `peers` (dedup by node id). Fresh addressing info
/// replaces the stored one — direct addrs are per-boot on the peer, so stale
/// entries age out instead of accumulating. Returns whether anything changed.
fn merge_peer(peers: &mut Vec<EndpointAddr>, addr: EndpointAddr) -> bool {
    match peers.iter_mut().find(|p| p.id == addr.id) {
        Some(existing) if !addr.is_empty() && existing.addrs != addr.addrs => {
            existing.addrs = addr.addrs;
            true
        }
        Some(_) => false,
        None => {
            peers.push(addr);
            true
        }
    }
}

/// A sync-relevant event mentioned `pk` (a live neighbor, the source of a
/// remote entry, or a finished sync exchange): fold it into the known-peer
/// list and persist. This is how the ticket CREATOR learns the joiner — no
/// ticket ever travels joiner→creator (S8a).
async fn note_observed_peer(
    state: &Arc<Mutex<DocState>>,
    endpoint: &Endpoint,
    dir: Option<&Path>,
    pk: PublicKey,
) {
    if pk == endpoint.id() {
        return;
    }
    // Best addressing info the endpoint has right now (loopback dials need
    // the direct addrs learned from the live connection).
    let addrs: Vec<TransportAddr> = endpoint
        .remote_info(pk)
        .await
        .map(|info| info.into_addrs().map(|a| a.into_addr()).collect())
        .unwrap_or_default();
    let snapshot;
    {
        let mut st = state.lock().expect("doc state mutex poisoned");
        if !merge_peer(&mut st.peers, EndpointAddr::from_parts(pk, addrs)) {
            return;
        }
        st.ever_paired = true;
        snapshot = st.peers.clone();
    }
    if let Some(dir) = dir {
        if let Err(e) = save_peers_file(dir, &snapshot) {
            tracing::warn!(error = %e, "failed to persist observed peer");
        }
    }
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
    /// creates the doc / default author on first use). The resolved doc is
    /// put into live sync and watched for peer arrivals (S8).
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
                let fresh = match resolve_or_no_doc(&self.docs, self.data_dir.as_deref()).await? {
                    Some(d) => d,
                    None => {
                        let d = self.docs.create().await?;
                        // S9: a brand-new notebook becomes the persisted
                        // ACTIVE doc immediately.
                        if let Some(dir) = &self.data_dir {
                            save_active_doc(dir, &d.id())?;
                        }
                        d
                    }
                };
                let mut st = self.state.lock().expect("doc state mutex poisoned");
                st.doc.get_or_insert(fresh).clone()
            }
        };
        self.state
            .lock()
            .expect("doc state mutex poisoned")
            .author = Some(author);
        self.ensure_live(&doc).await;
        Ok((doc, author))
    }

    /// Put `doc` into live sync with the currently-known peers and make sure
    /// a peer recorder is subscribed to it. No-op when already watching this
    /// doc. `start_sync` is additive/idempotent (rejoins the gossip swarm and
    /// re-adds peer addresses to the endpoint lookup), so a retry on the NEXT
    /// round is cheap; a failure here therefore just waits for `sync_once`.
    async fn ensure_live(&self, doc: &Doc) {
        let doc_id = doc.id();
        let watching = self
            .watch
            .lock()
            .expect("watch mutex poisoned")
            .as_ref()
            .is_some_and(|w| w.doc == doc_id);
        if watching {
            return;
        }
        let peers = {
            let st = self.state.lock().expect("doc state mutex poisoned");
            st.peers.clone()
        };
        if let Err(e) = doc.start_sync(peers).await {
            tracing::warn!(error = %e, "start_sync failed (continuing; sync_once retries)");
        }
        self.spawn_peer_recorder(doc).await;
    }

    /// Subscribe `doc` and record every peer it reveals — `NeighborUp`,
    /// `InsertRemote::from`, and `SyncFinished::peer` (success or failure;
    /// we observed an endpoint id either way) — into `state.peers` and
    /// `peers.json` (S8a). Superseded watchers are aborted; re-calling with
    /// the same doc is a no-op.
    async fn spawn_peer_recorder(&self, doc: &Doc) {
        let doc_id = doc.id();
        {
            let mut w = self.watch.lock().expect("watch mutex poisoned");
            if w.as_ref().is_some_and(|cur| cur.doc == doc_id) {
                return;
            }
            if let Some(old) = w.take() {
                old.task.abort();
            }
        }
        let mut events = match doc.subscribe().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "doc subscribe for peer recording failed");
                return;
            }
        };
        let state = Arc::clone(&self.state);
        let endpoint = self.endpoint.clone();
        let dir = self.data_dir.clone();
        let task = tokio::spawn(async move {
            while let Some(item) = events.next().await {
                let peer = match item {
                    Ok(LiveEvent::NeighborUp(pk)) => Some(pk),
                    Ok(LiveEvent::InsertRemote { from, .. }) => Some(from),
                    Ok(LiveEvent::SyncFinished(ev)) => Some(ev.peer),
                    Ok(_) => None,
                    Err(e) => {
                        tracing::warn!(error = %e, "peer recorder event error");
                        None
                    }
                };
                if let Some(pk) = peer {
                    note_observed_peer(&state, &endpoint, dir.as_deref(), pk).await;
                }
            }
        });
        *self.watch.lock().expect("watch mutex poisoned") = Some(PeerWatcher { doc: doc_id, task });
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
    /// ticket string), make it the ACTIVE doc (persisted, S9), and remember
    /// its nodes as sync peers. Attempts an immediate sync but tolerates the
    /// sharer being unreachable — the next [`Self::sync_once`] retries.
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
            st.peers = nodes;
            st.doc = Some(doc.clone());
            st.ever_paired = true;
        }
        if let Some(dir) = &self.data_dir {
            save_peers_file(dir, &self.state.lock().expect("doc state mutex poisoned").peers)?;
            save_active_doc(dir, &doc.id())?;
        }
        // Enter live sync (best-effort immediate round; failure is not fatal,
        // the next sync_once retries) and start recording observed peers.
        self.ensure_live(&doc).await;
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
            // A blob may lag its entry across the network (iroh-docs syncs
            // entry metadata first). Skip it for this round — the next
            // pull retries. Never abort the whole pull on one straggler.
            let bytes = match self
                .blobs
                .blobs()
                .get_bytes(entry.content_hash())
                .await
            {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(
                        key = %String::from_utf8_lossy(key),
                        error = %e,
                        "entry blob not yet available; skipping this round"
                    );
                    continue;
                }
            };
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
    /// for convergence events.
    ///
    /// Clean no-op only when nothing was ever paired (fresh device with its
    /// own notebook). A device that DID pair reports a degraded round instead
    /// of clean success (S8b/c): an empty peer list, or a round whose
    /// `SyncFinished` events are ALL failures (e.g. every peer unreachable),
    /// surfaces as `Err`. Rounds producing no signal at all (deadline hit,
    /// zero events) stay tolerated as Ok — sync is opportunistic. Partial
    /// rounds (at least one peer synced OK) are Ok with a warning for the
    /// failed ones.
    pub async fn sync_once(&self) -> Result<()> {
        let (doc, peers, ever_paired) = {
            let st = self.state.lock().expect("doc state mutex poisoned");
            (st.doc.clone(), st.peers.clone(), st.ever_paired)
        };
        let Some(doc) = doc else {
            return Ok(());
        };
        if peers.is_empty() {
            if ever_paired {
                // Paired before but nobody to dial now: stay live so the peer
                // can dial US, and say so instead of reporting a clean round.
                self.ensure_live(&doc).await;
                bail!(
                    "sync incomplete: paired notebook has no known peers to dial yet \
                     (waiting for the peer device to come online)"
                );
            }
            return Ok(());
        }
        let targets: HashSet<PublicKey> = peers.iter().map(|p| p.id).collect();
        let target_count = targets.len();
        let mut events = doc.subscribe().await?;
        doc.start_sync(peers).await?;
        let mut ok: HashSet<PublicKey> = HashSet::new();
        let mut failed: HashMap<PublicKey, String> = HashMap::new();
        let deadline = std::time::Instant::now() + SYNC_WAIT_TIMEOUT;
        while ok.len() + failed.len() < target_count {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, events.next()).await {
                // S8b: only a SUCCESSFUL SyncFinished counts as completion.
                Ok(Some(Ok(LiveEvent::SyncFinished(ev)))) if targets.contains(&ev.peer) => {
                    match ev.result {
                        Ok(_) => {
                            ok.insert(ev.peer);
                        }
                        // iroh-docs 0.101 wart: the post-exchange QUIC close
                        // handshake (finish/stopped/read_to_end) races under
                        // mutual live-sync dials and surfaces as a FAILED
                        // SyncFinished even though the set-reconciliation
                        // exchange itself completed. "Failed to close …" is
                        // only produced by those post-exchange close steps
                        // (Connect/Abort/Sync failures carry distinct
                        // variants), so treat it as convergence and move on
                        // rather than failing the round on a benign race.
                        Err(e) if e.starts_with("Failed to close") => {
                            tracing::debug!(peer = %ev.peer, error = %e, "ignoring post-exchange close race");
                            ok.insert(ev.peer);
                        }
                        Err(e) => {
                            failed.insert(ev.peer, e);
                        }
                    }
                }
                Ok(Some(Ok(_))) => {}
                Ok(Some(Err(e))) => bail!("live sync error: {e}"),
                Ok(None) => break,
                Err(_) => break, // deadline hit, no signal: tolerating as opportunity lost
            }
        }
        if !failed.is_empty() && ok.is_empty() {
            let mut reasons: Vec<String> = failed
                .iter()
                .map(|(pk, e)| format!("{pk}: {e}"))
                .collect();
            reasons.sort();
            bail!(
                "live sync failed against all {target_count} known peer(s): {}",
                reasons.join("; ")
            );
        }
        if !failed.is_empty() {
            tracing::warn!(
                failed = failed.len(),
                ok = ok.len(),
                "partial sync round: some peers failed, at least one converged"
            );
        }
        Ok(())
    }

    /// This device's iroh node id (hex public key) — handy for diagnostics.
    pub fn node_id(&self) -> String {
        self.endpoint.id().to_string()
    }

    /// Graceful shutdown: stop the peer recorder, shut down the router
    /// (which shuts down docs/gossip/blobs and closes the endpoint). A plain
    /// `drop` leaves flushes to background tasks; tests that reopen the same
    /// data dir in-process must await this first so the redb-backed stores
    /// are fully closed (and flushed) before the next instance opens them.
    pub async fn shutdown(&self) -> Result<()> {
        if let Some(w) = self.watch.lock().expect("watch mutex poisoned").take() {
            w.task.abort();
        }
        self.router.shutdown().await.context("router shutdown")?;
        Ok(())
    }

    /// Stable id (base32 pubkey form) of the currently-open notebook's
    /// namespace — the destination identity used to scope the iroh push
    /// cursor (S7). Lazily creates the doc exactly like the push/ticket
    /// paths do, and follows `join_from_ticket` (the joined doc becomes
    /// current), so a namespace change naturally yields a fresh cursor
    /// scope.
    pub async fn doc_namespace_id(&self) -> Result<String> {
        let (doc, _author) = self.doc_and_author().await?;
        Ok(doc.id().to_string())
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

    /// Log output when RUST_LOG is set (debugging flaky live-sync runs).
    fn init_logs() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
    }

    /// Persistent loopback transport rooted at a fresh temp dir.
    async fn dir_transport(tag: &str) -> Result<(SyncIrohTransport, PathBuf)> {
        let dir = std::env::temp_dir()
            .join(format!("sync-iroh-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir)?;
        let t = SyncIrohTransport::spawn(Some(&dir), NetMode::Loopback).await?;
        Ok((t, dir))
    }

    fn known_peer_ids(t: &SyncIrohTransport) -> Vec<String> {
        t.state
            .lock()
            .expect("doc state mutex poisoned")
            .peers
            .iter()
            .map(|p| p.id.to_string())
            .collect()
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
        init_logs();
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

    /// S9: a device that CREATED doc A and then JOINED doc B must reopen B
    /// (the persisted active_doc), not the first stored namespace — and the
    /// S7 cursor fingerprint (`doc_namespace_id`) scopes to B after restart.
    #[tokio::test]
    async fn reopen_resolves_persisted_active_doc_after_join() -> Result<()> {
        let (a, dir_a) = dir_transport("s9-a").await?;
        let b = loopback_transport().await?;

        // A creates its own notebook first.
        let _ticket_a = a.create_or_get_ticket().await?;
        let ns_a = a.doc_namespace_id().await?;

        // …then joins B's notebook. B's doc is now a SECOND stored namespace.
        let ticket_b = format!("{TICKET_SCHEME_PREFIX}{}", b.create_or_get_ticket().await?);
        a.join_from_ticket(&ticket_b).await?;
        let ns_b = a.doc_namespace_id().await?;
        assert_ne!(ns_a, ns_b);

        // Persisted marker on disk.
        let persisted = load_active_doc(&dir_a).expect("active_doc must be written on join");
        assert_eq!(persisted.to_string(), ns_b);

        // Restart from the same data dir: B's namespace wins (not first-stored).
        a.shutdown().await?;
        drop(a);
        let a_reopened = SyncIrohTransport::spawn(Some(&dir_a), NetMode::Loopback).await?;
        assert_eq!(
            a_reopened.doc_namespace_id().await?,
            ns_b,
            "reopen must resolve the JOINED doc; S7 cursor scope follows it"
        );
        Ok(())
    }

    /// S8b: a round whose SyncFinished events are all FAILURES must not
    /// report clean success.
    #[tokio::test]
    async fn failed_sync_finished_is_reported_not_success() -> Result<()> {
        let x = loopback_transport().await?;
        x.create_or_get_ticket().await?; // doc exists
        {
            // Simulate a paired device that is now unreachable. node-id-only
            // addressing can't resolve in loopback (no relay/DNS), so the
            // dial fails fast with a SyncFinished(Err), not a 20s hang.
            let mut st = x.state.lock().expect("doc state mutex poisoned");
            st.peers = vec![EndpointAddr::new(SecretKey::generate().public())];
            st.ever_paired = true;
        }
        let err = x
            .sync_once()
            .await
            .expect_err("an all-failed round must surface as Err, not Ok");
        let msg = format!("{err:#}");
        assert!(msg.contains("failed against all"), "msg: {msg}");

        // S8c: a zero-peer round on a PAIRED device is degraded, not clean.
        {
            let mut st = x.state.lock().expect("doc state mutex poisoned");
            st.peers = Vec::new();
            st.ever_paired = true;
        }
        let err = x
            .sync_once()
            .await
            .expect_err("paired-but-peerless round must surface as Err, not Ok");
        let msg = format!("{err:#}");
        assert!(msg.contains("no known peers"), "msg: {msg}");
        Ok(())
    }

    /// S8 headline restart E2E (in-process): the ticket creator restarts and
    /// converges WITHOUT re-sharing a ticket — its peer list was repopulated
    /// from peers it OBSERVED during the joiner's sync.
    #[tokio::test]
    async fn restart_e2e_creator_resumes_via_observed_peers() -> Result<()> {
        init_logs();
        let (creator, dir_creator) = dir_transport("s8-creator").await?;
        let (joiner, _dir_joiner) = dir_transport("s8-joiner").await?;
        let joiner_id = joiner.node_id();

        // Creator shares; joiner joins and syncs its own payload in.
        creator
            .push_local(&(0..2).map(|i| fake_event(i, "dev-C")).collect::<Vec<_>>(), &[])
            .await?;
        let ticket = format!("{TICKET_SCHEME_PREFIX}{}", creator.create_or_get_ticket().await?);
        joiner.join_from_ticket(&ticket).await?;
        joiner
            .push_local(&(2..4).map(|i| fake_event(i, "dev-J")).collect::<Vec<_>>(), &[])
            .await?;
        joiner.sync_once().await?;

        // The creator never saw a ticket carrying the joiner — it can only
        // learn the joiner by OBSERVING the accepted sync (S8a).
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !known_peer_ids(&creator).contains(&joiner_id) {
            if std::time::Instant::now() > deadline {
                panic!("creator never observed the joiner (peers: {:?})", known_peer_ids(&creator));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // …and the observation made it to peers.json with dialable addrs.
        let (disk_peers, existed) = load_peers_file(&dir_creator)?;
        assert!(existed, "creator must persist observed peers");
        let on_disk = disk_peers
            .iter()
            .find(|p| p.id.to_string() == joiner_id)
            .expect("joiner must be in the creator's peers.json");
        assert!(!on_disk.is_empty(), "observed peer must carry transport addrs");

        let ns = creator.doc_namespace_id().await?;

        // Phase A sanity: BEFORE restart, the creator must converge to the
        // full 4-entry dataset (its own 2 + the joiner's 2) — i.e. entry
        // content transfer completed while both nodes were up.
        let mut got_pre = 0usize;
        for _ in 0..40 {
            let (reviews, _) = creator.pull_remote().await?;
            got_pre = reviews.len();
            if got_pre == 4 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        assert_eq!(got_pre, 4, "pre-restart convergence check failed (content transfer broken?)");

        // --- creator "restarts" (fresh instance, same data dir) ---
        // In-process reopen shares redb file paths with the old instance, so
        // shut it down cleanly first (a real app restart is a new process;
        // the transport has no Drop-time flush).
        creator.shutdown().await?;
        drop(creator);
        let creator2 = SyncIrohTransport::spawn(Some(&dir_creator), NetMode::Loopback).await?;
        assert_eq!(creator2.doc_namespace_id().await?, ns);
        assert!(
            known_peer_ids(&creator2).contains(&joiner_id),
            "peer list must be repopulated from disk without a new ticket"
        );

        // Joiner writes something new; the RESTARTED creator dials out using
        // the observed+persisted peer record and converges (no re-share!).
        joiner
            .push_local(&[fake_event(4, "dev-J")], &[])
            .await?;
        let mut got = 0usize;
        for _ in 0..15 {
            creator2.sync_once().await?;
            let (reviews, _) = creator2.pull_remote().await?;
            got = reviews.len();
            if got == 5 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        assert_eq!(got, 5, "restarted creator must converge to 2 local + 3 joiner reviews");
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
