//! Supabase Realtime client, implemented directly on the wire protocol.
//!
//! Supabase Realtime is a Phoenix Channels server. The overlay's JS frontend
//! gets a ready-made client (supabase-js); here we speak the protocol
//! ourselves so the chat pane compiles to one dependency-free binary:
//!
//!   1. Connect to  wss://<ref>.supabase.co/realtime/v1/websocket?apikey=K&vsn=1.0.0
//!      (vsn=1.0.0 selects the JSON-object message framing supabase-js uses).
//!   2. Send `phx_join` on topic "realtime:global-chat" with a config payload
//!      (broadcast self:true so our own messages echo back, presence key).
//!   3. After the ok reply: send a presence `track`, then exchange
//!      `broadcast` events. A `heartbeat` on topic "phoenix" every 25s keeps
//!      the server from dropping us (its idle timeout is ~60s).
//!   4. `presence_state` (full snapshot) and `presence_diff` (joins/leaves)
//!      maintain the online count.
//!
//! Every wire message is `{"topic", "event", "payload", "ref"}` JSON.

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashSet;
use tokio::sync::mpsc;
use tokio::time::{interval, sleep, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

pub const CHANNEL: &str = "global-chat";
const HEARTBEAT_SECS: u64 = 25;
const RECONNECT_BASE_MS: u64 = 3000;
const RECONNECT_JITTER_MS: u64 = 2000;

/// Events flowing realtime → UI.
#[derive(Debug, Clone)]
pub enum RtEvent {
    Connected,
    Disconnected,
    Chat {
        username: String,
        text: String,
        timestamp_ms: Option<i64>,
    },
    Online(usize),
}

/// Commands flowing UI → realtime.
#[derive(Debug)]
pub enum RtCommand {
    /// payload: {username, text, timestamp}
    Broadcast(Value),
}

/// Spawns the connection task; returns (command sender, event receiver).
/// The task reconnects forever with jittered backoff and re-tracks presence
/// after every successful re-join.
pub fn spawn(
    url: String,
    key: String,
    username: String,
) -> (
    mpsc::UnboundedSender<RtCommand>,
    mpsc::UnboundedReceiver<RtEvent>,
) {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (evt_tx, evt_rx) = mpsc::unbounded_channel();
    tokio::spawn(run(url, key, username, cmd_rx, evt_tx));
    (cmd_tx, evt_rx)
}

async fn run(
    url: String,
    key: String,
    username: String,
    mut cmd_rx: mpsc::UnboundedReceiver<RtCommand>,
    evt_tx: mpsc::UnboundedSender<RtEvent>,
) {
    // Presence is keyed per *process*, not per username, so two people with
    // the same name (or two panes on one machine) still count separately.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let presence_key = format!("tui-{:x}-{:x}", std::process::id(), nanos);

    let ws_url = format!(
        "{}/realtime/v1/websocket?apikey={}&vsn=1.0.0",
        url.trim_end_matches('/').replacen("https://", "wss://", 1),
        key
    );

    loop {
        let ended = session(&ws_url, &key, &username, &presence_key, &mut cmd_rx, &evt_tx).await;
        if evt_tx.send(RtEvent::Disconnected).is_err() || ended {
            return; // UI is gone — stop reconnecting
        }
        // Jittered backoff so a fleet of clients doesn't stampede the server
        // when it comes back up.
        let jitter = (nanos as u64).wrapping_mul(31) % RECONNECT_JITTER_MS;
        sleep(Duration::from_millis(RECONNECT_BASE_MS + jitter)).await;
    }
}

/// One connection lifetime. Returns true if the UI side hung up (stop for
/// good), false on any network/server failure (reconnect).
async fn session(
    ws_url: &str,
    key: &str,
    username: &str,
    presence_key: &str,
    cmd_rx: &mut mpsc::UnboundedReceiver<RtCommand>,
    evt_tx: &mpsc::UnboundedSender<RtEvent>,
) -> bool {
    let Ok((ws, _)) = connect_async(ws_url).await else {
        return false;
    };
    let (mut sink, mut stream) = ws.split();
    let topic = format!("realtime:{CHANNEL}");

    // Join the channel. `access_token` is the same publishable key — that is
    // exactly what supabase-js sends when no user is logged in.
    let join = json!({
        "topic": topic,
        "event": "phx_join",
        "ref": "1",
        "payload": {
            "config": {
                "broadcast": { "ack": false, "self": true },
                "presence": { "key": presence_key },
                "postgres_changes": [],
                "private": false
            },
            "access_token": key
        }
    });
    if sink.send(WsMessage::Text(join.to_string())).await.is_err() {
        return false;
    }

    let mut heartbeat = interval(Duration::from_secs(HEARTBEAT_SECS));
    heartbeat.tick().await; // consume the immediate first tick
    let mut msg_ref: u64 = 2;
    let mut present: HashSet<String> = HashSet::new();

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                let hb = json!({
                    "topic": "phoenix", "event": "heartbeat",
                    "payload": {}, "ref": msg_ref.to_string()
                });
                msg_ref += 1;
                if sink.send(WsMessage::Text(hb.to_string())).await.is_err() {
                    return false;
                }
            }

            cmd = cmd_rx.recv() => {
                let Some(RtCommand::Broadcast(payload)) = cmd else {
                    return true; // sender dropped → UI exited
                };
                let msg = json!({
                    "topic": topic, "event": "broadcast",
                    "payload": { "type": "broadcast", "event": "message", "payload": payload },
                    "ref": msg_ref.to_string()
                });
                msg_ref += 1;
                if sink.send(WsMessage::Text(msg.to_string())).await.is_err() {
                    return false;
                }
            }

            frame = stream.next() => {
                let Some(Ok(frame)) = frame else {
                    return false; // socket closed or errored
                };
                let WsMessage::Text(text) = frame else {
                    if matches!(frame, WsMessage::Close(_)) { return false; }
                    continue; // ping/pong/binary — tungstenite answers pings itself
                };
                let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                let event = v["event"].as_str().unwrap_or("");

                match event {
                    // Reply to our phx_join (ref "1"): on ok, announce
                    // ourselves to presence. track() must re-run after every
                    // re-join, which happens naturally because each session()
                    // call goes through this path again.
                    "phx_reply" if v["ref"] == "1" => {
                        if v["payload"]["status"] == "ok" {
                            let _ = evt_tx.send(RtEvent::Connected);
                            let now_ms = chrono::Utc::now().timestamp_millis();
                            let track = json!({
                                "topic": topic, "event": "presence",
                                "payload": {
                                    "type": "presence", "event": "track",
                                    "payload": { "username": username, "joinedAt": now_ms }
                                },
                                "ref": msg_ref.to_string()
                            });
                            msg_ref += 1;
                            if sink.send(WsMessage::Text(track.to_string())).await.is_err() {
                                return false;
                            }
                        } else {
                            return false; // join refused (bad key, paused project…)
                        }
                    }

                    "broadcast" => {
                        if v["payload"]["event"] == "message" {
                            let p = &v["payload"]["payload"];
                            if let (Some(username), Some(text)) =
                                (p["username"].as_str(), p["text"].as_str())
                            {
                                let _ = evt_tx.send(RtEvent::Chat {
                                    username: username.to_string(),
                                    text: text.to_string(),
                                    timestamp_ms: p["timestamp"].as_i64(),
                                });
                            }
                        }
                    }

                    // Full presence snapshot: sent right after join and after
                    // server-side re-syncs. Keys are presence keys, one per
                    // connected client.
                    "presence_state" => {
                        present = v["payload"]
                            .as_object()
                            .map(|o| o.keys().cloned().collect())
                            .unwrap_or_default();
                        let _ = evt_tx.send(RtEvent::Online(present.len()));
                    }

                    // Incremental joins/leaves after the snapshot.
                    "presence_diff" => {
                        if let Some(joins) = v["payload"]["joins"].as_object() {
                            present.extend(joins.keys().cloned());
                        }
                        if let Some(leaves) = v["payload"]["leaves"].as_object() {
                            for k in leaves.keys() {
                                present.remove(k);
                            }
                        }
                        let _ = evt_tx.send(RtEvent::Online(present.len()));
                    }

                    // Server-side channel teardown → reconnect from scratch.
                    "phx_error" | "phx_close" => {
                        if v["topic"] == json!(topic) {
                            return false;
                        }
                    }

                    _ => {}
                }
            }
        }
    }
}
