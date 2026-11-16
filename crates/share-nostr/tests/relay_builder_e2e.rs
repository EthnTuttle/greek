//! End-to-end against a REAL local nostr relay instance —
//! `nostr-relay-builder`'s `LocalRelay` with its actual in-memory store,
//! verification, and genuine `duplicate:` OK semantics for event resubmits.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use nostr_relay_builder::builder::{PolicyResult, RateLimit, RelayBuilder, WritePolicy};
use nostr_relay_builder::LocalRelay;
use nostr_sdk::nostr::util::BoxedFuture;
use nostr_sdk::prelude::Event;
use tokio::sync::Notify;

use share_nostr::identity::ShareKeys;
use share_nostr::ops;
use share_nostr::publisher::{OkClass, Publisher};
use share_nostr::templates::{self, TemplateLang};

fn keys() -> ShareKeys {
    ShareKeys::derive_from_sync(&[0x77; 32]).unwrap()
}

fn builder() -> RelayBuilder {
    // Reserving the port first keeps the fixture deterministic; url() is
    // async in nostr-relay-builder 0.44 and reports the bound address.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("reserve ephemeral port")
        .local_addr()
        .unwrap()
        .port();
    RelayBuilder::default()
        .addr(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
        .port(port)
        .rate_limit(RateLimit {
            max_reqs: 1000,
            notes_per_minute: 1_000_000,
        })
}

async fn start_relay() -> LocalRelay {
    let relay = LocalRelay::new(builder());
    relay.run().await.expect("start local relay");
    relay
}

#[derive(Debug)]
struct EventGate {
    entered: Arc<Notify>,
    resume: Arc<Notify>,
}

impl WritePolicy for EventGate {
    fn admit_event<'a>(
        &'a self,
        _event: &'a Event,
        _addr: &'a SocketAddr,
    ) -> BoxedFuture<'a, PolicyResult> {
        Box::pin(async {
            self.entered.notify_one();
            self.resume.notified().await;
            PolicyResult::Accept
        })
    }
}

#[tokio::test]
async fn real_relay_accepts_then_answers_duplicate_on_identical_resend() {
    let relay = start_relay().await;
    let url = relay.url().await.to_string();
    let keys = keys();
    let publisher = Publisher::connect(&keys, std::slice::from_ref(&url))
        .await
        .unwrap();

    let event = keys.sign_note("7-day Greek streak — every due review cleared, day after day 🔥").unwrap();
    let first = publisher.publish(&event).await.unwrap();
    assert_eq!(first.outcomes.len(), 1);
    assert_eq!(first.outcomes[0].class, OkClass::Accepted);

    // Identical bytes re-sent: nostr-relay-builder answers
    // `OK true "duplicate: already have this event"` (a *successful* OK,
    // unlike strfry-style OK-false duplicates, which our scripted-fixture
    // tests cover). From a delivery-posture view both are "delivered".
    let publisher2 = Publisher::connect(&keys, std::slice::from_ref(&url))
        .await
        .unwrap();
    let resend = publisher2.publish(&event).await.unwrap();
    assert_eq!(resend.outcomes.len(), 1);
    assert!(
        resend.outcomes[0].class.delivered(),
        "identical resend must converge to delivered, got {:?}",
        resend.outcomes[0]
    );
    publisher2.disconnect().await;

    // Read-back resolves it too.
    assert!(publisher.content_already_posted(&event.content).await.unwrap());
    assert!(
        !publisher
            .content_already_posted("some other content entirely")
            .await
            .unwrap()
    );

    publisher.disconnect().await;
    relay.shutdown();
}

/// Events signed by ShareKeys are NIP-01 valid for a strict relay: id and
/// signature verify server-side (relay-builder rejects bad signatures, so
/// acceptance itself is the assertion).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn facade_flush_releases_db_lock_while_relay_publish_is_in_flight() {
    let entered = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let relay = LocalRelay::new(builder().write_policy(EventGate {
        entered: entered.clone(),
        resume: resume.clone(),
    }));
    relay.run().await.expect("start gated local relay");
    let url = relay.url().await.to_string();

    let db = Arc::new(Mutex::new(greek_core::Database::open(":memory:", None).unwrap()));
    {
        let guard = db.lock().unwrap();
        greek_core::sync::setting_put(&guard, ops::ENABLED_KEY, "1").unwrap();
        greek_core::sync::setting_put(&guard, "share.relays", &url).unwrap();
        greek_core::secret_put(&guard, "nostr_nsec", &"77".repeat(32)).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        ops::enqueue_milestone(
            &guard,
            templates::SLUG_CHAPTER_MASTERED,
            "chapter.mastered:athenaze/1",
            "athenaze/1",
            TemplateLang::En,
            None,
            None,
            &keys(),
            now,
        )
        .unwrap();
    }

    let flush_db = db.clone();
    let flush = tokio::spawn(async move { share_nostr::flush_now(&*flush_db).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .expect("publish reached the relay write policy");

    // The relay is now deliberately parked BEFORE its OK response. A local
    // learning/DB read must nevertheless finish: the old facade held this
    // exact mutex for the full network wait and this step timed out.
    let read_db = db.clone();
    let read = tokio::task::spawn_blocking(move || {
        let guard = read_db.lock().unwrap();
        guard
            .conn()
            .query_row("SELECT COUNT(*) FROM user_settings", [], |r| r.get::<_, i64>(0))
            .unwrap()
    });
    let settings_rows = tokio::time::timeout(std::time::Duration::from_secs(1), read)
        .await
        .expect("local DB read completed while relay publish was unresolved")
        .unwrap();
    assert!(settings_rows >= 1);

    resume.notify_one();
    let report = flush.await.unwrap().unwrap();
    assert_eq!(report.posted, 1);
    relay.shutdown();
}

#[tokio::test]
async fn real_relay_signature_gate_passes_for_share_events() {
    let relay = start_relay().await;
    let url = relay.url().await.to_string();
    let keys = keys();
    let publisher = Publisher::connect(&keys, std::slice::from_ref(&url))
        .await
        .unwrap();
    let long_greek = format!("Χαῖρε! {}", "προσκυνῶ ".repeat(100));
    let event = keys.sign_note(&long_greek).unwrap();
    let res = publisher.publish(&event).await.unwrap();
    assert_eq!(res.outcomes[0].class, OkClass::Accepted, "{:?}", res.outcomes[0]);
    publisher.disconnect().await;
    relay.shutdown();
}
