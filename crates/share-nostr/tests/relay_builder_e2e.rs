//! End-to-end against a REAL local nostr relay instance —
//! `nostr-relay-builder`'s `LocalRelay` with its actual in-memory store,
//! verification, and genuine `duplicate:` OK semantics for event resubmits.

use nostr_relay_builder::builder::{RateLimit, RelayBuilder};
use nostr_relay_builder::LocalRelay;

use share_nostr::identity::ShareKeys;
use share_nostr::publisher::{OkClass, Publisher};

fn keys() -> ShareKeys {
    ShareKeys::derive_from_sync(&[0x77; 32]).unwrap()
}

async fn start_relay() -> LocalRelay {
    // RelayBuilder::port(0) binds an ephemeral listener but reports
    // `ws://127.0.0.1:0` from url() — reserve a port first instead.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("reserve ephemeral port")
        .local_addr()
        .unwrap()
        .port();
    LocalRelay::run(
        RelayBuilder::default()
            .addr(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
            .port(port)
            .rate_limit(RateLimit {
                max_reqs: 1000,
                notes_per_minute: 1_000_000,
            }),
    )
    .await
    .expect("start local relay")
}

#[tokio::test]
async fn real_relay_accepts_then_answers_duplicate_on_identical_resend() {
    let relay = start_relay().await;
    let url = relay.url().to_string();
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
#[tokio::test]
async fn real_relay_signature_gate_passes_for_share_events() {
    let relay = start_relay().await;
    let url = relay.url().to_string();
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
