# Sync Wire Format v1

This file is the canonical specification of how sync batches leave the device,
travel, and are reassembled. It is shared with the Android client later — write
implementations against these sections, not against any one codebase.

## 2. Nostr spool encoding

### 2.0 Preamble — relay probes and production target

**Probe (a): `wss://jumble.virginiafreedom.tech`** — real WebSocket upgrade
attempt, explored 2026-10-17. Expected failure; confirmed. The host is a static
Jumble PWA: it answers the WS upgrade with a plain 200 HTML page.

Verbatim HTTP evidence (curl with full WS upgrade + `Accept: application/nostr+json`):

```
=== jumble.virginiafreedom.tech ===
HTTP/2 200 
accept-ranges: bytes
alt-svc: h3=":443"; ma=2592000
content-type: text/html
date: Sun, 18 Oct 2026 02:37:21 GMT
etag: "6a2052ba-714"
last-modified: Wed, 03 Jun 2026 16:13:46 GMT
server: nginx
via: 1.1 Caddy
content-length: 1812

<!doctype html>
<html lang="en">
```

Verbatim nostr-sdk probe (`Client::add_relay` + `connect`, 5s cap):

```
=== PROBE wss://jumble.virginiafreedom.tech ===
probe wss://jumble.virginiafreedom.tech: connected status=Disconnected in 4.001313763s
probe wss://jumble.virginiafreedom.tech: fetch ok, 0 event(s)
```

**Probe (b): `wss://git.virginiafreedom.tech`** — same procedure; also not a
relay. It's a Gitea instance (303 redirect to `/user/login`).

```
=== git.virginiafreedom.tech ===
HTTP/2 303 
location: /user/login
set-cookie: gitea_session=...; set-cookie: _csrf=...
server: nginx; via: 1.1 Caddy
```

```
=== PROBE wss://git.virginiafreedom.tech ===
probe wss://git.virginiafreedom.tech: connected status=Disconnected in 4.001600183s
probe wss://git.virginiafreedom.tech: fetch ok, 0 event(s)
```

(Note on the nostr-sdk lines: `fetch ok, 0 event(s)` with `status=Disconnected`
means the REQ was never answered — nostr-sdk returns an empty set on
timeout, so the `Disconnected` status is the dispositive signal.)

**Probe (c): production target.** Sync traffic does NOT go to public relays in
production. The target is a **private strfry relay colocated on the jumble
box**, write-whitelisted to the owner's single pubkey. Nothing has been
configured on the server; runbook only:

```
# strfry runbook (10 lines) — jumble.virginiafreedom.tech
1.  curl -L -o strfry https://github.com/hoytech/strfry/releases/latest/download/strfry-amd64  # or build from source
2.  install -m755 strfry /usr/local/bin/strfry
3.  useradd -r -m -d /var/lib/strfry strfry
4.  cp strfry.conf /etc/strfry.conf; edit:
5.    relay { bind = "127.0.0.1"; port = 7777; nofiles = 1000000;
6.            info { name = "greek-sync"; }
7.            writePolicy { plugin = ""; } }   # then restrict via allowlist:
8.    events { rejectEventsNewerThanSeconds = 900; maxEventSize = 131072; }
    # plus a simple strfry write policy plugin (or firewall-layer allowlist)
    # that ACCEPTs only events whose pubkey == <owner npub hex>, else "blocked: not whitelisted".
9.  nginx on jumble host: location /relay { proxy_pass http://127.0.0.1:7777;
    proxy_http_version 1.1; proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade"; }   # reuses existing TLS cert/vhost
10. systemd unit strfry.service: ExecStart=/usr/local/bin/strfry relay,
    User=strfry, Restart=always, WantedBy=multi-user.target → systemctl enable --now strfry
```

Result: `wss://jumble.virginiafreedom.tech/relay` becomes the production sync
endpoint; write-whitelist means stray/nefarious kind-1078 blobs can't be
stored, and all content is NIP-44 ciphertext anyway (§2.2).

### 2.1 SyncBatch JSON schema (VERBATIM shared shape)

```rust
pub struct ReviewEvent { pub uuid: String, pub card_id: String, pub timestamp: String, pub rating: u32, pub elapsed_days: f64, pub scheduled_days: f64, pub source: String, pub device_id: String, pub hlc: String }
pub struct SyncBatch { pub v: u32 /*1*/, pub device_id: String, pub from_hlc: String, pub to_hlc: String, pub reviews: Vec<ReviewEvent>, pub settings: Vec<(String,String,String)> /*key,value,hlc*/ }
```

Serialized with `serde_json` (UTF-8, field order as declared, floats via
shortest-round-trip). `settings` is a JSON array of 3-element arrays
(`["key","value","hlc"]`) because serde serializes Rust tuples as arrays.

### 2.2 Nostr envelope

Identity: one human = one nostr keypair (secp256k1). Relays are untrusted
storage. Everything is NIP-44 v2 encrypted **to self** (conversation key
derived with our own pubkey, `nip44::v2::ConversationKey::derive(sk, own_pk)`).

**Publish pipeline (one batch):**

1. `batch_json = serde_json::to_vec(&SyncBatch)` — v1 is uncompressed
   (ciphertext size ≈ plaintext size; reviews are tiny).
2. Chunk `batch_json` at **≤ 48 KiB (49152 bytes)** plaintext per chunk
   (chunk on byte boundaries; reassembly is byte concatenation).
3. Per chunk index `i` of count `n`: NIP-44-v2 encrypt the chunk to self
   (each event carries an independent ciphertext of a plaintext chunk).
4. Emit one nostr event per chunk:
   - `kind = 1078` (regular kind, NIP-01 range 1000-9999 → relay-retained)
   - tags: `["t","greek-sync"]`, `["d","<device_id>"]`, `["b","<batch uuid v4>"]`,
     `["i","<idx>"]`, `["n","<count>"]`
   - `content` = base64 NIP-44 ciphertext
   - signed with the identity key; **require relay `OK` ack per event**
     (send timeout 10s). Multi-relay: at least one `OK` suffices to proceed,
     but the client publishes to all connected relays.)
5. Then publish a **checkpoint** event:
   - `kind = 30078` (NIP-78 parameterized-replaceable)
   - tags: `["t","greek-sync"]`,
     `["d", hex(HMAC-SHA256(nip44_conversation_key, "greek-sync/checkpoint"))]`
     — **BLINDED**: relays never see semantic labels; the d-tag is a
     deterministic mac of the conversation key, so only the owner can locate
     or replace their checkpoint.
   - `content` = NIP-44(self) of JSON `{"device_id","last_to_hlc","batch"}`
     (`batch` = the batch uuid from step 4).

**Fetch pipeline `fetch_since(since_hlc)`:**

1. REQ filter `{authors: [self_pubkey], kinds: [1078, 30078],
   "#t": ["greek-sync"]}`, collect until EOSE (timeout 8s).
2. nostr-sdk verifies event signatures.
3. Keep kind-1078 events; regroup by `b` tag; within a group sort by `i`;
   require every index `0..n` present (else count as partial batch, skip).
4. Decrypt each event's content (NIP-44 v2 to self). Events that fail
   decrypt or JSON parse are **dropped and counted**
   (`SyncNostrError::DecryptFailed{count}` if nothing usable remains).
5. Byte-concat the decrypted chunks, `serde_json::from_slice` → `SyncBatch`.
6. Filter to batches with `to_hlc > since_hlc` (string compare, HLCs are
   UUID-padded ISO-8601 so lexicographic == causal order), return sorted by
   `to_hlc` ascending. The kind-30078 checkpoint is fetched with the same
   REQ and used by higher layers; decode via the blinded d-tag.

### 2.3 Relay policy notes

- One `OK` ack per event is required before push returns; requests/specs
  SHOULD point at the private strfry relay (write-whitelisted, generous
  limits). Public relays are degraded mode.
- Public relays rate-limit fresh keys and bursts. Exponential/linear backoff
  and chunk pacing are REQUIRED on public relays (see evidence below).
- ~65 KiB base64 ciphertext per event (48 KiB plaintext chunk) was accepted
  by relay.damus.io; do not raise `CHUNK_SIZE` — some relays cap event
  content at 64 KiB.
- Never log plaintext; the only cleartext visible to relays is: pubkey,
  event count/size/timing, `device_id` in the `d` tag of chunk events, and
  the `greek-sync` topic tag. (Metadata leak accepted in v1; topic tag could
  be blinded in v2.)

### 2.4 Live-relay evidence (`#[ignore]` test `live_damus_roundtrip`, run 2026-10-18)

Fresh random `Keys`; relays: damus (+ nos.lol/offchain.pub fallbacks).

```
pubkey: cfc1985f02b7f88c18e5f4f5a25f2db17e1b2f51b56f0de15cfa0f2edd46780f
pushed small batch (1390 bytes json) in 1.770850798s
push big batch: Publish("no relay accepted event 60ac061e...: {RelayUrl(\"wss://relay.damus.io\"): \"rate-limited: you are noting too much\"}")
```

Bursting ~140 chunk events from a fresh key trips damus's rate limiter.
With 500ms pacing between chunk publishes (backoff 10s on `rate-limited`):

```
pubkey: 8f42227df9abe0bcbdcfa2b54be34c4924ba19be9b1c0de9527c69200a3fb35a
pushed small batch (1390 bytes json) in 1.110524056s
pushed big batch (366509 bytes json, 8 chunks, 8 accepted) in (~2.5 min with pacing/backoff)
fetched 3 batch(es) in 824.48011ms
fetch_since filter OK: 0 batch(es) after
test result: ok. 1 passed; 0 failed; 0 ignored
```

Observed: damus accepts kind 1078 and kind 30078 from fresh keys, retains
them (fetch-after-minutes returns all batches), per-event OK acks work,
fetch to EOSE < 1s for ~145 authored events. Rate limits are the binding
constraint on public relays; production must use the private strfry relay.

### 2.5 Implementation note (interpretive deviation)

The design text reads "chunk plaintext ... concat → NIP-44 decrypt".
Implemented as: **encrypt each chunk independently** (per-event
ciphertext), and on fetch **decrypt each event, then byte-concat the
plaintext**. This is the only reading compatible with "each chunk one
nostr event" plus drop-and-count semantics for individual corrupt events,
and it is what `crates/sync-nostr` does. Unit tests in the crate cover
the exact 48 KiB boundary and reassembly equality on a >200 KiB batch.
## 3. iroh-docs encoding

Validated by `spikes/iroh/` (see `spikes/iroh/SPIKE-IROH.md`). Locked versions:
`iroh 1.3.0`, `iroh-docs 0.101.0`, `iroh-blobs 0.103.1`, `iroh-gossip 0.101.0`,
rustls **ring** crypto provider.

### 3.1 Doc / namespace mapping

- One iroh-docs document == one user's notebook (the whole sync scope).
  Namespace public key (`NamespaceId`) == doc id. Multi-device = multi-author
  doc.
- Each device holds its own **author** keypair (created with
  `DocsApi::author_create`), persisted next to the redb store via
  `DefaultAuthorStorage`. A given author writes only from its device, so
  per-key latest-write-wins inside docs is safe for append-only review events
  (each event has a unique key; rewrites never race across devices).
- Sharing uses `ShareMode::Write` tickets: the ticket's `capability` embeds
  the **full namespace secret key**, which is required for multi-writer.
  (`ShareMode::Read` embeds only the namespace public key + a delegate read
  capability.) Both variants are string-encodable via `DocTicket`'s `Ticket`
  impl (kind `doc`, postcard wire format, base32-like; observed length
  **277 chars** with 1 direct address — read-only tickets are similar).

### 3.2 Entry schema

Key (bytes, UTF-8):

```
reviews/<hlc-millis:013>-<device_id>-<uuid4-first-8-hex>
```

- The zero-padded HLC millisecond prefix makes lexicographic key order equal
  chronological order; `get_many(Query::all())` returns entries in key order,
  so range scans over a time window are free.
- `reviews/` prefix reserves the doc for future entry namespaces
  (`cards/`, `decks/`, …).

Content (bytes): one JSON review event, ~300 bytes:

```json
{
  "uuid": "…",
  "card_id": "athenaze-i-0001",
  "timestamp": "2026-10-17T10:00:00Z",
  "rating": 3,
  "elapsed_days": 2.0,
  "scheduled_days": 2.0,
  "source": "flashcard",
  "device_id": "…",
  "hlc": "<hlc-millis>:<counter>:<device_id>"
}
```

Insertion is `Doc::set_bytes(author, key, content)`; content rides
iroh-blobs hashes inside docs entries.

### 3.3 Ticket / QR exchange

QR payload URL scheme:

```
greeksync://ticket/<ticket-string>
```

where `<ticket-string>` is `DocTicket::to_string()` (kind prefix `doc…`,
postcard-encoded `{ capability, nodes }`). Build with
`doc.share(ShareMode::Write, AddrInfoOptions::Addresses)` so the ticket
carries the scanning device's direct addresses (LAN) — the write-capable
namespace secret travels inside the ticket, so **the QR code is a credential**
and must never go over a network. Both dial ends upside down: importer calls
`docs.import(ticket)` then `doc.start_sync(vec![sharer_addr])`; convergence is
observed via `doc.subscribe()` until `SyncFinished` + entry count stable.

### 3.4 Android lifecycle checklist (per official iroh docs guidance)

- **Rust shim, not iroh-ffi**: the published `computer.iroh:iroh` Kotlin
  artifacts do not cover Android; we embed a Rust shim cdylib next to
  `uniffi-bindings` (validated in Part C — aarch64 + x86_64 build clean with
  NDK 27 clang-26, ring provider, no cmake/aws-lc-rs needed).
- **Persist the endpoint secret key** across process restarts or the device's
  node id (and peers' tickets pointing at it) changes every boot.
- **Persist docs via `Docs::persistent(<filesDir>)`** (redb store + author
  store under the same dir) — reopen keeps entries + authors (validated).
- **Shutdown on background**: call `Router::shutdown()` when the app
  backgrounds; Android kills sockets / the QUIC sockets will silently die
  otherwise. Re-bind a fresh `Endpoint` + Router on foreground.
- Optional: run sync under a **foreground service** for long transfers so the
  OS doesn't throttle networking mid-sync.
- **Crypto provider**: install `rustls::crypto::ring::default_provider()`
  explicitly (`Endpoint::builder(...).crypto_provider(..)` required in
  iroh 1.3 when using `presets::Empty`/custom presets).

### 3.5 Constraints

- **Simultaneous-online**: iroh sync is set-reconciliation between live peers
  (relay-assisted NAT traversal via n0 relay is possible but both devices must
  be online together). Mitigation: the nostr spool (§2) acts as an async
  backstop for devices that never overlap.
