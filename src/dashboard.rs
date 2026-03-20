use std::collections::VecDeque;

use serde::Serialize;

use crate::plugin::ConnectionStatus;

const MAX_LOG_ENTRIES: usize = 1000;
const MAX_LATENCY_SAMPLES: usize = 30;

// ── Dashboard Data ──────────────────────────────────────────────────

/// Health check record for a single plugin.
#[derive(Debug, Clone, Serialize)]
pub struct PluginHealthRecord {
    pub reachable: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
    pub timestamp: String,
}

/// Aggregated status for a single plugin shown on the dashboard.
#[derive(Debug, Clone, Serialize)]
pub struct PluginStatusCard {
    pub id: String,
    pub name: String,
    pub plugin_type: String,
    pub icon: String,
    pub status: String,
    pub latency_ms: Option<u64>,
    pub uptime_secs: Option<u64>,
    pub health: Option<PluginHealthRecord>,
    pub uptime_pct: Option<f64>,
}

/// Full dashboard payload sent to the WebView.
#[derive(Debug, Clone, Serialize)]
pub struct DashboardData {
    pub plugins: Vec<PluginStatusCard>,
    pub node_name: Option<String>,
    pub platform: String,
    pub uptime_secs: u64,
    pub version: String,
    pub latency_history: Vec<u64>,
}

impl DashboardData {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            node_name: None,
            platform: std::env::consts::OS.to_string(),
            uptime_secs: 0,
            version: env!("CARGO_PKG_VERSION").to_string(),
            latency_history: Vec::new(),
        }
    }
}

// ── Log System ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Info => write!(f, "INFO"),
            Self::Warn => write!(f, "WARN"),
            Self::Error => write!(f, "ERROR"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub source: String,
    pub message: String,
}

/// Ring buffer of log entries with a configurable max size.
#[derive(Debug)]
pub struct LogBuffer {
    entries: VecDeque<LogEntry>,
    max_size: usize,
    next_id: u64,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            max_size: MAX_LOG_ENTRIES,
            next_id: 0,
        }
    }

    pub fn with_capacity(max_size: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(max_size.min(MAX_LOG_ENTRIES)),
            max_size,
            next_id: 0,
        }
    }

    pub fn push(&mut self, entry: LogEntry) {
        if self.entries.len() >= self.max_size {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
        self.next_id += 1;
    }

    pub fn entries(&self) -> &VecDeque<LogEntry> {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn max_size(&self) -> usize {
        self.max_size
    }

    /// Filter entries by level and/or text search.
    pub fn filter(&self, level: Option<&LogLevel>, search: Option<&str>) -> Vec<&LogEntry> {
        self.entries
            .iter()
            .filter(|e| {
                if let Some(lvl) = level {
                    if &e.level != lvl {
                        return false;
                    }
                }
                if let Some(q) = search {
                    if !q.is_empty()
                        && !e.message.to_lowercase().contains(&q.to_lowercase())
                        && !e.source.to_lowercase().contains(&q.to_lowercase())
                    {
                        return false;
                    }
                }
                true
            })
            .collect()
    }
}

/// Latency tracker that maintains a sliding window of samples.
pub struct LatencyTracker {
    samples: VecDeque<u64>,
    max_samples: usize,
}

impl LatencyTracker {
    pub fn new() -> Self {
        Self {
            samples: VecDeque::new(),
            max_samples: MAX_LATENCY_SAMPLES,
        }
    }

    pub fn push(&mut self, ms: u64) {
        if self.samples.len() >= self.max_samples {
            self.samples.pop_front();
        }
        self.samples.push_back(ms);
    }

    pub fn samples(&self) -> Vec<u64> {
        self.samples.iter().copied().collect()
    }

    pub fn avg(&self) -> Option<u64> {
        if self.samples.is_empty() {
            return None;
        }
        let sum: u64 = self.samples.iter().sum();
        Some(sum / self.samples.len() as u64)
    }

    pub fn max(&self) -> Option<u64> {
        self.samples.iter().copied().max()
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

/// Build a DashboardData snapshot from plugin statuses and latency tracker.
/// Tracks health check history per plugin for uptime calculation.
pub struct HealthTracker {
    /// (plugin_id → ring buffer of (reachable, timestamp))
    history: std::collections::HashMap<String, VecDeque<(bool, std::time::Instant)>>,
    /// Most recent health result per plugin.
    latest: std::collections::HashMap<String, crate::plugin::HealthStatus>,
}

impl HealthTracker {
    pub fn new() -> Self {
        Self {
            history: std::collections::HashMap::new(),
            latest: std::collections::HashMap::new(),
        }
    }

    /// Record a health check result.
    pub fn record(&mut self, plugin_id: &str, status: crate::plugin::HealthStatus) {
        let buf = self.history.entry(plugin_id.to_string()).or_insert_with(|| VecDeque::with_capacity(61));
        buf.push_back((status.reachable, std::time::Instant::now()));
        // Keep last ~60 samples (1 hour at 60s interval)
        while buf.len() > 60 {
            buf.pop_front();
        }
        self.latest.insert(plugin_id.to_string(), status);
    }

    /// Get uptime percentage for a plugin (last hour).
    pub fn uptime_pct(&self, plugin_id: &str) -> Option<f64> {
        let buf = self.history.get(plugin_id)?;
        if buf.is_empty() {
            return None;
        }
        let total = buf.len() as f64;
        let up = buf.iter().filter(|(ok, _)| *ok).count() as f64;
        Some((up / total) * 100.0)
    }

    /// Get the latest health record for a plugin.
    pub fn latest_record(&self, plugin_id: &str) -> Option<PluginHealthRecord> {
        self.latest.get(plugin_id).map(|h| PluginHealthRecord {
            reachable: h.reachable,
            latency_ms: h.latency_ms,
            error: h.error.clone(),
            timestamp: now_timestamp(),
        })
    }
}

pub fn build_dashboard_data(
    plugin_statuses: &[(String, String, ConnectionStatus)],
    plugin_types: &[(String, String, String)], // (id, plugin_type, icon)
    latency_tracker: &LatencyTracker,
    start_time: std::time::Instant,
    health_tracker: Option<&HealthTracker>,
) -> DashboardData {
    let mut data = DashboardData::new();
    data.uptime_secs = start_time.elapsed().as_secs();
    data.latency_history = latency_tracker.samples();

    for (id, name, status) in plugin_statuses {
        let (ptype, icon) = plugin_types
            .iter()
            .find(|(pid, _, _)| pid == id)
            .map(|(_, pt, ic)| (pt.clone(), ic.clone()))
            .unwrap_or_else(|| ("unknown".to_string(), "?".to_string()));

        let status_str = match status {
            ConnectionStatus::Connected => "connected",
            ConnectionStatus::Disconnected => "disconnected",
            ConnectionStatus::Reconnecting => "reconnecting",
            ConnectionStatus::Error(_) => "error",
        };

        let health = health_tracker.and_then(|ht| ht.latest_record(id));
        let uptime_pct = health_tracker.and_then(|ht| ht.uptime_pct(id));

        data.plugins.push(PluginStatusCard {
            id: id.clone(),
            name: name.clone(),
            plugin_type: ptype,
            icon,
            status: status_str.to_string(),
            latency_ms: latency_tracker.avg(),
            uptime_secs: Some(start_time.elapsed().as_secs()),
            health,
            uptime_pct,
        });
    }

    data
}

/// Format a timestamp for log entries using chrono.
pub fn now_timestamp() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_buffer_respects_max_size() {
        let mut buf = LogBuffer::with_capacity(3);
        for i in 0..5 {
            buf.push(LogEntry {
                timestamp: format!("00:00:0{i}"),
                level: LogLevel::Info,
                source: "test".to_string(),
                message: format!("msg-{i}"),
            });
        }
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.entries()[0].message, "msg-2");
        assert_eq!(buf.entries()[2].message, "msg-4");
    }

    #[test]
    fn log_buffer_filter_by_level() {
        let mut buf = LogBuffer::new();
        buf.push(LogEntry {
            timestamp: "00:00:01".to_string(),
            level: LogLevel::Info,
            source: "core".to_string(),
            message: "info msg".to_string(),
        });
        buf.push(LogEntry {
            timestamp: "00:00:02".to_string(),
            level: LogLevel::Error,
            source: "core".to_string(),
            message: "error msg".to_string(),
        });

        let errors = buf.filter(Some(&LogLevel::Error), None);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "error msg");
    }

    #[test]
    fn log_buffer_filter_by_search() {
        let mut buf = LogBuffer::new();
        buf.push(LogEntry {
            timestamp: "00:00:01".to_string(),
            level: LogLevel::Info,
            source: "openclaw".to_string(),
            message: "WebSocket connected".to_string(),
        });
        buf.push(LogEntry {
            timestamp: "00:00:02".to_string(),
            level: LogLevel::Info,
            source: "ollama".to_string(),
            message: "Model loaded".to_string(),
        });

        let results = buf.filter(None, Some("websocket"));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, "openclaw");
    }

    #[test]
    fn latency_tracker_sliding_window() {
        let mut tracker = LatencyTracker::new();
        for i in 0..35 {
            tracker.push(i);
        }
        assert_eq!(tracker.len(), MAX_LATENCY_SAMPLES);
        assert_eq!(tracker.samples()[0], 5); // oldest kept
    }

    #[test]
    fn latency_tracker_avg_and_max() {
        let mut tracker = LatencyTracker::new();
        tracker.push(10);
        tracker.push(20);
        tracker.push(30);
        assert_eq!(tracker.avg(), Some(20));
        assert_eq!(tracker.max(), Some(30));
    }
}
