# Phase 5 Spec: Gateway WebSocket Operator Connection

## Overview
Connect to the OpenClaw Gateway via WebSocket as an `operator` client. This enables:
- Real-time node status monitoring (no more process detection polling)
- Remote monitoring (widget on machine A, node on machine B)
- Proper integration with the OpenClaw ecosystem

## Protocol Summary (from docs.openclaw.ai/gateway/protocol)

### Transport
- WebSocket, text frames with JSON payloads
- First frame must be a `connect` request

### Handshake Flow
1. Client opens WebSocket to `ws://{host}:{port}`
2. Gateway sends `connect.challenge` event with `nonce` and `ts`
3. Client builds v3 signature payload, signs with Ed25519 private key
4. Client sends `connect` request with auth + device identity + signature
5. Gateway responds with `hello-ok` (success) or error

### Signature Payload (v3 format)
```
"v3|{deviceId}|{clientId}|{clientMode}|{role}|{scopes}|{signedAtMs}|{token}|{nonce}|{platform}|{deviceFamily}"
```
Fields joined by `|`. Sign this string with Ed25519 private key.

### Connect Request
```json
{
  "type": "req",
  "id": "<uuid>",
  "method": "connect",
  "params": {
    "minProtocol": 3,
    "maxProtocol": 3,
    "client": {
      "id": "node-widget",
      "version": "0.2.0",
      "platform": "<windows|macos|linux>",
      "mode": "operator"
    },
    "role": "operator",
    "scopes": ["operator.read"],
    "caps": [],
    "commands": [],
    "permissions": {},
    "auth": { "token": "<gateway_token>" },
    "locale": "en-US",
    "userAgent": "openclaw-node-widget/0.2.0",
    "device": {
      "id": "<fingerprint_of_public_key>",
      "publicKey": "<PEM_encoded_ed25519_public_key>",
      "signature": "<base64_signature>",
      "signedAt": <timestamp_ms>,
      "nonce": "<server_nonce>"
    }
  }
}
```

### Framing
- Request: `{"type":"req", "id":"...", "method":"...", "params":{...}}`
- Response: `{"type":"res", "id":"...", "ok":true|false, "payload":{...}}`
- Event: `{"type":"event", "event":"...", "payload":{...}}`

### Keepalive
- Gateway sends tick events based on `policy.tickIntervalMs` (typically 15s)
- Client should send pings/pongs as needed

## Dependencies to Add
```toml
tokio-tungstenite = { version = "0.26", features = ["native-tls"] }
tokio = { version = "1", features = ["full"] }
ed25519-dalek = { version = "2", features = ["rand_core"] }
rand = "0.8"
base64 = "0.22"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
hex = "0.4"
```

## Architecture

### New file: `src/gateway.rs` (~400 lines)

#### Structs
```rust
pub struct GatewayClient {
    url: String,
    token: Option<String>,
    device_id: String,
    public_key_pem: String,
    private_key: ed25519_dalek::SigningKey,
    tx: mpsc::UnboundedSender<GatewayEvent>,
}

pub enum GatewayEvent {
    Connected,
    Disconnected(String),
    NodeStatus { online: bool, node_id: String },
    Error(String),
}

pub enum NodeStatus {
    Online,
    Offline,
    Unknown,
}
```

#### Key Functions
```rust
/// Generate or load Ed25519 keypair from config directory
pub fn load_or_create_keypair(config_dir: &Path) -> Result<(SigningKey, String, String)>

/// Compute device ID from public key fingerprint (SHA-256 hex)
pub fn compute_device_id(public_key: &VerifyingKey) -> String

/// Build v3 signature payload string
pub fn build_signature_payload_v3(params: &SignatureParams) -> String

/// Sign the payload with Ed25519
pub fn sign_payload(key: &SigningKey, payload: &str) -> String

/// Main connection loop (runs in tokio task)
pub async fn connect_loop(client: GatewayClient) -> Result<()>
```

#### Connection Loop Logic
1. Connect WebSocket to `ws://{url}`
2. Wait for `connect.challenge` event
3. Extract `nonce` from challenge
4. Build v3 payload, sign it
5. Send `connect` request
6. Wait for `hello-ok` response
7. Enter event loop:
   - Listen for events (node status, etc.)
   - Handle disconnection → reconnect with backoff
   - Send presence queries periodically

#### Presence Polling
After connected, periodically call `system-presence` to check node status:
```json
{
  "type": "req",
  "id": "<uuid>",
  "method": "system-presence",
  "params": {}
}
```
Response contains list of connected devices with roles.

### Changes to `src/config.rs`
Add gateway fields:
```rust
#[derive(Deserialize, Serialize, Clone)]
pub struct GatewayConfig {
    pub url: Option<String>,      // ws://host:port
    pub token: Option<String>,    // gateway auth token
}
```

### Changes to `src/monitor.rs`
- Add gateway connection as primary status source
- Fall back to process detection if gateway not configured or disconnected
- Gateway events update tray icon in real-time

### Changes to `src/main.rs`
- Spawn tokio runtime for async gateway connection
- Route GatewayEvent through existing mpsc channel to tray

### Changes to `src/tray.rs`
- Handle GatewayEvent variants
- Update tooltip with gateway connection info

### Changes to `src/wizard.rs`
- Gateway URL/token from wizard now also configures WebSocket connection
- Wizard step 3 already collects these fields

## Reconnection Strategy
- Initial delay: 1 second
- Max delay: 60 seconds
- Exponential backoff with jitter
- Reset delay on successful connection
- Max retries: unlimited (always try to reconnect)

## Keypair Storage
- Store in `{config_dir}/openclaw-node-widget/device.key` (private key PEM)
- Store in `{config_dir}/openclaw-node-widget/device.pub` (public key PEM)
- Generate on first launch or when files are missing
- Device ID = SHA-256 hex of DER-encoded public key

## Error Handling
- Gateway unreachable → fall back to process detection, retry connection
- Auth failure → log error, show in tooltip, stop retrying (user needs to fix token)
- Challenge timeout (10s) → reconnect
- Unexpected disconnect → reconnect with backoff

## Platform Notes
- tokio runtime needed for async WebSocket
- Ed25519 via `ed25519-dalek` (pure Rust, no OpenSSL dependency)
- WebSocket via `tokio-tungstenite` (mature, well-maintained)

## Testing Checklist
1. [ ] Keypair generated on first launch
2. [ ] Keypair persisted and reloaded
3. [ ] WebSocket connects to gateway
4. [ ] Challenge-response handshake succeeds
5. [ ] hello-ok received
6. [ ] Node status reflected in tray icon
7. [ ] Disconnect → automatic reconnect
8. [ ] Auth failure → error in tooltip
9. [ ] No gateway config → falls back to process detection
10. [ ] Gateway + process detection coexist gracefully
