//! sync-peer — host-side cross-device sync E2E harness.
//!
//! Protocol: **greek-sync-e2e v1** (FROZEN — byte-compatible with the
//! Android instrumented test and the orchestrator script). One invocation
//! plays one device of a two-device sync test; the other side may be another
//! sync-peer process (host↔host selfcheck) or the on-device instrumented
//! test, bridged by an orchestrator into a shared evidence dir `<DIR>`.
//!
//! Evidence files in <DIR>:
//!   ticket.txt      A(serve) writes, B(join) reads   — `greeksync://ticket/…` (one line)  [iroh]
//!   nsec_in.hex     orchestrator writes, BOTH read   — shared nostr identity hex          [nostr]
//!   seeds_a.json    A writes, B reads                — `{"uuids":[5],"hlc":"..."}`
//!   push_done.flag  A writes, B reads                — A's nostr push landed              [nostr]
//!   b_pushed.flag   B writes, A reads                — B's nostr push landed              [nostr]
//!   pull_done.flag  A writes                         — A converged
//!   export_a.json / export_b.json                    — full v1 SyncBatch string per side
//!   stats_a.json                                     — `{"receivedNonlocal":N,"exportSize":N}`
//!
//! Seeds:  A = reviews on cards "d1".."d5" ratings [3,4,2,3,1] plus
//!         record_alphabet_check_result(23, 24) (gate latch).
//!         B = reviews on cards "h1".."h3" ratings [4,3,2].
//!         Total event expectation after convergence: 8.
//!
//! Exit codes: 0 converge/ok · 1 failed assert · 2 timeout · 3 usage error.
//! Progress lines go to stdout prefixed `[A]`/`[B]` + elapsed ms;
//! errors/diagnostics go to stderr.
//!
//! iroh handshake semantics mirror tests/e2e_two_device_sync.rs: A creates
//! the ticket and pushes into the doc BEFORE B exists (harmless — the doc
//! replicates an empty peer set); B joins from the ticket, so B dials A and
//! live replication flows both ways from then on.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use uniffi_bindings::GreekCore;

/// The real letters track — same fixture as tests/e2e_two_device_sync.rs.
/// Without a letters deck the alphabet gate fails open (satisfied with an
/// empty method) and B's gate-latch convergence check would be meaningless.
const LETTERS_JSON: &str = include_str!("../../../data/cards/letters.json");

/// Hard cap on any single transport op so a wedged relay/endpoint can never
/// hang the harness (mirrors IROH_OP_TIMEOUT in the two-device e2e test).
const SYNC_OP_TIMEOUT: Duration = Duration::from_secs(60);

const EXIT_OK: i32 = 0;
const EXIT_ASSERT: i32 = 1;
const EXIT_TIMEOUT: i32 = 2;
const EXIT_USAGE: i32 = 3;

const USAGE: &str = "\
sync-peer — Greek sync E2E harness (protocol: greek-sync-e2e v1)

USAGE:
  sync-peer serve --dir <DIR> --db <DBPATH> [--nostr --relay ws://...]
      Play device side (\"A\"): seed d1..d5 + alphabet latch, share the doc /
      push to the nostr spool, converge on B's h1..h3, write export_a.json,
      stats_a.json and pull_done.flag (iroh also writes ticket.txt first;
      nostr writes push_done.flag and waits for b_pushed.flag).
  sync-peer join --dir <DIR> --db <DBPATH> [--nostr --relay ws://...]
      Play host side (\"B\"): wait for ticket.txt (iroh) or push_done.flag
      (nostr; both sides share nsec_in.hex — one keypair, spool-to-self),
      converge on A's uuids, seed h1..h3, push, write export_b.json
      (nostr also writes b_pushed.flag).
  sync-peer dump --db <DBPATH>
      Print {\"uuids\":[...]} from sync_export_backup_json on stdout.
  sync-peer ticket --db <DBPATH>
      Print a fresh greeksync://ticket/... line on stdout and exit (diag).

OPTIONS: --nostr selects the nostr-spool protocol; --relay may repeat and is
comma-joined into sync_set_nostr_relays (absent = library default relay).

EXIT CODES: 0 converge/ok · 1 failed assert · 2 timeout · 3 usage error";

/// Stdout progress clock: `[A]  1234ms message`.
struct Clock {
    t0: Instant,
    role: &'static str,
}

impl Clock {
    fn new(role: &'static str) -> Self {
        Self {
            t0: Instant::now(),
            role,
        }
    }
    fn log(&self, msg: impl std::fmt::Display) {
        println!("[{}] {:>7}ms {}", self.role, self.t0.elapsed().as_millis(), msg);
        use std::io::Write as _;
        let _ = std::io::stdout().flush();
    }
    /// Diagnostics go to stderr so the stdout protocol stream stays greppable
    /// (and dump/ticket stdout remains machine-parseable).
    fn err(&self, msg: impl std::fmt::Display) {
        eprintln!("[{}] {:>7}ms ERROR {}", self.role, self.t0.elapsed().as_millis(), msg);
    }
}

fn die(code: i32, clk: &Clock, msg: impl std::fmt::Display) -> ! {
    clk.err(msg);
    std::process::exit(code);
}

#[derive(PartialEq)]
enum Cmd {
    Serve,
    Inproc,
    Join,
    Dump,
    Ticket,
}

struct Cli {
    cmd: Cmd,
    dir: Option<PathBuf>,
    db: String,
    nostr: bool,
    relays: Vec<String>,
}

fn parse_args(argv: &[String]) -> Result<Cli, String> {
    let cmd = match argv.first().map(String::as_str) {
        Some("serve") => Cmd::Serve,
        Some("inproc") => Cmd::Inproc,
        Some("join") => Cmd::Join,
        Some("dump") => Cmd::Dump,
        Some("ticket") => Cmd::Ticket,
        Some("-h") | Some("--help") => {
            println!("{USAGE}");
            std::process::exit(EXIT_OK);
        }
        Some(other) => return Err(format!("unknown command {other:?}")),
        None => return Err(String::new()),
    };
    let mut dir = None;
    let mut db = None;
    let mut nostr = false;
    let mut relays = Vec::new();
    let mut it = argv[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dir" => dir = Some(PathBuf::from(it.next().ok_or("--dir needs a value")?)),
            "--db" => db = Some(it.next().ok_or("--db needs a value")?.to_string()),
            "--nostr" => nostr = true,
            "--relay" => relays.push(it.next().ok_or("--relay needs a value")?.to_string()),
            "-h" | "--help" => return Err(String::new()), // usage error prints below
            other => return Err(format!("unknown flag {other:?}")),
        }
    }
    let db = db.ok_or("--db is required")?;
    if matches!(cmd, Cmd::Serve | Cmd::Join | Cmd::Inproc) && dir.is_none() {
        return Err("serve/join/inproc need --dir <DIR>".to_string());
    }
    if matches!(cmd, Cmd::Inproc) && nostr {
        return Err("inproc is iroh-only (nostr selfcheck uses serve/join)".to_string());
    }
    Ok(Cli {
        cmd,
        dir,
        db,
        nostr,
        relays,
    })
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    // Diag opt-in: RUST_LOG=debug ./sync-peer ... shows iroh/gossip interna.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let cli = match parse_args(&argv) {
        Ok(c) => c,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("usage error: {e}\n");
            }
            eprintln!("{USAGE}");
            std::process::exit(EXIT_USAGE);
        }
    };
    match cli.cmd {
        Cmd::Serve => {
            let clk = Clock::new("A");
            if cli.nostr {
                serve_nostr(&cli, &clk).await;
            } else {
                serve_iroh(&cli, &clk).await;
            }
        }
        Cmd::Inproc => inproc_iroh(&cli).await,
        Cmd::Join => {
            let clk = Clock::new("B");
            if cli.nostr {
                join_nostr(&cli, &clk).await;
            } else {
                join_iroh(&cli, &clk).await;
            }
        }
        Cmd::Dump => cmd_dump(&cli.db),
        Cmd::Ticket => cmd_ticket(&cli.db).await,
    }
    std::process::exit(EXIT_OK);
}

// --- shared helpers ---------------------------------------------------------

fn open_core(db: &str, clk: &Clock) -> Arc<GreekCore> {
    if let Some(parent) = Path::new(db).parent().filter(|p| !p.as_os_str().is_empty()) {
        if let Err(e) = std::fs::create_dir_all(parent) {
            die(
                EXIT_ASSERT,
                clk,
                format!("cannot create db dir {}: {e}", parent.display()),
            );
        }
    }
    GreekCore::new_with_cards(db.to_string(), None, LETTERS_JSON.to_string())
        .unwrap_or_else(|e| die(EXIT_ASSERT, clk, format!("GreekCore::new_with_cards({db}): {e}")))
}

/// Parsed view of `sync_export_backup_json` output.
struct Export {
    /// The raw full SyncBatch string (written verbatim to export_{a,b}.json).
    json: String,
    /// Review uuids in export order, deduped.
    uuids: Vec<String>,
    set: HashSet<String>,
    to_hlc: String,
}

fn export(core: &GreekCore, clk: &Clock) -> Export {
    let json = core.sync_export_backup_json();
    let v: serde_json::Value = serde_json::from_str(&json)
        .unwrap_or_else(|e| die(EXIT_ASSERT, clk, format!("export not valid JSON: {e}\n{json}")));
    if let Some(err) = v.get("error") {
        die(EXIT_ASSERT, clk, format!("export returned error payload: {err}"));
    }
    let mut uuids: Vec<String> = Vec::new();
    let mut set: HashSet<String> = HashSet::new();
    for r in v["reviews"].as_array().cloned().unwrap_or_default() {
        if let Some(u) = r["uuid"].as_str() {
            if set.insert(u.to_string()) {
                uuids.push(u.to_string());
            }
        }
    }
    let to_hlc = v["to_hlc"].as_str().unwrap_or_default().to_string();
    Export {
        json,
        uuids,
        set,
        to_hlc,
    }
}

/// Write a protocol file atomically (tmp + rename) so a reader polling on
/// file existence never observes a partial write.
fn write_proto(dir: &Path, name: &str, contents: &str, clk: &Clock) {
    let tmp = dir.join(format!(".{name}.tmp"));
    let dst = dir.join(name);
    if let Err(e) = std::fs::write(&tmp, contents).and_then(|_| std::fs::rename(&tmp, &dst)) {
        die(EXIT_ASSERT, clk, format!("write {}: {e}", dst.display()));
    }
    clk.log(format!("wrote {}", dst.display()));
}

/// Poll for a protocol file until it exists and is non-empty.
/// Exits 2 on deadline.
async fn poll_file(path: &Path, deadline: Duration, what: &str, clk: &Clock) -> String {
    let start = Instant::now();
    loop {
        if let Ok(s) = std::fs::read_to_string(path) {
            if !s.trim().is_empty() {
                clk.log(format!("{what} appeared after {}ms", start.elapsed().as_millis()));
                return s;
            }
        }
        if start.elapsed() >= deadline {
            die(
                EXIT_TIMEOUT,
                clk,
                format!("timeout ({deadline:?}) waiting for {what} at {}", path.display()),
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Read the `uuids` list out of seeds_a.json. Returns None while absent/unparsable.
fn read_seeds(content: &str) -> Option<HashSet<String>> {
    let v: serde_json::Value = serde_json::from_str(content).ok()?;
    let arr = v["uuids"].as_array()?;
    let set: HashSet<String> = arr
        .iter()
        .filter_map(|u| u.as_str().map(str::to_string))
        .collect();
    if set.is_empty() {
        None
    } else {
        Some(set)
    }
}

/// Fatal transport op: hard-fail on error or 60s op timeout (used where the
/// protocol demands exactly one op succeed: ticket, join, mandatory push).
async fn op<T, F>(clk: &Clock, what: &str, fut: F) -> T
where
    F: std::future::Future<Output = Result<T, uniffi_bindings::GreekError>>,
{
    match tokio::time::timeout(SYNC_OP_TIMEOUT, fut).await {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => die(EXIT_ASSERT, clk, format!("{what} failed: {e}")),
        Err(_) => die(EXIT_TIMEOUT, clk, format!("{what} exceeded {SYNC_OP_TIMEOUT:?}")),
    }
}

/// Retryable transport op inside a convergence loop: logs to stderr and
/// returns None so the outer deadline loop decides when to give up.
async fn try_op<T, F>(clk: &Clock, what: &str, fut: F) -> Option<T>
where
    F: std::future::Future<Output = Result<T, uniffi_bindings::GreekError>>,
{
    match tokio::time::timeout(SYNC_OP_TIMEOUT, fut).await {
        Ok(Ok(v)) => Some(v),
        Ok(Err(e)) => {
            clk.err(format!("{what} failed (will retry): {e}"));
            None
        }
        Err(_) => {
            clk.err(format!("{what} exceeded {SYNC_OP_TIMEOUT:?} (will retry)"));
            None
        }
    }
}

fn seed_a(core: &GreekCore, clk: &Clock) {
    for (card, rating) in [("d1", 3u8), ("d2", 4), ("d3", 2), ("d4", 3), ("d5", 1)] {
        if !core.submit_review(card.to_string(), rating) {
            die(EXIT_ASSERT, clk, format!("submit_review({card},{rating}) returned false"));
        }
    }
    if !core.record_alphabet_check_result(23, 24) {
        die(
            EXIT_ASSERT,
            clk,
            "record_alphabet_check_result(23, 24) did not satisfy the gate",
        );
    }
    clk.log("seeded d1..d5 with ratings [3,4,2,3,1]; alphabet gate latched (check_passed)");
}

fn seed_b(core: &GreekCore, clk: &Clock) {
    for (card, rating) in [("h1", 4u8), ("h2", 3), ("h3", 2)] {
        if !core.submit_review(card.to_string(), rating) {
            die(EXIT_ASSERT, clk, format!("submit_review({card},{rating}) returned false"));
        }
    }
    clk.log("seeded h1..h3 with ratings [4,3,2]");
}

// --- nostr identity/relay setup (shared by serve_nostr and join_nostr) ------

async fn nostr_setup(core: &GreekCore, cli: &Cli, dir: &Path, clk: &Clock) {
    // Both sides use the SAME identity from nsec_in.hex — the wallet-of-one
    // design: one nostr keypair, spool-to-self via the relay.
    let nsec = poll_file(
        &dir.join("nsec_in.hex"),
        Duration::from_secs(90),
        "nsec_in.hex",
        clk,
    )
    .await;
    if let Err(e) = core.sync_set_nostr_nsec(nsec.trim().to_string()) {
        die(EXIT_ASSERT, clk, format!("sync_set_nostr_nsec rejected nsec_in.hex: {e}"));
    }
    if !cli.relays.is_empty() {
        core.sync_set_nostr_relays(cli.relays.join(","));
    }
    let cfg = core.sync_nostr_config();
    clk.log(format!(
        "nostr ready: identity set, relays={}",
        cfg.relays_csv
    ));
}

// --- iroh flows --------------------------------------------------------------

/// A flow (iroh): seed+latch -> ticket -> 1x sync_iroh_now -> publish
/// ticket.txt + seeds_a.json -> poll sync (5s, 150s deadline) until
/// >=3 non-seed uuids arrive -> assert |uuids|==8 -> terminal artifacts ->
/// 30s grace linger -> exit 0.
async fn serve_iroh(cli: &Cli, clk: &Clock) {
    let dir = cli.dir.clone().unwrap();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        die(EXIT_ASSERT, clk, format!("create {}: {e}", dir.display()));
    }
    let core = open_core(&cli.db, clk);
    clk.log(format!("device_id={}", core.sync_device_id()));

    seed_a(&core, clk);
    let seeds = export(&core, clk);
    if seeds.uuids.len() != 5 {
        die(EXIT_ASSERT, clk, format!("A seeded {} reviews, expected 5", seeds.uuids.len()));
    }
    let st = core.get_alphabet_gate_state();
    clk.log(format!("gate: satisfied={} method={}", st.satisfied, st.method));

    let ticket = op(clk, "sync_iroh_ticket", core.sync_iroh_ticket()).await;
    if !ticket.starts_with("greeksync://ticket/") {
        die(EXIT_ASSERT, clk, format!("ticket missing greeksync://ticket/ prefix: {ticket}"));
    }
    // Push BEFORE any peer exists (harmless — the doc accepts entries with an
    // empty peer set); B joins from the ticket and dials us afterwards.
    let st = op(clk, "sync_iroh_now (initial push)", core.sync_iroh_now()).await;
    clk.log(format!(
        "initial push: sent={} received={} settings={}",
        st.reviews_sent, st.reviews_received, st.settings_applied
    ));
    if st.reviews_sent != 5 {
        die(EXIT_ASSERT, clk, format!("initial push sent {}, expected 5", st.reviews_sent));
    }

    write_proto(&dir, "ticket.txt", &format!("{ticket}\n"), clk);
    write_proto(
        &dir,
        "seeds_a.json",
        &json!({ "uuids": seeds.uuids, "hlc": seeds.to_hlc }).to_string(),
        clk,
    );

    // Convergence: keep syncing until B's h1..h3 events appear in our doc.
    let deadline = Instant::now() + Duration::from_secs(150);
    let mut converged: Option<Export> = None;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(5)).await;
        match try_op(clk, "sync_iroh_now", core.sync_iroh_now()).await {
            Some(st) => clk.log(format!(
                "sync round: sent={} received={} settings={}",
                st.reviews_sent, st.reviews_received, st.settings_applied
            )),
            None => continue,
        }
        let ex = export(&core, clk);
        let received = ex.set.difference(&seeds.set).count();
        if received >= 3 {
            clk.log(format!("converged: received {received} non-local events (|uuids|={})", ex.uuids.len()));
            converged = Some(ex);
            break;
        }
    }
    let Some(ex) = converged else {
        die(EXIT_TIMEOUT, clk, "150s elapsed without receiving B's 3 events");
    };
    if ex.uuids.len() != 8 {
        die(EXIT_ASSERT, clk, format!("assert |uuids|==8 failed: got {}", ex.uuids.len()));
    }
    let received_nonlocal = ex.set.difference(&seeds.set).count() as u32;

    write_proto(&dir, "export_a.json", &ex.json, clk);
    write_proto(
        &dir,
        "stats_a.json",
        &json!({ "receivedNonlocal": received_nonlocal, "exportSize": ex.uuids.len() }).to_string(),
        clk,
    );
    write_proto(&dir, "pull_done.flag", "done\n", clk);

    // Grace linger: stay alive (iroh endpoint keeps answering dials) but no
    // more syncs needed — B is already done by protocol construction.
    clk.log("grace linger 30s");
    tokio::time::sleep(Duration::from_secs(30)).await;
    clk.log("A done (exit 0)");
}

/// Hermetic selfcheck: run A and B as two GreekCore instances in ONE
/// process. The transports are still two real iroh endpoints talking QUIC
/// over loopback sockets; only the process boundary is removed. This is the
/// selfcheck topology because cross-process join_startup currently hangs on
/// some hosts (tracked: scripts/e2e/README.md § Known limits) — the
/// cross-device topology is what `--device` mode and the pairing rehearsal
/// exist to prove.
async fn inproc_iroh(cli: &Cli) {
    let base = PathBuf::from(&cli.db);
    if let Err(e) = std::fs::create_dir_all(&base) {
        eprintln!("inproc: cannot create --db dir {}: {e}", base.display());
        std::process::exit(EXIT_ASSERT);
    }
    let dir = cli.dir.clone().unwrap();
    // Each core gets its OWN subdirectory: the iroh node store (and its
    // persisted endpoint secret key) lives at <db-parent>/iroh-sync, so
    // same-parent dbs would silently share ONE node identity and the
    // join would dial itself — a blocked 60s no-progress failure mode
    // that cost real debugging time. KEEP THESE SEPARATE.
    let cli_a = Cli {
        cmd: Cmd::Serve,
        dir: Some(dir.clone()),
        db: base.join("a/greek_app.db").to_string_lossy().into_owned(),
        nostr: false,
        relays: Vec::new(),
    };
    let cli_b = Cli {
        cmd: Cmd::Join,
        dir: Some(dir),
        db: base.join("b/greek_app.db").to_string_lossy().into_owned(),
        nostr: false,
        relays: Vec::new(),
    };
    let clk_a = Clock::new("A");
    let clk_b = Clock::new("B");
    // die() inside either role exits the process with the proper code; on
    // success both return and we exit 0 through main.
    let ta = tokio::spawn(async move { serve_iroh(&cli_a, &clk_a).await });
    let tb = tokio::spawn(async move { join_iroh(&cli_b, &clk_b).await });
    if let Err(e) = ta.await {
        eprintln!("inproc: A task panicked/joined badly: {e}");
        std::process::exit(EXIT_ASSERT);
    }
    if let Err(e) = tb.await {
        eprintln!("inproc: B task panicked/joined badly: {e}");
        std::process::exit(EXIT_ASSERT);
    }
}

/// B flow (iroh): poll ticket.txt (<=90s) -> join -> seed h1..h3 -> poll sync
/// (5s, 120s deadline) until export superset-of A's seeds AND gate latched ->
/// linger 75s syncing every 5s (so A can converge) -> export_b.json -> exit 0.
async fn join_iroh(cli: &Cli, clk: &Clock) {
    let dir = cli.dir.clone().unwrap();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        die(EXIT_ASSERT, clk, format!("create {}: {e}", dir.display()));
    }
    let core = open_core(&cli.db, clk);
    clk.log(format!("device_id={}", core.sync_device_id()));

    let ticket = poll_file(
        &dir.join("ticket.txt"),
        Duration::from_secs(90),
        "ticket.txt",
        clk,
    )
    .await;
    let ticket = ticket.trim().to_string();
    op(clk, "sync_iroh_join", core.sync_iroh_join(ticket)).await;
    clk.log("joined A's doc (B dials the ticket author)");

    // Seed BEFORE the convergence loop: our first sync round already pushes
    // h1..h3 while pulling A's history (mirrors the wave-7 two-instance test).
    seed_b(&core, clk);

    let deadline = Instant::now() + Duration::from_secs(120);
    let mut seeds_set: Option<HashSet<String>> = None;
    loop {
        if let Some(st) = try_op(clk, "sync_iroh_now", core.sync_iroh_now()).await {
            clk.log(format!(
                "sync round: sent={} received={} settings={}",
                st.reviews_sent, st.reviews_received, st.settings_applied
            ));
        }
        // In iroh mode B reads seeds_a.json (published by A alongside the ticket).
        if seeds_set.is_none() {
            if let Ok(s) = std::fs::read_to_string(dir.join("seeds_a.json")) {
                seeds_set = read_seeds(&s);
            }
        }
        let ex = export(&core, clk);
        let gate_ok = core.get_alphabet_gate_state().satisfied;
        let have_all = seeds_set
            .as_ref()
            .map(|ss| ss.iter().all(|u| ex.set.contains(u)))
            .unwrap_or(false);
        clk.log(format!(
            "progress: uuids={} seeds_superset={} gate={}",
            ex.uuids.len(),
            have_all,
            gate_ok
        ));
        if have_all && gate_ok {
            break;
        }
        if Instant::now() >= deadline {
            die(
                EXIT_TIMEOUT,
                clk,
                "120s elapsed before receiving A's 5 events + gate latch",
            );
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    clk.log(format!("converged on A's seeds; gate={}", core.get_alphabet_gate_state().method));

    // Linger 75s, syncing every 5s, so A's pull loop observes our entries.
    let linger = Instant::now() + Duration::from_secs(75);
    while Instant::now() < linger {
        tokio::time::sleep(Duration::from_secs(5)).await;
        if let Some(st) = try_op(clk, "sync_iroh_now (linger)", core.sync_iroh_now()).await {
            clk.log(format!(
                "linger sync: sent={} received={} settings={}",
                st.reviews_sent, st.reviews_received, st.settings_applied
            ));
        }
    }

    let ex = export(&core, clk);
    write_proto(&dir, "export_b.json", &ex.json, clk);
    clk.log(format!("B done (exit 0); |uuids|={}", ex.uuids.len()));
}

// --- nostr flows (shared identity: both sides read nsec_in.hex) --------------

/// A flow (nostr): set nsec/relays -> seed+latch -> sync_nostr_push ->
/// publish seeds_a.json + push_done.flag -> wait b_pushed.flag (<=90s) ->
/// poll sync_nostr_pull (3s, 90s deadline) until |uuids|==8 -> same terminal
/// artifacts as iroh A -> exit 0.
async fn serve_nostr(cli: &Cli, clk: &Clock) {
    let dir = cli.dir.clone().unwrap();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        die(EXIT_ASSERT, clk, format!("create {}: {e}", dir.display()));
    }
    let core = open_core(&cli.db, clk);
    clk.log(format!("device_id={}", core.sync_device_id()));

    nostr_setup(&core, cli, &dir, clk).await;
    seed_a(&core, clk);
    let seeds = export(&core, clk);
    if seeds.uuids.len() != 5 {
        die(EXIT_ASSERT, clk, format!("A seeded {} reviews, expected 5", seeds.uuids.len()));
    }

    let st = op(clk, "sync_nostr_push", core.sync_nostr_push()).await;
    clk.log(format!(
        "nostr push: sent={} received={} settings={}",
        st.reviews_sent, st.reviews_received, st.settings_applied
    ));
    if st.reviews_sent != 5 {
        die(EXIT_ASSERT, clk, format!("nostr push sent {}, expected 5", st.reviews_sent));
    }

    write_proto(
        &dir,
        "seeds_a.json",
        &json!({ "uuids": seeds.uuids, "hlc": seeds.to_hlc }).to_string(),
        clk,
    );
    write_proto(&dir, "push_done.flag", "done\n", clk);

    poll_file(&dir.join("b_pushed.flag"), Duration::from_secs(90), "b_pushed.flag", clk).await;

    let deadline = Instant::now() + Duration::from_secs(90);
    let mut converged: Option<Export> = None;
    while Instant::now() < deadline {
        match try_op(clk, "sync_nostr_pull", core.sync_nostr_pull()).await {
            Some(st) => clk.log(format!("nostr pull: received={} settings={}", st.reviews_received, st.settings_applied)),
            None => {
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
        }
        let ex = export(&core, clk);
        if ex.uuids.len() == 8 {
            clk.log(format!("converged: |uuids|==8 (received {} non-local)", ex.set.difference(&seeds.set).count()));
            converged = Some(ex);
            break;
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    let Some(ex) = converged else {
        die(EXIT_TIMEOUT, clk, "90s of pulls did not yield |uuids|==8");
    };
    let received_nonlocal = ex.set.difference(&seeds.set).count() as u32;

    write_proto(&dir, "export_a.json", &ex.json, clk);
    write_proto(
        &dir,
        "stats_a.json",
        &json!({ "receivedNonlocal": received_nonlocal, "exportSize": ex.uuids.len() }).to_string(),
        clk,
    );
    write_proto(&dir, "pull_done.flag", "done\n", clk);
    clk.log("A done (exit 0)");
}

/// B flow (nostr): set same nsec/relays -> wait push_done.flag (<=90s) ->
/// poll sync_nostr_pull (<=60s) until A's seeds are present -> seed h1..h3 ->
/// sync_nostr_push -> b_pushed.flag + export_b.json -> exit 0.
async fn join_nostr(cli: &Cli, clk: &Clock) {
    let dir = cli.dir.clone().unwrap();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        die(EXIT_ASSERT, clk, format!("create {}: {e}", dir.display()));
    }
    let core = open_core(&cli.db, clk);
    clk.log(format!("device_id={}", core.sync_device_id()));

    nostr_setup(&core, cli, &dir, clk).await;

    poll_file(
        &dir.join("push_done.flag"),
        Duration::from_secs(90),
        "push_done.flag",
        clk,
    )
    .await;
    let seeds_set = {
        let raw = poll_file(&dir.join("seeds_a.json"), Duration::from_secs(30), "seeds_a.json", clk).await;
        read_seeds(&raw).unwrap_or_else(|| die(EXIT_ASSERT, clk, "seeds_a.json had no uuids"))
    };

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(st) = try_op(clk, "sync_nostr_pull", core.sync_nostr_pull()).await {
            clk.log(format!("nostr pull: received={} settings={}", st.reviews_received, st.settings_applied));
        }
        let ex = export(&core, clk);
        let have_all = seeds_set.iter().all(|u| ex.set.contains(u));
        clk.log(format!("progress: uuids={} seeds_superset={have_all}", ex.uuids.len()));
        if have_all {
            break;
        }
        if Instant::now() >= deadline {
            die(EXIT_TIMEOUT, clk, "60s of pulls did not yield A's 5 seed events");
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    clk.log("converged on A's seeds over nostr spool");

    seed_b(&core, clk);
    let st = op(clk, "sync_nostr_push", core.sync_nostr_push()).await;
    // B's push cursor is fresh, so its first push re-uploads the 5 events it
    // pulled from the spool (idempotent by design — A's pull-since-cursor
    // skips them) plus its own 3 seeds. Assert the floor, not the exact count.
    clk.log(format!("nostr push: sent={} (>=3: h1..h3 + re-push of pulled)", st.reviews_sent));
    if st.reviews_sent < 3 {
        die(EXIT_ASSERT, clk, format!("B nostr push sent {}, expected >= 3", st.reviews_sent));
    }

    let ex = export(&core, clk);
    write_proto(&dir, "b_pushed.flag", "done\n", clk);
    write_proto(&dir, "export_b.json", &ex.json, clk);
    clk.log(format!("B done (exit 0); |uuids|={}", ex.uuids.len()));
}

// --- diagnostics -------------------------------------------------------------

fn cmd_dump(db: &str) {
    let clk = Clock::new("-");
    let core = open_core(db, &clk);
    let ex = export(&core, &clk);
    println!("{}", json!({ "uuids": ex.uuids }));
}

async fn cmd_ticket(db: &str) {
    let clk = Clock::new("-");
    let core = open_core(db, &clk);
    let ticket = op(&clk, "sync_iroh_ticket", core.sync_iroh_ticket()).await;
    println!("{ticket}");
}
