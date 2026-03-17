use std::{
    fs,
    path::Path,
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
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info};
use uuid::Uuid;

use crate::config;

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
    pub url: String,
    pub token: Option<String>,
    pub device_id: String,
    pub public_key_pem: String,
    pub private_key: SigningKey,
    pub tx: mpsc::UnboundedSender<GatewayEvent>,
}

#[derive(Debug, Clone)]
pub enum GatewayEvent {
    Connected,
    Disconnected(String),
    NodeStatus { online: bool, node_id: String },
    Error(String),
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
    let der = public_key_der(public_key);
    let digest = Sha256::digest(der);
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

pub async fn spawn_if_configured(
    url: Option<String>,
    token: Option<String>,
    tx: mpsc::UnboundedSender<GatewayEvent>,
) -> bool {
    let Some(url) = url.filter(|v| !v.trim().is_empty()) else {
        return false;
    };

    let config_dir = match config::app_dir() {
        Ok(path) => path,
        Err(err) => {
            let _ = tx.send(GatewayEvent::Error(format!(
                "gateway config path error: {err}"
            )));
            return false;
        }
    };

    let (private_key, device_id, public_key_pem) = match load_or_create_keypair(&config_dir) {
        Ok(values) => values,
        Err(err) => {
            let _ = tx.send(GatewayEvent::Error(err));
            return false;
        }
    };

    let client = GatewayClient {
        url,
        token,
        device_id,
        public_key_pem,
        private_key,
        tx,
    };

    tokio::spawn(async move {
        connect_loop(client).await;
    });

    true
}

pub async fn connect_loop(client: GatewayClient) {
    let mut delay_secs = 1u64;

    loop {
        let result = connect_once(&client).await;

        match result {
            Ok(()) => {
                delay_secs = 1;
                let _ = client.tx.send(GatewayEvent::Disconnected(
                    "gateway disconnected".to_string(),
                ));
            }
            Err(ConnectError::Fatal(message)) => {
                let _ = client.tx.send(GatewayEvent::Error(message));
                break;
            }
            Err(ConnectError::Retryable(message)) => {
                let _ = client.tx.send(GatewayEvent::Disconnected(message));
            }
        }

        let jitter = rand::thread_rng().gen_range(0..=(delay_secs / 4).max(1));
        let sleep_secs = delay_secs.saturating_add(jitter).min(60);
        tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
        delay_secs = (delay_secs.saturating_mul(2)).min(60);
    }
}

async fn connect_once(client: &GatewayClient) -> Result<(), ConnectError> {
    info!(url = %client.url, "gateway connecting");
    let (stream, _) = connect_async(&client.url)
        .await
        .map_err(|e| ConnectError::Retryable(format!("connect failed: {e}")))?;

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
                    let frame = parse_frame(&text)?;
                    if frame_type(&frame) == Some("event")
                        && frame_name(&frame) == Some("connect.challenge")
                    {
                        return Ok(frame);
                    }
                }
                Message::Ping(payload) => {
                    write
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|e| ConnectError::Retryable(format!("pong failed: {e}")))?;
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
    let client_id = Uuid::new_v4().to_string();
    let platform = platform_name();
    let payload = build_signature_payload_v3(&SignatureParams {
        device_id: &client.device_id,
        client_id: &client_id,
        client_mode: "operator",
        role: "operator",
        scopes: "operator.read",
        signed_at_ms,
        token: &token,
        nonce,
        platform,
        device_family: "desktop",
    });

    let signature = sign_payload(&client.private_key, &payload);
    let connect_id = Uuid::new_v4().to_string();

    let connect_frame = json!({
        "type": "req",
        "id": connect_id,
        "method": "connect",
        "params": {
            "minProtocol": 3,
            "maxProtocol": 3,
            "client": {
                "id": client_id,
                "version": env!("CARGO_PKG_VERSION"),
                "platform": platform,
                "mode": "operator"
            },
            "role": "operator",
            "scopes": ["operator.read"],
            "caps": [],
            "commands": [],
            "permissions": {},
            "auth": { "token": token },
            "locale": "en-US",
            "userAgent": format!("openclaw-node-widget/{}", env!("CARGO_PKG_VERSION")),
            "device": {
                "id": &client.device_id,
                "publicKey": &client.public_key_pem,
                "signature": signature,
                "signedAt": signed_at_ms,
                "nonce": nonce
            }
        }
    });

    write
        .send(Message::Text(connect_frame.to_string().into()))
        .await
        .map_err(|e| ConnectError::Retryable(format!("connect request send failed: {e}")))?;

    loop {
        let Some(message) = read.next().await else {
            return Err(ConnectError::Retryable(
                "gateway closed during connect".to_string(),
            ));
        };

        let message =
            message.map_err(|e| ConnectError::Retryable(format!("gateway read failed: {e}")))?;

        match message {
            Message::Text(text) => {
                let frame = parse_frame(&text)?;
                if frame_type(&frame) == Some("res") {
                    let id = frame.get("id").and_then(Value::as_str).unwrap_or_default();
                    if id == connect_id {
                        let ok = frame.get("ok").and_then(Value::as_bool).unwrap_or(false);
                        if ok {
                            break;
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
                let reason = close_frame
                    .map(|f| f.reason.to_string())
                    .unwrap_or_else(|| "gateway closed".to_string());
                return Err(ConnectError::Retryable(reason));
            }
            _ => {}
        }
    }

    let _ = client.tx.send(GatewayEvent::Connected);

    let mut presence_ticker = tokio::time::interval(Duration::from_secs(15));
    let mut pending_presence: Option<String> = None;

    loop {
        tokio::select! {
            _ = presence_ticker.tick() => {
                let req_id = Uuid::new_v4().to_string();
                let frame = json!({
                    "type": "req",
                    "id": req_id,
                    "method": "system-presence",
                    "params": {}
                });
                write
                    .send(Message::Text(frame.to_string().into()))
                    .await
                    .map_err(|e| ConnectError::Retryable(format!("presence request send failed: {e}")))?;
                pending_presence = Some(req_id);
            }
            message = read.next() => {
                let Some(message) = message else {
                    return Err(ConnectError::Retryable("gateway closed".to_string()));
                };

                match message.map_err(|e| ConnectError::Retryable(format!("gateway read failed: {e}")))? {
                    Message::Text(text) => {
                        let frame = parse_frame(&text)?;
                        handle_frame(&client.tx, &frame, pending_presence.as_deref());

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
    tx: &mpsc::UnboundedSender<GatewayEvent>,
    frame: &Value,
    pending_presence: Option<&str>,
) {
    match frame_type(frame) {
        Some("event") => {
            if let Some(event_name) = frame_name(frame) {
                if let Some(event) = node_status_from_event(event_name, frame.get("payload")) {
                    let _ = tx.send(event);
                }
            }
        }
        Some("res") => {
            let ok = frame.get("ok").and_then(Value::as_bool).unwrap_or(false);
            let id = frame.get("id").and_then(Value::as_str);
            if ok && id.is_some() && id == pending_presence {
                if let Some(event) = node_status_from_presence(frame.get("payload")) {
                    let _ = tx.send(event);
                }
            }
        }
        _ => {}
    }
}

fn node_status_from_event(event_name: &str, payload: Option<&Value>) -> Option<GatewayEvent> {
    match event_name {
        "node.online" => Some(GatewayEvent::NodeStatus {
            online: true,
            node_id: payload
                .and_then(|v| v.get("nodeId"))
                .and_then(Value::as_str)
                .unwrap_or("node")
                .to_string(),
        }),
        "node.offline" => Some(GatewayEvent::NodeStatus {
            online: false,
            node_id: payload
                .and_then(|v| v.get("nodeId"))
                .and_then(Value::as_str)
                .unwrap_or("node")
                .to_string(),
        }),
        "node.status" => {
            let online = payload
                .and_then(|v| v.get("online"))
                .and_then(Value::as_bool)?;
            let node_id = payload
                .and_then(|v| v.get("nodeId"))
                .and_then(Value::as_str)
                .unwrap_or("node")
                .to_string();
            Some(GatewayEvent::NodeStatus { online, node_id })
        }
        _ => None,
    }
}

fn node_status_from_presence(payload: Option<&Value>) -> Option<GatewayEvent> {
    let payload = payload?;
    let devices = payload
        .get("devices")
        .or_else(|| payload.get("clients"))
        .or_else(|| payload.get("items"))?
        .as_array()?;

    for device in devices {
        let role = device
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mode = device
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or_default();

        if role == "node" || mode == "node" {
            let node_id = device
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| device.get("nodeId").and_then(Value::as_str))
                .unwrap_or("node")
                .to_string();
            return Some(GatewayEvent::NodeStatus {
                online: true,
                node_id,
            });
        }
    }

    Some(GatewayEvent::NodeStatus {
        online: false,
        node_id: "node".to_string(),
    })
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
