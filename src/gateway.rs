use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, warn};

use crate::{config::Config, error::AppError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Connected,
    Disconnected,
}

#[derive(Debug, Clone)]
pub enum GatewayEvent {
    ConnectionState(ConnectionState),
    NodeOnline(bool),
}

pub fn spawn_gateway(config: Config, tx: mpsc::UnboundedSender<GatewayEvent>) {
    tokio::spawn(async move {
        if let Err(err) = run_gateway(config, tx).await {
            warn!("gateway loop stopped: {err}");
        }
    });
}

async fn run_gateway(
    config: Config,
    tx: mpsc::UnboundedSender<GatewayEvent>,
) -> Result<(), AppError> {
    let mut backoff = 1u64;

    loop {
        let _ = tx.send(GatewayEvent::ConnectionState(ConnectionState::Connecting));

        match connect_async(&config.gateway.url).await {
            Ok((stream, _)) => {
                backoff = 1;
                let _ = tx.send(GatewayEvent::ConnectionState(ConnectionState::Connected));

                let (mut write, mut read) = stream.split();
                if !config.gateway.token.is_empty() {
                    let auth = serde_json::json!({
                        "type": "auth",
                        "token": config.gateway.token,
                    })
                    .to_string();
                    let _ = write.send(Message::Text(auth)).await;
                }

                while let Some(msg) = read.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            if let Some(state) = parse_node_status(&text) {
                                let _ = tx.send(GatewayEvent::NodeOnline(state));
                            }
                        }
                        Ok(Message::Ping(payload)) => {
                            let _ = write.send(Message::Pong(payload)).await;
                        }
                        Ok(Message::Close(_)) => {
                            break;
                        }
                        Ok(_) => {}
                        Err(err) => {
                            warn!("gateway read error: {err}");
                            break;
                        }
                    }
                }

                let _ = tx.send(GatewayEvent::ConnectionState(ConnectionState::Disconnected));
            }
            Err(err) => {
                debug!("gateway connection failed: {err}");
                let _ = tx.send(GatewayEvent::ConnectionState(ConnectionState::Disconnected));
            }
        }

        tokio::time::sleep(Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(60);
    }
}

fn parse_node_status(text: &str) -> Option<bool> {
    let value: Value = serde_json::from_str(text).ok()?;

    if let Some(online) = value.get("node_online").and_then(Value::as_bool) {
        return Some(online);
    }

    if let Some(status) = value.get("status").and_then(Value::as_str) {
        return match status.to_lowercase().as_str() {
            "online" | "connected" => Some(true),
            "offline" | "disconnected" => Some(false),
            _ => None,
        };
    }

    value
        .get("node")
        .and_then(|n| n.get("online"))
        .and_then(Value::as_bool)
}
