use anyhow::{Context, Result};
use iroh::endpoint::presets;
use iroh::protocol::Router;
use iroh::Endpoint;
use iroh_blobs::BlobsProtocol;
use iroh_blobs::store::mem::MemStore;
use iroh_docs::api::protocol::{AddrInfoOptions, ShareMode};
use iroh_docs::engine::LiveEvent;
use iroh_docs::protocol::Docs;
use iroh_docs::store::Query;
use iroh_docs::{DocTicket, NamespaceId};
use iroh_gossip::net::Gossip;
use n0_future::StreamExt;
use std::path::PathBuf;
use std::time::Instant;

mod data;
use data::{entry_key, review_event_json};

pub struct Node {
    pub router: Router,
    pub endpoint: Endpoint,
    pub docs: Docs,
    _blobs: MemStore,
    _gossip: Gossip,
}

pub async fn spawn_node(persist_dir: Option<PathBuf>) -> Result<Node> {
    // Empty preset: no relay, no DNS discovery — loopback/local only.
    let endpoint = Endpoint::builder(presets::Empty)
        .crypto_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
        .bind()
        .await?;
    let gossip = Gossip::builder().spawn(endpoint.clone());
    let mem = MemStore::new();
    let store: iroh_blobs::api::Store = mem.clone().into();
    let docs = match persist_dir {
        Some(dir) => {
            std::fs::create_dir_all(&dir)?;
            Docs::persistent(dir)
                .spawn(endpoint.clone(), store.clone(), gossip.clone())
                .await?
        }
        None => {
            Docs::memory()
                .spawn(endpoint.clone(), store.clone(), gossip.clone())
                .await?
        }
    };
    let router = Router::builder(endpoint.clone())
        .accept(iroh_blobs::ALPN, BlobsProtocol::new(&store, None))
        .accept(iroh_gossip::net::GOSSIP_ALPN, gossip.clone())
        .accept(iroh_docs::ALPN, docs.clone())
        .spawn();
    Ok(Node {
        router,
        endpoint,
        docs,
        _blobs: mem,
        _gossip: gossip,
    })
}

async fn count_entries(doc: &iroh_docs::api::Doc) -> Result<usize> {
    let stream = doc.get_many(Query::all()).await?;
    tokio::pin!(stream);
    let mut n: usize = 0;
    let mut last_key: Vec<u8> = Vec::new();
    let mut sorted = true;
    while let Some(entry) = stream.next().await {
        let entry = entry?;
        if !entry.key().starts_with(b"reviews/") {
            continue;
        }
        if n > 0 && entry.key() < last_key.as_slice() {
            sorted = false;
            eprintln!(
                "UNSORTED: prev={:?} cur={:?}",
                String::from_utf8_lossy(&last_key),
                String::from_utf8_lossy(entry.key())
            );
        }
        last_key = entry.key().to_vec();
        n += 1;
    }
    if !sorted {
        println!("NOTE: get_many did not return strictly sorted keys");
    }
    Ok(n)
}

/// Wait until `doc` has `want` entries matching `reviews/` prefix.
async fn wait_until_converged(doc: &iroh_docs::api::Doc, want: usize) -> Result<()> {
    let mut events = doc.subscribe().await?;
    loop {
        let n = count_entries(doc).await?;
        if n >= want {
            // drain any pending event check: ensure count stable after next event
            return Ok(());
        }
        match tokio::time::timeout(std::time::Duration::from_secs(30), events.next()).await {
            Ok(Some(Ok(ev))) => {
                if let LiveEvent::SyncFinished(_) = ev {
                    let n = count_entries(doc).await?;
                    if n >= want {
                        return Ok(());
                    }
                }
            }
            Ok(Some(Err(e))) => anyhow::bail!("live event error: {e}"),
            Ok(None) => anyhow::bail!("event stream ended"),
            Err(_) => anyhow::bail!("timeout waiting for convergence (have {n}, want {want})"),
        }
    }
}

pub async fn part_x_writes_y_syncs(persist: Option<(PathBuf, PathBuf)>) -> Result<f64> {
    let (dx, dy) = match &persist {
        Some((a, b)) => (Some(a.clone()), Some(b.clone())),
        None => (None, None),
    };
    let x = spawn_node(dx).await?;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let y = spawn_node(dy).await?;

    // X: create doc + author, insert 50 entries.
    let doc_x = x.docs.create().await?;
    let author_x = x.docs.author_create().await?;
    for i in 0..50u32 {
        let key = entry_key(i, "dev-X");
        let content = review_event_json(i, "dev-X");
        doc_x
            .set_bytes(author_x, key.into_bytes(), content.into_bytes())
            .await?;
    }

    // Ticket with write-capable namespace (ShareMode::Write).
    let ticket: DocTicket = doc_x
        .share(ShareMode::Write, AddrInfoOptions::Addresses)
        .await?;
    let ticket_str = ticket.to_string();
    println!("PART A: ticket len={} chars prefix={}", ticket_str.len(), &ticket_str[..12]);

    // Round-trip the ticket string (this is what travels the QR code).
    let ticket2: DocTicket = ticket_str.parse()?;
    let t0 = Instant::now();
    let doc_y = y.docs.import(ticket2.clone()).await?;
    let x_addr = x.endpoint.addr();
    doc_y.start_sync(vec![x_addr.clone()]).await?;
    wait_until_converged(&doc_y, 50).await?;
    let initial_sync_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let n = count_entries(&doc_y).await?;
    anyhow::ensure!(n == 50, "Y converged to {n} entries, want 50");
    println!(
        "PART A: initial sync of 50 entries OK in {:.1} ms",
        initial_sync_ms
    );

    // Y writes 10 entries as its own author; X syncs; X should see 60.
    let author_y = y.docs.author_create().await?;
    for i in 50..60u32 {
        let key = entry_key(i, "dev-Y");
        let content = review_event_json(i, "dev-Y");
        doc_y
            .set_bytes(author_y, key.into_bytes(), content.into_bytes())
            .await?;
    }
    doc_x.start_sync(vec![y.endpoint.addr()]).await?;
    wait_until_converged(&doc_x, 60).await?;
    let n = count_entries(&doc_x).await?;
    anyhow::ensure!(n == 60, "X converged to {n} entries, want 60");
    println!("PART A: reverse sync OK, X sees 60 entries");

    // record namespace id for reuse across persistence reopen
    let ns = doc_x.id();
    println!("PART A: namespace id = {}", ns);

    x.router.shutdown().await.ok();
    y.router.shutdown().await.ok();
    Ok(initial_sync_ms)
}

pub async fn reopen_and_verify(dir: PathBuf) -> Result<usize> {
    let x = spawn_node(Some(dir)).await?;
    let mut ns: Option<NamespaceId> = None;
    let mut listed = 0usize;
    let stream = x.docs.list().await?;
    tokio::pin!(stream);
    while let Some(item) = stream.next().await {
        let (doc_id, _cap) = item?;
        ns = Some(doc_id);
        listed += 1;
    }
    let ns = ns.context("no doc found after reopen")?;
    let doc = x.docs.open(ns).await?.context("open failed")?;
    let n = count_entries(&doc).await?;
    println!("PART B: after reopen, listed {listed} doc(s), {n} entries intact");
    x.router.shutdown().await.ok();
    Ok(n)
}

pub async fn run() -> Result<()> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "all".to_string());
    match mode.as_str() {
        "a" => {
            part_x_writes_y_syncs(None).await?;
        }
        "b" => {
            let base = std::env::temp_dir().join(format!("spike-iroh-persist-{}", std::process::id()));
            let dx = base.join("x");
            let dy = base.join("y");
            let _ = std::fs::remove_dir_all(&base);
            part_x_writes_y_syncs(Some((dx.clone(), dy))).await?;
            drop(base);
            let n = reopen_and_verify(dx).await?;
            anyhow::ensure!(n == 60, "reopen count {n}");
        }
        _ => {
            part_x_writes_y_syncs(None).await?;
            let base = std::env::temp_dir().join(format!("spike-iroh-persist-{}", std::process::id()));
            let dx = base.join("x");
            let dy = base.join("y");
            let _ = std::fs::remove_dir_all(&base);
            part_x_writes_y_syncs(Some((dx.clone(), dy))).await?;
            let n = reopen_and_verify(dx).await?;
            anyhow::ensure!(n == 60, "reopen count {n}");
        }
    }
    println!("ALL SPIKE PARTS PASSED");
    Ok(())
}
