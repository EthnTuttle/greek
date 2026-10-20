//! Scripted fake nostr relay used by the share-nostr integration tests.
//!
//! Built directly on `async-wsocket` (the same wire stack nostr-relay-builder
//! itself uses) because relay-builder's `WritePolicy` rejections are always
//! re-wrapped as `blocked: <msg>`, whereas Appendix R needs fixtures that
//! emit each distinct OK-false prefix verbatim, plus mid-publish socket
//! closes. Speaks a minimal NIP-01 subset: EVENT (scripted OK response +
//! recording), REQ (serves stored events for author/kind filters + EOSE),
//! CLOSE (dropped).

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use async_wsocket::native::{self, Message};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};

/// How the relay responds to an incoming EVENT.
#[derive(Debug, Clone)]
pub enum EventScript {
    /// Store the event and answer `OK true`; if the same id arrives again,
    /// answer `OK false "duplicate: already have this event"` (what real
    /// relays do).
    AcceptAll,
    /// Answer `OK false "<msg>"` for every event (recorded, not stored).
    /// The msg MUST carry its own machine-readable prefix
    /// (e.g. "rate-limited: slow down").
    Reject(String),
    /// Tear the connection down the instant an EVENT arrives, without any
    /// OK — simulates a mid-publish socket death.
    CloseOnEvent,
}

#[derive(Default)]
pub struct RelayState {
    /// Every EVENT json the relay has ever received, in order.
    pub received: Vec<Value>,
    /// Every REQ the relay has ever received, in order.
    pub reqs: Vec<Value>,
    /// Events the relay "has" (served in REQs); the read-back fixture seeds
    /// this directly.
    pub stored: Vec<Value>,
}

pub struct FakeRelay {
    pub url: String,
    pub script: Arc<Mutex<EventScript>>,
    pub state: Arc<Mutex<RelayState>>,
    join: Option<tokio::task::JoinHandle<()>>,
}

impl FakeRelay {
    pub async fn start(script: EventScript) -> Self {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(RelayState::default()));
        let script = Arc::new(Mutex::new(script));
        let st = state.clone();
        let sc = script.clone();
        let join = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, addr)) => {
                        let st = st.clone();
                        let sc = sc.clone();
                        tokio::spawn(async move {
                            let _ = handle_conn(stream, addr, st, sc).await;
                        });
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url: format!("ws://{addr}"),
            script,
            state,
            join: Some(join),
        }
    }

    pub fn set_script(&self, script: EventScript) {
        *self.script.lock().unwrap() = script;
    }

    /// Events physically received over the wire (publishes).
    pub fn received(&self) -> Vec<Value> {
        self.state.lock().unwrap().received.clone()
    }

    pub fn received_count(&self) -> usize {
        self.state.lock().unwrap().received.len()
    }

    pub fn req_count(&self) -> usize {
        self.state.lock().unwrap().reqs.len()
    }

    /// Plant an event the relay will serve in REQs without it ever having
    /// been published over this connection (the read-back fixture).
    pub fn seed_stored(&self, event: Value) {
        self.state.lock().unwrap().stored.push(event);
    }

    pub fn stored(&self) -> Vec<Value> {
        self.state.lock().unwrap().stored.clone()
    }

    pub async fn stop(self) {
        if let Some(j) = self.join {
            j.abort();
        }
    }
}

async fn handle_conn(
    stream: tokio::net::TcpStream,
    _addr: SocketAddr,
    state: Arc<Mutex<RelayState>>,
    script: Arc<Mutex<EventScript>>,
) -> anyhow::Result<()> {
    let ws = native::accept(stream).await?;
    let (mut tx, mut rx) = ws.split();
    let mut alive = true;
    while alive {
        let Some(msg) = rx.next().await else { break };
        let msg = match msg {
            Ok(m) => m,
            Err(_) => break,
        };
        let text = match msg {
            Message::Text(t) => t,
            Message::Ping(p) => {
                tx.send(Message::Pong(p)).await?;
                continue;
            }
            Message::Close(_) => break,
            _ => continue,
        };
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let arr = match v.as_array() {
            Some(a) => a,
            None => continue,
        };
        match arr.first().and_then(Value::as_str) {
            Some("EVENT") if arr.len() >= 2 => {
                let ev = arr[1].clone();
                let id = ev["id"].as_str().unwrap_or("").to_string();
                state.lock().unwrap().received.push(ev.clone());
                let behavior = script.lock().unwrap().clone();
                match behavior {
                    EventScript::AcceptAll => {
                        let (reply, store) = {
                            let mut st = state.lock().unwrap();
                            let dup =
                                st.stored.iter().any(|e| e["id"].as_str() == Some(id.as_str()));
                            if dup {
                                (json!(["OK", id, false, "duplicate: already have this event"]), false)
                            } else {
                                st.stored.push(ev.clone());
                                (json!(["OK", id, true, ""]), true)
                            }
                        };
                        let _ = store;
                        alive = send_text(&mut tx, &reply).await?;
                    }
                    EventScript::Reject(m) => {
                        let reply = json!(["OK", id, false, m]);
                        alive = send_text(&mut tx, &reply).await?;
                    }
                    EventScript::CloseOnEvent => {
                        // No OK at all: socket dies mid-publish.
                        return Ok(());
                    }
                }
            }
            Some("REQ") if arr.len() >= 3 => {
                let sub = arr[1].clone();
                state.lock().unwrap().reqs.push(v.clone());
                // Incredibly naive filter: honor "authors" + "kinds" + limit.
                let filter = &arr[2];
                let authors: Vec<String> = filter["authors"]
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default();
                let kinds: Vec<i64> = filter["kinds"]
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_i64).collect())
                    .unwrap_or_default();
                let limit = filter["limit"].as_u64().unwrap_or(50) as usize;
                let matches: Vec<Value> = {
                    let st = state.lock().unwrap();
                    st.stored
                        .iter()
                        .filter(|e| {
                            authors
                                .iter()
                                .any(|a| e["pubkey"].as_str() == Some(a.as_str()))
                                && kinds.iter().any(|k| e["kind"].as_i64() == Some(*k))
                        })
                        .take(limit)
                        .cloned()
                        .collect()
                };
                for ev in matches {
                    alive = send_text(&mut tx, &json!(["EVENT", sub, ev])).await?;
                    if !alive {
                        break;
                    }
                }
                alive = send_text(&mut tx, &json!(["EOSE", sub])).await?;
            }
            Some("CLOSE") | Some("COUNT") => {}
            _ => {}
        }
    }
    Ok(())
}

async fn send_text<S>(tx: &mut futures_util::stream::SplitSink<S, Message>, v: &Value) -> anyhow::Result<bool>
where
    S: futures_util::Stream + futures_util::Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    match tx.send(Message::Text(v.to_string().into())).await {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

// Silence unused-import warnings if a subset of helpers gets used per test target.
#[allow(dead_code)]
fn _unused(_: VecDeque<()>) {}
