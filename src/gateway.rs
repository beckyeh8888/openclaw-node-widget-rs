use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::config;

pub fn mask_token(token: &str) -> String {
    if token.len() <= 8 {
        return "****".to_string();
    }
    format!("{}***{}", &token[..4], &token[token.len() - 4..])
}

const PKCS8_PRIVATE_KEY_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];
const SPKI_PUBLIC_KEY_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];
const DEVICE_PRIVATE_KEY_FILE: &str = "device.key";
const DEVICE_PUBLIC_KEY_FILE: &str = "device.pub";

#[derive(Debug)]
pub struct GatewayClient {
    pub connection_name: String,
    pub url: String,
    pub token: Option<String>,
    pub device_id: String,
    pub public_key_pem: String,
    pub private_key: SigningKey,
    pub tx: mpsc::UnboundedSender<GatewayEvent>,
    pub chat_state: Arc<Mutex<crate::chat::ChatState>>,
}

#[derive(Debug, Clone, Default)]
pub struct GatewayStats {
    pub active_sessions: u32,
    pub total_errors_24h: u32,
    pub last_agent_activity: Option<String>,
}

#[derive(Debug, Clone)]
pub enum GatewayEvent {
    Connected {
        connection_name: String,
        gateway_version: Option<String>,
    },
    Disconnected {
        connection_name: String,
        reason: String,
    },
    NodeStatus {
        connection_name: String,
        online: bool,
        node_name: Option<String>,
        stats: GatewayStats,
    },
    Error {
        connection_name: String,
        message: String,
    },
    Latency {
        #[allow(dead_code)]
        connection_name: String,
        ms: u64,
    },
}

#[derive(Debug, Clone)]
pub struct SignatureParams<'a> {
    pub device_id: &'a str,
    pub client_id: &'a str,
    pub client_mode: &'a str,
    pub role: &'a str,
    pub scopes: &'a str,
    pub signed_at_ms: i64,
    pub token: &'a str,
    pub nonce: &'a str,
    pub platform: &'a str,
    pub device_family: &'a str,
}

#[derive(Debug)]
enum ConnectError {
    Retryable(String),
    Fatal(String),
}

pub fn load_or_create_keypair(config_dir: &Path) -> Result<(SigningKey, String, String), String> {
    fs::create_dir_all(config_dir)
        .map_err(|e| format!("failed to create config dir {}: {e}", config_dir.display()))?;

    let private_path = config_dir.join(DEVICE_PRIVATE_KEY_FILE);
    let public_path = config_dir.join(DEVICE_PUBLIC_KEY_FILE);

    let signing_key = if private_path.exists() {
        load_signing_key_from_pem(&private_path)?
    } else {
        let mut rng = rand::rngs::OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        let private_pem = private_key_to_pem(&signing_key);
        fs::write(&private_path, private_pem)
            .map_err(|e| format!("failed to write {}: {e}", private_path.display()))?;
        signing_key
    };

    let verifying_key = signing_key.verifying_key();
    let public_key_pem = public_key_to_pem(&verifying_key);
    fs::write(&public_path, &public_key_pem)
        .map_err(|e| format!("failed to write {}: {e}", public_path.display()))?;

    let device_id = compute_device_id(&verifying_key);
    Ok((signing_key, device_id, public_key_pem))
}

pub fn compute_device_id(public_key: &VerifyingKey) -> String {
    // Gateway derives deviceId from raw 32-byte public key (SPKI DER minus prefix)
    let digest = Sha256::digest(public_key.as_bytes());
    hex::encode(digest)
}

pub fn build_signature_payload_v3(params: &SignatureParams<'_>) -> String {
    [
        "v3",
        params.device_id,
        params.client_id,
        params.client_mode,
        params.role,
        params.scopes,
        &params.signed_at_ms.to_string(),
        params.token,
        params.nonce,
        params.platform,
        params.device_family,
    ]
    .join("|")
}

pub fn sign_payload(key: &SigningKey, payload: &str) -> String {
    let signature = key.sign(payload.as_bytes());
    BASE64.encode(signature.to_bytes())
}

pub async fn spawn_connection(
    connection_name: String,
    url: Option<String>,
    token: Option<String>,
    tx: mpsc::UnboundedSender<GatewayEvent>,
    chat_state: Arc<Mutex<crate::chat::ChatState>>,
    cmd_rx: Option<mpsc::UnboundedReceiver<GatewayCommand>>,
) -> bool {
    let Some(url) = url.filter(|v| !v.trim().is_empty()) else {
        return false;
    };

    let config_dir = match config::app_dir() {
        Ok(path) => path,
        Err(err) => {
            let _ = tx.send(GatewayEvent::Error {
                connection_name,
                message: format!("gateway config path error: {err}"),
            });
            return false;
        }
    };

    let (private_key, device_id, public_key_pem) = match load_or_create_keypair(&config_dir) {
        Ok(values) => values,
        Err(err) => {
            let _ = tx.send(GatewayEvent::Error {
                connection_name,
                message: err,
            });
            return false;
        }
    };

    let client = GatewayClient {
        connection_name,
        url,
        token,
        device_id,
        public_key_pem,
        private_key,
        tx,
        chat_state,
    };

    tokio::spawn(async move {
        connect_loop(client, cmd_rx).await;
    });

    true
}

/// Spawn gateway tasks for all configured connections.
/// Returns (count_spawned, optional_command_sender_for_first_connection).
pub async fn spawn_all_connections(
    connections: &[config::ConnectionConfig],
    tx: mpsc::UnboundedSender<GatewayEvent>,
    chat_state: Arc<Mutex<crate::chat::ChatState>>,
) -> (usize, Option<mpsc::UnboundedSender<GatewayCommand>>) {
    let mut count = 0;
    let mut first_cmd_tx = None;
    for (i, conn) in connections.iter().enumerate() {
        // Only the first connection gets a command channel for chat
        let cmd_rx = if i == 0 {
            let (tx, rx) = mpsc::unbounded_channel();
            first_cmd_tx = Some(tx);
            Some(rx)
        } else {
            None
        };
        let spawned = spawn_connection(
            conn.name.clone(),
            Some(conn.gateway_url.clone()),
            conn.gateway_token.clone(),
            tx.clone(),
            Arc::clone(&chat_state),
            cmd_rx,
        )
        .await;
        if spawned {
            count += 1;
        }
    }
    (count, first_cmd_tx)
}

pub async fn connect_loop(client: GatewayClient, mut cmd_rx: Option<mpsc::UnboundedReceiver<GatewayCommand>>) {
    let mut delay_secs = 1u64;
    let name = client.connection_name.clone();

    loop {
        let result = connect_once(&client, &mut cmd_rx).await;

        // Notify chat state of disconnection
        if let Ok(mut cs) = client.chat_state.lock() {
            cs.inbox.push(crate::chat::ChatInbound::Disconnected);
        }

        match result {
            Ok(()) => {
                delay_secs = 1;
                let _ = client.tx.send(GatewayEvent::Disconnected {
                    connection_name: name.clone(),
                    reason: "gateway disconnected".to_string(),
                });
            }
            Err(ConnectError::Fatal(message)) => {
                let _ = client.tx.send(GatewayEvent::Error {
                    connection_name: name.clone(),
                    message,
                });
                break;
            }
            Err(ConnectError::Retryable(message)) => {
                let _ = client.tx.send(GatewayEvent::Disconnected {
                    connection_name: name.clone(),
                    reason: message,
                });
            }
        }

        let jitter = rand::thread_rng().gen_range(0..=(delay_secs / 4).max(1));
        let sleep_secs = delay_secs.saturating_add(jitter).min(60);
        tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
        delay_secs = (delay_secs.saturating_mul(2)).min(60);
    }
}

async fn connect_once(client: &GatewayClient, cmd_rx: &mut Option<mpsc::UnboundedReceiver<GatewayCommand>>) -> Result<(), ConnectError> {
    let masked_url = if let Some(ref t) = client.token {
        client.url.replace(t, &mask_token(t))
    } else {
        client.url.clone()
    };
    info!(url = %masked_url, "gateway connecting");
    let url = url::Url::parse(&client.url)
        .map_err(|e| ConnectError::Fatal(format!("invalid gateway URL: {e}")))?;
    let host = url.host_str().unwrap_or("127.0.0.1");
    let port = url.port().unwrap_or(18789);
    let addr = format!("{host}:{port}");

    info!(%addr, "resolving tcp connection");
    let tcp = tokio::net::TcpStream::connect(&addr)
        .await
        .map_err(|e| ConnectError::Retryable(format!("tcp connect failed: {e}")))?;

    info!("tcp connected, starting websocket handshake");
    let (stream, _) = tokio_tungstenite::client_async(
        url.as_str(),
        tcp,
    )
    .await
    .map_err(|e| ConnectError::Retryable(format!("ws handshake failed: {e}")))?;

    info!("gateway websocket connected, waiting for challenge...");

    let (mut write, mut read) = stream.split();

    let challenge_frame = timeout(Duration::from_secs(10), async {
        loop {
            let Some(message) = read.next().await else {
                return Err(ConnectError::Retryable(
                    "gateway closed before challenge".to_string(),
                ));
            };

            let message = message
                .map_err(|e| ConnectError::Retryable(format!("gateway read failed: {e}")))?;

            match message {
                Message::Text(text) => {
                    debug!(raw_frame = %text, "received gateway frame");
                    let frame = parse_frame(&text)?;
                    let ft = frame_type(&frame);
                    let fn_ = frame_name(&frame);
                    debug!(?ft, ?fn_, "parsed frame");
                    if ft == Some("event") && fn_ == Some("connect.challenge") {
                        info!("challenge received!");
                        return Ok(frame);
                    }
                }
                Message::Ping(payload) => {
                    debug!("received ping");
                    write
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|e| ConnectError::Retryable(format!("pong failed: {e}")))?;
                }
                Message::Close(close_frame) => {
                    let (code, reason) = match &close_frame {
                        Some(f) => (f.code.into(), f.reason.to_string()),
                        None => (0u16, "gateway closed".to_string()),
                    };
                    if code == 4001 || code == 4003 {
                        return Err(ConnectError::Fatal(format!("gateway auth rejected (code={code}): {reason}")));
                    }
                    return Err(ConnectError::Retryable(reason));
                }
                _ => {}
            }
        }
    })
    .await
    .map_err(|_| ConnectError::Retryable("connect challenge timeout".to_string()))??;

    let nonce = challenge_frame
        .get("payload")
        .and_then(|p| p.get("nonce"))
        .and_then(Value::as_str)
        .ok_or_else(|| ConnectError::Retryable("challenge missing nonce".to_string()))?;

    let token = client.token.clone().unwrap_or_default();
    let signed_at_ms = now_ms();
    let instance_id = Uuid::new_v4().to_string();
    let platform = platform_name();
    let connect_id = Uuid::new_v4().to_string();

    let role = "operator";
    let scopes = "operator.read,operator.write";
    let payload = build_signature_payload_v3(&SignatureParams {
        device_id: &client.device_id,
        client_id: "gateway-client",
        client_mode: "backend",
        role,
        scopes,
        signed_at_ms,
        token: &token,
        nonce,
        platform: &platform,
        device_family: "desktop",
    });
    let signature = sign_payload(&client.private_key, &payload);

    let connect_frame = json!({
        "type": "req",
        "id": connect_id,
        "method": "connect",
        "params": {
            "minProtocol": 3,
            "maxProtocol": 3,
            "client": {
                "id": "gateway-client",
                "version": env!("CARGO_PKG_VERSION"),
                "platform": platform,
                "deviceFamily": "desktop",
                "mode": "backend",
                "instanceId": instance_id
            },
            "device": {
                "id": client.device_id,
                "publicKey": client.public_key_pem,
                "signature": signature,
                "signedAt": signed_at_ms,
                "nonce": nonce
            },
            "role": role,
            "scopes": ["operator.read", "operator.write"],
            "caps": [],
            "auth": { "token": token },
            "locale": "en-US",
            "userAgent": format!("openclaw-node-widget/{}", env!("CARGO_PKG_VERSION"))
        }
    });

    let connect_json = connect_frame.to_string();
    info!(frame_len = connect_json.len(), "sending connect frame");
    // Mask token in debug output to prevent leaking credentials
    let masked_json = connect_json.replace(&token, &mask_token(&token));
    debug!(connect_frame = %masked_json, "connect frame payload");
    write
        .send(Message::Text(connect_json.into()))
        .await
        .map_err(|e| ConnectError::Retryable(format!("connect request send failed: {e}")))?;
    info!("connect frame sent, waiting for response...");

    let _hello_ok_frame: Value = loop {
        let Some(message) = read.next().await else {
            info!("gateway stream ended during connect (no more frames)");
            return Err(ConnectError::Retryable(
                "gateway closed during connect".to_string(),
            ));
        };

        let message = message.map_err(|e| {
            info!(error = %e, "gateway read error during connect");
            ConnectError::Retryable(format!("gateway read failed: {e}"))
        })?;

        debug!(msg_type = ?message, "received message during connect response");

        match message {
            Message::Text(text) => {
                debug!(response = %text, "connect response frame");
                let frame = parse_frame(&text)?;
                if frame_type(&frame) == Some("res") {
                    let id = frame.get("id").and_then(Value::as_str).unwrap_or_default();
                    if id == connect_id {
                        let ok = frame.get("ok").and_then(Value::as_bool).unwrap_or(false);
                        if ok {
                            break frame;
                        }

                        let reason = frame
                            .get("error")
                            .and_then(Value::as_str)
                            .or_else(|| {
                                frame
                                    .get("payload")
                                    .and_then(|v| v.get("message"))
                                    .and_then(Value::as_str)
                            })
                            .unwrap_or("connect rejected")
                            .to_string();

                        if reason.to_ascii_lowercase().contains("auth")
                            || reason.to_ascii_lowercase().contains("token")
                        {
                            return Err(ConnectError::Fatal(format!(
                                "gateway auth failed: {reason}"
                            )));
                        }

                        return Err(ConnectError::Retryable(format!(
                            "gateway connect rejected: {reason}"
                        )));
                    }
                }
            }
            Message::Ping(payload) => {
                write
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|e| ConnectError::Retryable(format!("pong failed: {e}")))?;
            }
            Message::Close(close_frame) => {
                let (code, reason) = match &close_frame {
                    Some(f) => (f.code.into(), f.reason.to_string()),
                    None => (0u16, "no close frame".to_string()),
                };
                info!(code, reason = %reason, "gateway sent close frame after connect");
                if code == 4001 || code == 4003 {
                    return Err(ConnectError::Fatal(format!(
                        "gateway auth rejected (code={code}): {reason}"
                    )));
                }
                return Err(ConnectError::Retryable(format!(
                    "gateway closed (code={code}): {reason}"
                )));
            }
            other => {
                info!(?other, "unexpected message type during connect");
            }
        }
    };

    // Extract gateway version from hello-ok response
    let gateway_version = _hello_ok_frame
        .get("payload")
        .and_then(|p| {
            p.get("version")
                .or_else(|| p.get("gatewayVersion"))
                .or_else(|| p.get("serverVersion"))
        })
        .and_then(Value::as_str)
        .map(String::from);

    info!(connection = %client.connection_name, ?gateway_version, "gateway connected, node status will be polled via node.list");
    let _ = client.tx.send(GatewayEvent::Connected {
        connection_name: client.connection_name.clone(),
        gateway_version,
    });

    // Notify chat state of connection
    if let Ok(mut cs) = client.chat_state.lock() {
        cs.inbox.push(crate::chat::ChatInbound::Connected);
    }

    // Attempt to subscribe to gateway events (best-effort, may not be supported)
    let subscribe_id = Uuid::new_v4().to_string();
    let subscribe_frame = json!({
        "type": "req",
        "id": subscribe_id,
        "method": "events.subscribe",
        "params": {
            "types": ["session.start", "session.end", "agent.error"]
        }
    });
    info!("sending events.subscribe request (best-effort)");
    if let Err(e) = write
        .send(Message::Text(subscribe_frame.to_string().into()))
        .await
    {
        info!("events.subscribe send failed (non-fatal): {e}");
    }

    let mut presence_ticker = tokio::time::interval(Duration::from_secs(30));
    let mut pending_presence: Option<String> = None;
    let mut ping_ticker = tokio::time::interval(Duration::from_secs(30));
    let mut ping_sent_at: Option<std::time::Instant> = None;
    let mut pending_chat_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    loop {
        tokio::select! {
            _ = ping_ticker.tick() => {
                let payload = b"ocw-ping".to_vec();
                if write.send(Message::Ping(payload.into())).await.is_ok() {
                    ping_sent_at = Some(std::time::Instant::now());
                }
            }
            _ = presence_ticker.tick() => {
                let req_id = Uuid::new_v4().to_string();
                info!(req_id = %req_id, "sending node.list request");
                let frame = json!({
                    "type": "req",
                    "id": req_id,
                    "method": "node.list",
                    "params": {}
                });
                write
                    .send(Message::Text(frame.to_string().into()))
                    .await
                    .map_err(|e| ConnectError::Retryable(format!("presence request send failed: {e}")))?;
                pending_presence = Some(req_id);
            }
            // Receive commands from the chat UI (only for the first connection)
            Some(cmd) = async {
                match cmd_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match cmd {
                    GatewayCommand::SendChat { message, session_key, attachments } => {
                        let req_id = Uuid::new_v4().to_string();
                        let idempotency_key = Uuid::new_v4().to_string();
                        let sk = session_key.unwrap_or_else(|| "main".to_string());
                        let mut params = json!({ "message": message, "idempotencyKey": idempotency_key, "sessionKey": sk });
                        if let Some(atts) = attachments {
                            let att_json: Vec<Value> = atts.iter().map(|a| {
                                json!({"data": a.data, "filename": a.filename, "mimeType": a.mime_type})
                            }).collect();
                            params.as_object_mut().unwrap().insert("attachments".to_string(), json!(att_json));
                        }
                        let frame = json!({
                            "type": "req",
                            "id": req_id,
                            "method": "chat.send",
                            "params": params
                        });
                        info!(req_id = %req_id, "sending chat.send");
                        pending_chat_ids.insert(req_id);
                        if let Err(e) = write.send(Message::Text(frame.to_string().into())).await {
                            warn!("chat.send failed: {e}");
                        }
                    }
                    GatewayCommand::ListSessions => {
                        let req_id = Uuid::new_v4().to_string();
                        let frame = json!({
                            "type": "req",
                            "id": req_id,
                            "method": "sessions.list",
                            "params": {}
                        });
                        info!(req_id = %req_id, "sending sessions.list");
                        pending_chat_ids.insert(req_id);
                        if let Err(e) = write.send(Message::Text(frame.to_string().into())).await {
                            warn!("sessions.list failed: {e}");
                        }
                    }
                }
            }
            message = read.next() => {
                let Some(message) = message else {
                    return Err(ConnectError::Retryable("gateway closed".to_string()));
                };

                match message.map_err(|e| ConnectError::Retryable(format!("gateway read failed: {e}")))? {
                    Message::Text(text) => {
                        let frame = parse_frame(&text)?;
                        handle_frame(&client.connection_name, &client.tx, &client.chat_state, &frame, pending_presence.as_deref(), &mut pending_chat_ids);

                        if frame_type(&frame) == Some("res") {
                            let id = frame.get("id").and_then(Value::as_str);
                            if id.is_some() && id == pending_presence.as_deref() {
                                pending_presence = None;
                            }
                        }
                    }
                    Message::Ping(payload) => {
                        write
                            .send(Message::Pong(payload))
                            .await
                            .map_err(|e| ConnectError::Retryable(format!("pong failed: {e}")))?;
                    }
                    Message::Pong(_) => {
                        if let Some(sent_at) = ping_sent_at.take() {
                            let ms = sent_at.elapsed().as_millis() as u64;
                            debug!(latency_ms = ms, "pong received");
                            let _ = client.tx.send(GatewayEvent::Latency {
                                connection_name: client.connection_name.clone(),
                                ms,
                            });
                        }
                    }
                    Message::Close(close_frame) => {
                        let reason = close_frame
                            .map(|f| f.reason.to_string())
                            .unwrap_or_else(|| "gateway closed".to_string());
                        return Err(ConnectError::Retryable(reason));
                    }
                    _ => {}
                }
            }
        }
    }
}

fn handle_frame(
    connection_name: &str,
    tx: &mpsc::UnboundedSender<GatewayEvent>,
    chat_state: &Arc<Mutex<crate::chat::ChatState>>,
    frame: &Value,
    pending_presence: Option<&str>,
    pending_chat_ids: &mut std::collections::HashSet<String>,
) {
    match frame_type(frame) {
        Some("event") => {
            if let Some(event_name) = frame_name(frame) {
                debug!(event = event_name, "gateway event received");
                if event_name == "chat" {
                    handle_chat_event(chat_state, frame.get("payload"));
                } else if let Some(event) = node_status_from_event(connection_name, event_name, frame.get("payload")) {
                    let _ = tx.send(event);
                }
            }
        }
        Some("res") => {
            let ok = frame.get("ok").and_then(Value::as_bool).unwrap_or(false);
            let id = frame.get("id").and_then(Value::as_str);

            // Check if this is a chat/sessions response
            if let Some(id_str) = id {
                if pending_chat_ids.remove(id_str) {
                    if ok {
                        handle_chat_response(chat_state, frame.get("payload"));
                    } else {
                        let error = frame.get("error").and_then(Value::as_str).unwrap_or("unknown error");
                        warn!(error, "chat/sessions request failed");
                    }
                    return;
                }
            }

            info!(ok, ?id, ?pending_presence, "response frame received");
            if id.is_some() && id == pending_presence {
                if ok {
                    if let Some(payload) = frame.get("payload") {
                        let node_count = payload.get("nodes").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0);
                        info!(node_count, "node.list response OK");
                    }
                    if let Some(event) = node_status_from_node_list(connection_name, frame.get("payload")) {
                        let _ = tx.send(event);
                    }
                } else {
                    let error = frame.get("error").or_else(|| frame.get("payload"));
                    info!(?error, "node.list request REJECTED");
                }
            }
        }
        _ => {}
    }
}

fn handle_chat_event(chat_state: &Arc<Mutex<crate::chat::ChatState>>, payload: Option<&Value>) {
    let Some(payload) = payload else {
        warn!("chat event with no payload");
        return;
    };

    let text = payload
        .get("text")
        .or_else(|| payload.get("message"))
        .or_else(|| payload.get("content"))
        .and_then(Value::as_str);

    let Some(text) = text else {
        warn!(?payload, "chat event with unknown format — skipping");
        return;
    };

    let agent_name = payload
        .get("agentName")
        .or_else(|| payload.get("agent"))
        .or_else(|| payload.get("sender"))
        .and_then(Value::as_str)
        .map(String::from);

    if let Ok(mut cs) = chat_state.lock() {
        cs.inbox.push(crate::chat::ChatInbound::Reply {
            text: text.to_string(),
            agent_name,
        });
    }
}

fn handle_chat_response(chat_state: &Arc<Mutex<crate::chat::ChatState>>, payload: Option<&Value>) {
    let Some(payload) = payload else { return };

    // sessions.list response: payload has "sessions" array
    if let Some(sessions) = payload.get("sessions").and_then(Value::as_array) {
        let session_list: Vec<ChatSessionInfo> = sessions
            .iter()
            .filter_map(|s| {
                let key = s.get("key").or_else(|| s.get("id")).and_then(Value::as_str)?;
                let name = s
                    .get("name")
                    .or_else(|| s.get("displayName"))
                    .and_then(Value::as_str)
                    .unwrap_or(key);
                Some(ChatSessionInfo {
                    key: key.to_string(),
                    name: name.to_string(),
                })
            })
            .collect();
        if let Ok(mut cs) = chat_state.lock() {
            cs.inbox.push(crate::chat::ChatInbound::SessionsList {
                sessions: session_list,
            });
        }
        return;
    }

    // chat.send response: may contain the agent reply directly
    let text = payload
        .get("text")
        .or_else(|| payload.get("message"))
        .or_else(|| payload.get("reply"))
        .and_then(Value::as_str);
    if let Some(text) = text {
        let agent_name = payload
            .get("agentName")
            .or_else(|| payload.get("agent"))
            .and_then(Value::as_str)
            .map(String::from);
        if let Ok(mut cs) = chat_state.lock() {
            cs.inbox.push(crate::chat::ChatInbound::Reply {
                text: text.to_string(),
                agent_name,
            });
        }
    }
}

fn node_status_from_event(connection_name: &str, event_name: &str, _payload: Option<&Value>) -> Option<GatewayEvent> {
    match event_name {
        "presence" => None,
        "tick" => None,
        "shutdown" => Some(GatewayEvent::Disconnected {
            connection_name: connection_name.to_string(),
            reason: "gateway shutdown".to_string(),
        }),
        "session.start" | "session.end" | "agent.error" => {
            debug!(event = event_name, "gateway subscription event received");
            None
        }
        _ => None,
    }
}

// is_node_presence removed — node status now determined via node.list API only

fn node_status_from_node_list(connection_name: &str, payload: Option<&Value>) -> Option<GatewayEvent> {
    let payload = payload?;
    // node.list response: payload is an array of nodes OR { nodes: [...] }
    let items = payload
        .as_array()
        .or_else(|| payload.get("nodes").and_then(Value::as_array))?;

    let node_online = items.iter().any(|n| {
        n.get("connected").and_then(Value::as_bool).unwrap_or(false)
    });

    // Extract displayName from the first connected node, or first node
    let node_name = items
        .iter()
        .find(|n| {
            n.get("connected")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .or_else(|| items.first())
        .and_then(|n| n.get("displayName").and_then(Value::as_str))
        .map(String::from);

    // Build stats from node.list data
    let active_sessions = items
        .iter()
        .filter(|n| n.get("connected").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|n| n.get("activeSessions").and_then(Value::as_u64))
        .sum::<u64>() as u32;

    let total_errors_24h = items
        .iter()
        .filter_map(|n| n.get("errors24h").and_then(Value::as_u64))
        .sum::<u64>() as u32;

    let last_agent_activity = items
        .iter()
        .filter(|n| n.get("connected").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|n| {
            let name = n.get("displayName").and_then(Value::as_str).unwrap_or("node");
            let last_active = n.get("lastActiveAt").and_then(Value::as_str)?;
            Some(format_agent_activity(name, last_active))
        })
        .next();

    let stats = GatewayStats {
        active_sessions,
        total_errors_24h,
        last_agent_activity,
    };

    Some(GatewayEvent::NodeStatus {
        connection_name: connection_name.to_string(),
        online: node_online,
        node_name,
        stats,
    })
}

fn format_agent_activity(name: &str, last_active_str: &str) -> String {
    // Try to parse ISO 8601 timestamp and compute "X ago"
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(last_active_str) {
        let now = chrono::Utc::now();
        let duration = now.signed_duration_since(dt);
        let ago = if duration.num_seconds() < 60 {
            "just now".to_string()
        } else if duration.num_minutes() < 60 {
            format!("{}m ago", duration.num_minutes())
        } else if duration.num_hours() < 24 {
            format!("{}h ago", duration.num_hours())
        } else {
            format!("{}d ago", duration.num_days())
        };
        format!("{name}: {ago}")
    } else {
        format!("{name}: {last_active_str}")
    }
}

fn parse_frame(text: &str) -> Result<Value, ConnectError> {
    serde_json::from_str::<Value>(text).map_err(|e| {
        debug!(?e, frame = text, "failed to parse gateway frame");
        ConnectError::Retryable(format!("invalid gateway frame: {e}"))
    })
}

fn frame_type(frame: &Value) -> Option<&str> {
    frame.get("type").and_then(Value::as_str)
}

fn frame_name(frame: &Value) -> Option<&str> {
    frame.get("event").and_then(Value::as_str)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn platform_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    }
}

fn private_key_to_pem(signing_key: &SigningKey) -> String {
    let mut der = Vec::with_capacity(PKCS8_PRIVATE_KEY_PREFIX.len() + 32);
    der.extend_from_slice(&PKCS8_PRIVATE_KEY_PREFIX);
    der.extend_from_slice(&signing_key.to_bytes());
    pem_wrap("PRIVATE KEY", &der)
}

fn public_key_to_pem(public_key: &VerifyingKey) -> String {
    let der = public_key_der(public_key);
    pem_wrap("PUBLIC KEY", &der)
}

fn public_key_der(public_key: &VerifyingKey) -> Vec<u8> {
    let mut der = Vec::with_capacity(SPKI_PUBLIC_KEY_PREFIX.len() + 32);
    der.extend_from_slice(&SPKI_PUBLIC_KEY_PREFIX);
    der.extend_from_slice(public_key.as_bytes());
    der
}

fn load_signing_key_from_pem(path: &Path) -> Result<SigningKey, String> {
    let pem =
        fs::read_to_string(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let der = pem_decode("PRIVATE KEY", &pem)?;
    if der.len() != PKCS8_PRIVATE_KEY_PREFIX.len() + 32
        || !der.starts_with(&PKCS8_PRIVATE_KEY_PREFIX)
    {
        return Err(format!(
            "invalid PKCS8 Ed25519 private key in {}",
            path.display()
        ));
    }

    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&der[PKCS8_PRIVATE_KEY_PREFIX.len()..]);
    Ok(SigningKey::from_bytes(&bytes))
}

fn pem_wrap(label: &str, der: &[u8]) -> String {
    let b64 = BASE64.encode(der);
    let mut out = String::new();
    out.push_str(&format!("-----BEGIN {label}-----\n"));

    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(&String::from_utf8_lossy(chunk));
        out.push('\n');
    }

    out.push_str(&format!("-----END {label}-----\n"));
    out
}

#[derive(Debug, Clone)]
pub struct ChatAttachment {
    pub data: String,
    pub filename: String,
    pub mime_type: String,
}

#[derive(Debug)]
pub enum GatewayCommand {
    SendChat {
        message: String,
        session_key: Option<String>,
        attachments: Option<Vec<ChatAttachment>>,
    },
    ListSessions,
}

#[derive(Debug, Clone)]
pub struct ChatSessionInfo {
    pub key: String,
    pub name: String,
}

fn pem_decode(label: &str, pem: &str) -> Result<Vec<u8>, String> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");

    let start = pem.find(&begin).ok_or_else(|| format!("missing {begin}"))? + begin.len();
    let finish = pem[start..]
        .find(&end)
        .map(|i| start + i)
        .ok_or_else(|| format!("missing {end}"))?;

    let b64: String = pem[start..finish]
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("");

    BASE64
        .decode(b64)
        .map_err(|e| format!("PEM decode failed: {e}"))
}
