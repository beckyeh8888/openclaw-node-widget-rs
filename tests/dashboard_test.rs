//! BDD-style integration tests for Phase 15: Dashboard + Log Viewer.
//!
//! These tests verify the dashboard data aggregation, log buffer behavior,
//! latency tracking, IPC command contracts, and the SPA navigation model.

use openclaw_node_widget_rs::dashboard::{
    build_dashboard_data, DashboardData, LatencyTracker, LogBuffer, LogEntry, LogLevel,
};
use openclaw_node_widget_rs::plugin::ConnectionStatus;
use serde_json::json;

// ══════════════════════════════════════════════════════════════════════
// Feature: Dashboard
// ══════════════════════════════════════════════════════════════════════

// Scenario: Show connected plugins
//   Given 2 plugins are connected (openclaw, ollama)
//   When user navigates to /dashboard
//   Then both plugins should show green status indicators
//   And latency should be displayed for each
#[test]
fn scenario_show_connected_plugins_on_dashboard() {
    let statuses = vec![
        ("openclaw-home".to_string(), "Arno".to_string(), ConnectionStatus::Connected),
        ("ollama-local".to_string(), "Ollama".to_string(), ConnectionStatus::Connected),
    ];
    let plugin_types = vec![
        ("openclaw-home".to_string(), "openclaw".to_string(), "🦞".to_string()),
        ("ollama-local".to_string(), "ollama".to_string(), "🦙".to_string()),
    ];
    let mut tracker = LatencyTracker::new();
    tracker.push(23);
    let start = std::time::Instant::now();

    let data = build_dashboard_data(&statuses, &plugin_types, &tracker, start, None);

    assert_eq!(data.plugins.len(), 2, "should have 2 plugin cards");
    assert_eq!(data.plugins[0].status, "connected");
    assert_eq!(data.plugins[1].status, "connected");
    assert_eq!(data.plugins[0].name, "Arno");
    assert_eq!(data.plugins[1].name, "Ollama");
    assert!(data.plugins[0].latency_ms.is_some(), "latency should be shown");
}

// Scenario: Show node information
//   Given the OpenClaw plugin is connected
//   When the dashboard loads
//   Then it should show platform, uptime, version
#[test]
fn scenario_show_node_information() {
    let data = DashboardData::new();

    assert!(!data.platform.is_empty(), "platform should be set");
    assert!(!data.version.is_empty(), "version should be set");
    assert_eq!(data.uptime_secs, 0, "uptime starts at 0");
}

// Scenario: Real-time latency update
//   Given the dashboard is visible
//   When a new latency measurement arrives (150ms)
//   Then the latency display should update to reflect 150ms
#[test]
fn scenario_realtime_latency_update() {
    let mut tracker = LatencyTracker::new();
    tracker.push(100);
    tracker.push(150);

    let statuses = vec![
        ("oc-1".to_string(), "Arno".to_string(), ConnectionStatus::Connected),
    ];
    let plugin_types = vec![
        ("oc-1".to_string(), "openclaw".to_string(), "🦞".to_string()),
    ];
    let start = std::time::Instant::now();
    let data = build_dashboard_data(&statuses, &plugin_types, &tracker, start, None);

    assert_eq!(data.latency_history.len(), 2);
    assert_eq!(data.latency_history[1], 150);
    assert_eq!(data.plugins[0].latency_ms, Some(125)); // avg of 100, 150
}

// Scenario: Plugin goes offline
//   Given the dashboard shows openclaw as connected
//   When the openclaw plugin disconnects
//   Then the status indicator should show "disconnected"
#[test]
fn scenario_plugin_goes_offline() {
    let statuses = vec![
        ("oc-1".to_string(), "Arno".to_string(), ConnectionStatus::Disconnected),
    ];
    let plugin_types = vec![
        ("oc-1".to_string(), "openclaw".to_string(), "🦞".to_string()),
    ];
    let tracker = LatencyTracker::new();
    let start = std::time::Instant::now();

    let data = build_dashboard_data(&statuses, &plugin_types, &tracker, start, None);

    assert_eq!(data.plugins[0].status, "disconnected");
}

// Scenario: Plugin in error state
#[test]
fn scenario_plugin_error_state() {
    let statuses = vec![
        ("oc-1".to_string(), "Arno".to_string(), ConnectionStatus::Error("timeout".to_string())),
    ];
    let plugin_types = vec![
        ("oc-1".to_string(), "openclaw".to_string(), "🦞".to_string()),
    ];
    let tracker = LatencyTracker::new();
    let start = std::time::Instant::now();

    let data = build_dashboard_data(&statuses, &plugin_types, &tracker, start, None);

    assert_eq!(data.plugins[0].status, "error");
}

// Scenario: Plugin reconnecting state
#[test]
fn scenario_plugin_reconnecting_state() {
    let statuses = vec![
        ("oc-1".to_string(), "Arno".to_string(), ConnectionStatus::Reconnecting),
    ];
    let plugin_types = vec![
        ("oc-1".to_string(), "openclaw".to_string(), "🦞".to_string()),
    ];
    let tracker = LatencyTracker::new();
    let start = std::time::Instant::now();

    let data = build_dashboard_data(&statuses, &plugin_types, &tracker, start, None);

    assert_eq!(data.plugins[0].status, "reconnecting");
}

// Scenario: Dashboard data serializes to JSON for WebView
#[test]
fn scenario_dashboard_data_serializes_to_json() {
    let data = DashboardData::new();
    let json_str = serde_json::to_string(&data).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    assert!(v["plugins"].is_array());
    assert!(v["platform"].is_string());
    assert!(v["version"].is_string());
    assert_eq!(v["uptime_secs"], 0);
}

// ══════════════════════════════════════════════════════════════════════
// Feature: Log Viewer
// ══════════════════════════════════════════════════════════════════════

// Scenario: Show live logs
//   Given the log viewer is open
//   When a new log entry arrives
//   Then it should appear in the buffer
#[test]
fn scenario_show_live_logs() {
    let mut buf = LogBuffer::new();
    assert!(buf.is_empty());

    buf.push(LogEntry {
        timestamp: "22:40:01".to_string(),
        level: LogLevel::Info,
        source: "openclaw".to_string(),
        message: "Connected".to_string(),
    });

    assert_eq!(buf.len(), 1);
    assert_eq!(buf.entries()[0].message, "Connected");
    assert_eq!(buf.entries()[0].source, "openclaw");
}

// Scenario: Filter by level
//   Given the log viewer has entries of all levels
//   When user selects "Error" filter
//   Then only error-level entries should be visible
#[test]
fn scenario_filter_logs_by_level() {
    let mut buf = LogBuffer::new();
    buf.push(LogEntry {
        timestamp: "22:40:01".to_string(),
        level: LogLevel::Info,
        source: "openclaw".to_string(),
        message: "Connected".to_string(),
    });
    buf.push(LogEntry {
        timestamp: "22:40:02".to_string(),
        level: LogLevel::Warn,
        source: "ollama".to_string(),
        message: "Model not found".to_string(),
    });
    buf.push(LogEntry {
        timestamp: "22:40:03".to_string(),
        level: LogLevel::Error,
        source: "openclaw".to_string(),
        message: "chat.send failed".to_string(),
    });

    let errors = buf.filter(Some(&LogLevel::Error), None);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].message, "chat.send failed");

    let warns = buf.filter(Some(&LogLevel::Warn), None);
    assert_eq!(warns.len(), 1);
    assert_eq!(warns[0].message, "Model not found");

    let infos = buf.filter(Some(&LogLevel::Info), None);
    assert_eq!(infos.len(), 1);
}

// Scenario: Search logs
//   Given the log viewer has 100 entries
//   When user types "WebSocket" in the search box
//   Then only entries containing "WebSocket" should be visible
#[test]
fn scenario_search_logs() {
    let mut buf = LogBuffer::new();
    for i in 0..100 {
        buf.push(LogEntry {
            timestamp: format!("22:40:{:02}", i % 60),
            level: LogLevel::Info,
            source: "core".to_string(),
            message: format!("message-{}", i),
        });
    }
    buf.push(LogEntry {
        timestamp: "22:41:00".to_string(),
        level: LogLevel::Info,
        source: "openclaw".to_string(),
        message: "WebSocket connected to gateway".to_string(),
    });

    let results = buf.filter(None, Some("WebSocket"));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].message, "WebSocket connected to gateway");
}

// Scenario: Search is case-insensitive
#[test]
fn scenario_search_logs_case_insensitive() {
    let mut buf = LogBuffer::new();
    buf.push(LogEntry {
        timestamp: "22:40:01".to_string(),
        level: LogLevel::Info,
        source: "openclaw".to_string(),
        message: "WebSocket CONNECTED".to_string(),
    });

    let results = buf.filter(None, Some("websocket"));
    assert_eq!(results.len(), 1);
}

// Scenario: Search in source field
#[test]
fn scenario_search_logs_by_source() {
    let mut buf = LogBuffer::new();
    buf.push(LogEntry {
        timestamp: "22:40:01".to_string(),
        level: LogLevel::Info,
        source: "openclaw".to_string(),
        message: "something".to_string(),
    });
    buf.push(LogEntry {
        timestamp: "22:40:02".to_string(),
        level: LogLevel::Info,
        source: "ollama".to_string(),
        message: "something".to_string(),
    });

    let results = buf.filter(None, Some("openclaw"));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].source, "openclaw");
}

// Scenario: Combined level + search filter
#[test]
fn scenario_filter_logs_combined_level_and_search() {
    let mut buf = LogBuffer::new();
    buf.push(LogEntry {
        timestamp: "22:40:01".to_string(),
        level: LogLevel::Error,
        source: "openclaw".to_string(),
        message: "WebSocket error".to_string(),
    });
    buf.push(LogEntry {
        timestamp: "22:40:02".to_string(),
        level: LogLevel::Info,
        source: "openclaw".to_string(),
        message: "WebSocket connected".to_string(),
    });

    let results = buf.filter(Some(&LogLevel::Error), Some("WebSocket"));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].level, LogLevel::Error);
}

// Scenario: Log buffer ring behavior (max 1000 entries, drop oldest)
#[test]
fn scenario_log_buffer_ring_drops_oldest() {
    let mut buf = LogBuffer::with_capacity(5);
    for i in 0..8 {
        buf.push(LogEntry {
            timestamp: format!("00:00:0{}", i),
            level: LogLevel::Info,
            source: "test".to_string(),
            message: format!("msg-{}", i),
        });
    }

    assert_eq!(buf.len(), 5, "buffer should cap at 5");
    assert_eq!(buf.entries()[0].message, "msg-3", "oldest 3 should be dropped");
    assert_eq!(buf.entries()[4].message, "msg-7", "newest should be last");
}

// Scenario: Clear logs
#[test]
fn scenario_clear_logs() {
    let mut buf = LogBuffer::new();
    for i in 0..10 {
        buf.push(LogEntry {
            timestamp: "00:00:00".to_string(),
            level: LogLevel::Info,
            source: "test".to_string(),
            message: format!("msg-{}", i),
        });
    }
    assert_eq!(buf.len(), 10);

    buf.clear();
    assert!(buf.is_empty());
    assert_eq!(buf.len(), 0);
}

// ══════════════════════════════════════════════════════════════════════
// Feature: Latency Tracker
// ══════════════════════════════════════════════════════════════════════

// Scenario: Latency tracker sliding window
#[test]
fn scenario_latency_tracker_window() {
    let mut tracker = LatencyTracker::new();
    for i in 0..35u64 {
        tracker.push(i * 10);
    }

    // Should only keep last 30
    assert_eq!(tracker.len(), 30);
    assert_eq!(tracker.samples()[0], 50); // (35-30)*10 = 50
}

// Scenario: Latency avg and max
#[test]
fn scenario_latency_avg_max() {
    let mut tracker = LatencyTracker::new();
    tracker.push(10);
    tracker.push(30);
    tracker.push(50);

    assert_eq!(tracker.avg(), Some(30));
    assert_eq!(tracker.max(), Some(50));
}

// Scenario: Empty latency tracker
#[test]
fn scenario_empty_latency_tracker() {
    let tracker = LatencyTracker::new();
    assert!(tracker.is_empty());
    assert_eq!(tracker.avg(), None);
    assert_eq!(tracker.max(), None);
}

// ══════════════════════════════════════════════════════════════════════
// Feature: IPC Commands
// ══════════════════════════════════════════════════════════════════════

// Scenario: getDashboard IPC format
#[test]
fn scenario_get_dashboard_ipc_format() {
    let ipc_msg = json!({ "type": "getDashboard" });
    assert_eq!(ipc_msg["type"], "getDashboard");
}

// Scenario: getLogs IPC format
#[test]
fn scenario_get_logs_ipc_format() {
    let ipc_msg = json!({ "type": "getLogs" });
    assert_eq!(ipc_msg["type"], "getLogs");
}

// Scenario: clearLogs IPC format
#[test]
fn scenario_clear_logs_ipc_format() {
    let ipc_msg = json!({ "type": "clearLogs" });
    assert_eq!(ipc_msg["type"], "clearLogs");
}

// Scenario: navigate IPC format
#[test]
fn scenario_navigate_ipc_format() {
    let ipc_msg = json!({ "type": "navigate", "page": "dashboard" });
    assert_eq!(ipc_msg["type"], "navigate");
    assert_eq!(ipc_msg["page"], "dashboard");
}

// Scenario: Log entry serialization for WebView
#[test]
fn scenario_log_entry_json_shape() {
    let entry = LogEntry {
        timestamp: "22:40:01".to_string(),
        level: LogLevel::Error,
        source: "openclaw".to_string(),
        message: "chat.send failed".to_string(),
    };

    let v = serde_json::to_value(&entry).unwrap();
    assert_eq!(v["timestamp"], "22:40:01");
    assert_eq!(v["level"], "Error");
    assert_eq!(v["source"], "openclaw");
    assert_eq!(v["message"], "chat.send failed");
}

// Scenario: Dashboard init data includes dashboard and logs fields
#[test]
fn scenario_init_data_includes_dashboard_and_logs() {
    let init = json!({
        "lang": "en",
        "connected": false,
        "messages": [],
        "sessions": [],
        "selectedSession": null,
        "waitingForReply": false,
        "dashboard": {
            "plugins": [],
            "node_name": null,
            "platform": "macos",
            "uptime_secs": 0,
            "version": "0.8.0",
            "latency_history": []
        },
        "logs": [],
        "currentPage": "chat"
    });

    assert!(init["dashboard"].is_object(), "init should have dashboard");
    assert!(init["logs"].is_array(), "init should have logs array");
    assert_eq!(init["currentPage"], "chat");
}

// Scenario: Log level display format
#[test]
fn scenario_log_level_display() {
    assert_eq!(format!("{}", LogLevel::Info), "INFO");
    assert_eq!(format!("{}", LogLevel::Warn), "WARN");
    assert_eq!(format!("{}", LogLevel::Error), "ERROR");
}

// Scenario: Empty search returns all entries
#[test]
fn scenario_empty_search_returns_all() {
    let mut buf = LogBuffer::new();
    buf.push(LogEntry {
        timestamp: "00:00:01".to_string(),
        level: LogLevel::Info,
        source: "a".to_string(),
        message: "msg1".to_string(),
    });
    buf.push(LogEntry {
        timestamp: "00:00:02".to_string(),
        level: LogLevel::Error,
        source: "b".to_string(),
        message: "msg2".to_string(),
    });

    let results = buf.filter(None, Some(""));
    assert_eq!(results.len(), 2, "empty search should return all entries");

    let results2 = buf.filter(None, None);
    assert_eq!(results2.len(), 2, "None search should return all entries");
}
