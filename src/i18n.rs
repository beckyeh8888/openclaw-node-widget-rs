use std::sync::OnceLock;

/// Configure egui fonts to support CJK characters.
/// Call once in the first `update()` of each egui app.
pub fn setup_cjk_fonts(ctx: &eframe::egui::Context) {
    use eframe::egui::{FontData, FontDefinitions, FontFamily};

    let mut fonts = FontDefinitions::default();

    // Try platform-specific CJK fonts
    let cjk_data = load_system_cjk_font();

    if let Some(data) = cjk_data {
        fonts
            .font_data
            .insert("cjk".to_string(), std::sync::Arc::new(FontData::from_owned(data)));

        // Add CJK font as fallback for proportional and monospace
        fonts
            .families
            .get_mut(&FontFamily::Proportional)
            .unwrap()
            .push("cjk".to_string());
        fonts
            .families
            .get_mut(&FontFamily::Monospace)
            .unwrap()
            .push("cjk".to_string());

        ctx.set_fonts(fonts);
    }
}

fn load_system_cjk_font() -> Option<Vec<u8>> {
    // Windows: Microsoft JhengHei (微軟正黑體) or Microsoft YaHei (微软雅黑)
    #[cfg(windows)]
    {
        let paths = [
            "C:\\Windows\\Fonts\\msjh.ttc",     // 微軟正黑體
            "C:\\Windows\\Fonts\\msyh.ttc",     // 微软雅黑
            "C:\\Windows\\Fonts\\simsun.ttc",   // 宋体
        ];
        for path in &paths {
            if let Ok(data) = std::fs::read(path) {
                return Some(data);
            }
        }
    }

    // macOS: PingFang or Hiragino
    #[cfg(target_os = "macos")]
    {
        let paths = [
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
        ];
        for path in &paths {
            if let Ok(data) = std::fs::read(path) {
                return Some(data);
            }
        }
    }

    // Linux: Noto Sans CJK or WenQuanYi
    #[cfg(target_os = "linux")]
    {
        let paths = [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
        ];
        for path in &paths {
            if let Ok(data) = std::fs::read(path) {
                return Some(data);
            }
        }
    }

    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    ZhTw,
    ZhCn,
}

use std::sync::atomic::{AtomicU8, Ordering};

static LANG: OnceLock<Lang> = OnceLock::new();
static OVERRIDE: AtomicU8 = AtomicU8::new(0); // 0=none, 1=En, 2=ZhTw, 3=ZhCn

#[allow(dead_code)]
pub fn init() {
    LANG.get_or_init(detect_lang);
}

pub fn init_with_config(language: &str) {
    LANG.get_or_init(detect_lang);
    set_language(language);
}

pub fn set_language(lang_str: &str) {
    let val = match lang_str.to_lowercase().as_str() {
        "en" | "english" => 1,
        "zh-tw" | "zh_tw" | "繁體中文" => 2,
        "zh-cn" | "zh_cn" | "简体中文" => 3,
        _ => 0, // auto
    };
    OVERRIDE.store(val, Ordering::Relaxed);
}

pub fn current_lang() -> Lang {
    let ov = OVERRIDE.load(Ordering::Relaxed);
    match ov {
        1 => Lang::En,
        2 => Lang::ZhTw,
        3 => Lang::ZhCn,
        _ => *LANG.get_or_init(detect_lang),
    }
}

pub const LANGUAGE_OPTIONS: &[(&str, &str)] = &[
    ("auto", "Auto"),
    ("en", "English"),
    ("zh-tw", "繁體中文"),
    ("zh-cn", "简体中文"),
];

fn detect_lang() -> Lang {
    let locale = sys_locale::get_locale()
        .unwrap_or_default()
        .to_lowercase();
    if locale.starts_with("zh-tw")
        || locale.starts_with("zh_tw")
        || locale.contains("hant")
    {
        Lang::ZhTw
    } else if locale.starts_with("zh") {
        Lang::ZhCn
    } else {
        Lang::En
    }
}

pub fn t(key: &str) -> &str {
    let lang = current_lang();
    let result = match lang {
        Lang::En => None,
        Lang::ZhTw => zh_tw(key),
        Lang::ZhCn => zh_cn(key),
    };
    result.unwrap_or_else(|| en(key))
}

fn en(key: &str) -> &str {
    match key {
        // Tray menu
        "status_unknown" => "Unknown",
        "status_online" => "Online",
        "status_offline" => "Offline",
        "status_stopped" => "Stopped",
        "status_crash_loop" => "Crash Loop",
        "status_gateway_down" => "Gateway Down",
        "status_auth_failed" => "Auth Failed",
        "status_reconnecting" => "Reconnecting...",
        "status_checking" => "Checking...",
        "status_refreshing" => "Refreshing...",
        "refresh" => "Refresh",
        "restart_node" => "Restart Node",
        "stop_node" => "Stop Node",
        "auto_restart" => "Auto-restart",
        "auto_start" => "Auto-start",
        "connection_details" => "Connection",
        "gateway_version_label" => "Gateway: ",
        "node_name_label" => "Node: ",
        "uptime_label" => "Uptime: ",
        "last_error_label" => "Last Error: ",
        "last_connected_label" => "Last Connected: ",
        "none" => "None",
        "na" => "N/A",
        "open_gateway_ui" => "Open Gateway UI",
        "view_logs" => "View Logs",
        "settings" => "Settings",
        "setup_wizard" => "Setup Wizard...",
        "check_for_updates" => "Check for Updates",
        "no_updates" => "No Updates Available",
        "copy_diagnostics" => "Copy Diagnostics",
        "diagnostics_copied" => "Diagnostics copied to clipboard",
        "repair" => "Repair",
        "uninstall" => "Uninstall",
        "exit" => "Exit",

        // Notifications
        "app_name" => "OpenClaw Node Widget",
        "notif_node_offline" => "OpenClaw Node is now offline. The node process has stopped running.",
        "notif_node_online" => "OpenClaw Node is back online. The node process is running normally.",
        "notif_crash_loop" => "Node crash loop detected. Auto-restart has been paused due to repeated failures.",
        "notif_update_available" => "A new version is available:",
        "notif_up_to_date" => "You are running the latest version.",
        "notif_uninstalled" => "OpenClaw Node Widget has been uninstalled.",

        // Settings window
        "settings_title" => "OpenClaw Node Widget Settings",
        "gateway_url" => "Gateway URL",
        "gateway_token" => "Gateway Token",
        "check_interval" => "Check Interval (seconds)",
        "notifications" => "Notifications",
        "notification_sound" => "Notification Sound",
        "save" => "Save",
        "settings_saved" => "Settings saved successfully.",
        "close" => "Close",

        // Wizard
        "wizard_title" => "OpenClaw Node Widget Setup",
        "welcome" => "Welcome",
        "welcome_msg" => "Welcome to OpenClaw Node Widget.",
        "welcome_desc" => "This wizard will help you set up your OpenClaw Node connection.",
        "detect_install" => "Detect / Install Node",
        "gateway_config" => "Gateway Configuration",
        "gateway_host" => "Gateway Host",
        "gateway_port" => "Gateway Port",
        "gateway_token_optional" => "Gateway Token (optional)",
        "node_command" => "Node command",
        "autostart" => "Autostart",
        "start_on_login" => "Start widget on login",
        "complete" => "Complete",
        "complete_msg" => "Setup complete! The widget will now start monitoring your node.",
        "next" => "Next",
        "back" => "Back",
        "cancel" => "Cancel",
        "finish" => "Finish",
        "done" => "Done",

        // Detect/Install
        "npm_available" => "npm is available. You can install OpenClaw now.",
        "npm_not_found" => "npm not found. Please install Node.js first.",
        "install_openclaw" => "Install OpenClaw Node",
        "open_nodejs" => "Open nodejs.org",
        "redetect" => "Re-detect",
        "no_node_script" => "No node script found in ~/.openclaw",
        "found_node_script" => "Found node script: ",
        "detected_host" => "Detected host: ",
        "detected_port" => "Detected port: ",
        "detected_token" => "Detected token: ",
        "setup_completed" => "OpenClaw node setup completed.",

        // Uninstall dialog
        "confirm_uninstall" => "Confirm Uninstall",
        "uninstall_msg" => "Are you sure you want to uninstall?\nThis will remove all configuration files and autostart entries.",
        "yes_uninstall" => "Yes, Uninstall",

        // Tooltips
        "tooltip_node" => "OpenClaw Node: ",
        "tooltip_gateway" => "Gateway: ",
        "gateway_not_configured" => "Not configured",
        "gateway_connecting" => "Connecting...",
        "gateway_connected" => "Connected",
        "gateway_node_offline" => "Connected (node offline)",

        // Multi-connection
        "connections_label" => "Connections",
        "connection_name" => "Name",
        "add_connection" => "+ Add Connection",
        "remove" => "Remove",

        // Gateway Stats
        "stats_sessions" => "Sessions: ",
        "stats_errors_24h" => "Errors (24h): ",
        "stats_last_activity" => "Last Activity: ",

        // Duration
        "hours_short" => "h",
        "minutes_short" => "m",
        "just_now" => "Just now",

        // Tailscale
        "tailscale_peers_found" => "Tailscale peers detected — select a Gateway host:",
        "tailscale_manual_entry" => "Manual entry",
        "tailscale_hint" => "Tip: Install Tailscale for secure remote access to your Gateway",
        "tailscale_connected" => "Tailscale: Connected",
        "tailscale_disconnected" => "Tailscale: Disconnected",
        "tailscale_not_installed" => "Tailscale: Not installed",
        "tailscale_warning" => "Tailscale is down but Gateway uses a Tailscale IP",

        // Connection quality
        "latency_label" => "Latency: ",
        "latency_na" => "Latency: N/A",
        "latency_warning" => "High latency detected",

        // Chat
        "chat" => "Chat",
        "chat_send" => "Send",
        "chat_placeholder" => "Type a message...",
        "chat_empty" => "No messages yet",
        "chat_typing" => "Typing...",
        "chat_not_connected" => "Not connected",

        // Wave 7: Installer flow
        "install_nodejs" => "Install Node.js",
        "nodejs_required" => "Node.js is required but not installed.",
        "nodejs_install_win" => "Downloading and installing Node.js LTS...",
        "nodejs_install_mac" => "Please download and install Node.js from nodejs.org.",
        "nodejs_install_linux" => "Run: curl -fsSL https://deb.nodesource.com/setup_lts.x | sudo -E bash -",
        "installing" => "Installing...",
        "install_failed" => "Installation failed",
        "retry" => "Retry",

        // Wave 7: Tailscale step
        "tailscale_step_title" => "Tailscale (Optional)",
        "tailscale_optional_desc" => "Tailscale enables secure VPN access to your Gateway from anywhere.",
        "tailscale_install_btn" => "Install Tailscale",
        "tailscale_skip" => "Skip",
        "tailscale_disconnected_msg" => "Tailscale is installed but not connected. Please login first.",
        "tailscale_open_btn" => "Open Tailscale",
        "tailscale_connected_label" => "Tailscale: Connected",
        "tailscale_select_gateway" => "Select your Gateway machine:",

        // Wave 7: Connection test
        "test_connection" => "Test Connection",
        "connection_success" => "Connected",
        "connection_failed" => "Cannot connect to Gateway",
        "connection_failed_hint" => "Is the Gateway running?",

        // Wave 7: Pairing step
        "pairing_title" => "Pairing",
        "pairing_checking" => "Checking pairing status...",
        "pairing_waiting" => "Waiting for approval from Gateway admin...",
        "pairing_approved" => "Paired successfully!",
        "pairing_timeout" => "Pairing timed out. Please try again.",
        "pairing_already_paired" => "Already paired",

        _ => key,
    }
}

fn zh_tw(key: &str) -> Option<&'static str> {
    Some(match key {
        "status_unknown" => "未知",
        "status_online" => "線上",
        "status_offline" => "離線",
        "status_stopped" => "已停止",
        "status_crash_loop" => "崩潰循環",
        "status_gateway_down" => "閘道斷線",
        "status_auth_failed" => "驗證失敗",
        "status_reconnecting" => "重新連線中...",
        "status_checking" => "檢查中...",
        "status_refreshing" => "重新整理中...",
        "refresh" => "重新整理",
        "restart_node" => "重新啟動節點",
        "stop_node" => "停止節點",
        "auto_restart" => "自動重新啟動",
        "auto_start" => "開機自動啟動",
        "connection_details" => "連線",
        "gateway_version_label" => "閘道: ",
        "node_name_label" => "節點: ",
        "uptime_label" => "運行時間: ",
        "last_error_label" => "最後錯誤: ",
        "last_connected_label" => "最後連線: ",
        "none" => "無",
        "na" => "無",
        "open_gateway_ui" => "開啟閘道介面",
        "view_logs" => "檢視日誌",
        "settings" => "設定",
        "setup_wizard" => "設定精靈...",
        "check_for_updates" => "檢查更新",
        "no_updates" => "目前沒有更新",
        "copy_diagnostics" => "複製診斷資訊",
        "diagnostics_copied" => "診斷資訊已複製到剪貼簿",
        "repair" => "修復",
        "uninstall" => "解除安裝",
        "exit" => "結束",

        "app_name" => "OpenClaw 節點小工具",
        "notif_node_offline" => "OpenClaw 節點已離線。節點程序已停止運行。",
        "notif_node_online" => "OpenClaw 節點已恢復上線。節點程序正常運行中。",
        "notif_crash_loop" => "偵測到節點崩潰循環。自動重啟已暫停。",
        "notif_update_available" => "有新版本可用：",
        "notif_up_to_date" => "您正在使用最新版本。",
        "notif_uninstalled" => "OpenClaw 節點小工具已解除安裝。",

        "settings_title" => "OpenClaw 節點小工具設定",
        "gateway_url" => "閘道網址",
        "gateway_token" => "閘道令牌",
        "check_interval" => "檢查間隔（秒）",
        "notifications" => "通知",
        "notification_sound" => "通知音效",
        "save" => "儲存",
        "settings_saved" => "設定已儲存成功。",
        "close" => "關閉",

        "wizard_title" => "OpenClaw 節點小工具設定",
        "welcome" => "歡迎",
        "welcome_msg" => "歡迎使用 OpenClaw 節點小工具。",
        "welcome_desc" => "此精靈將協助您設定 OpenClaw 節點連線。",
        "detect_install" => "偵測 / 安裝節點",
        "gateway_config" => "閘道設定",
        "gateway_host" => "閘道主機",
        "gateway_port" => "閘道連接埠",
        "gateway_token_optional" => "閘道令牌（選填）",
        "node_command" => "節點指令",
        "autostart" => "自動啟動",
        "start_on_login" => "登入時啟動小工具",
        "complete" => "完成",
        "complete_msg" => "設定完成！小工具現在將開始監控您的節點。",
        "next" => "下一步",
        "back" => "上一步",
        "cancel" => "取消",
        "finish" => "完成",
        "done" => "完成",

        "npm_available" => "npm 可用。您可以立即安裝 OpenClaw。",
        "npm_not_found" => "找不到 npm。請先安裝 Node.js。",
        "install_openclaw" => "安裝 OpenClaw 節點",
        "open_nodejs" => "開啟 nodejs.org",
        "redetect" => "重新偵測",
        "no_node_script" => "在 ~/.openclaw 中未找到節點腳本",

        "confirm_uninstall" => "確認解除安裝",
        "uninstall_msg" => "您確定要解除安裝嗎？\n這將移除所有設定檔和自動啟動項目。",
        "yes_uninstall" => "是，解除安裝",

        "tooltip_node" => "OpenClaw 節點: ",
        "tooltip_gateway" => "閘道: ",
        "gateway_not_configured" => "未設定",
        "gateway_connecting" => "連線中...",
        "gateway_connected" => "已連線",
        "gateway_node_offline" => "已連線（節點離線）",

        "connections_label" => "連線",
        "connection_name" => "名稱",
        "add_connection" => "+ 新增連線",
        "remove" => "移除",

        "stats_sessions" => "工作階段: ",
        "stats_errors_24h" => "錯誤 (24h): ",
        "stats_last_activity" => "最後活動: ",

        "just_now" => "剛剛",

        "tailscale_peers_found" => "偵測到 Tailscale 節點 — 選擇閘道主機：",
        "tailscale_manual_entry" => "手動輸入",
        "tailscale_hint" => "提示：安裝 Tailscale 可安全遠端存取您的閘道",
        "tailscale_connected" => "Tailscale: 已連線",
        "tailscale_disconnected" => "Tailscale: 已斷線",
        "tailscale_not_installed" => "Tailscale: 未安裝",
        "tailscale_warning" => "Tailscale 已斷線，但閘道使用 Tailscale IP",

        "latency_label" => "延遲: ",
        "latency_na" => "延遲: 無",
        "latency_warning" => "偵測到高延遲",

        "chat" => "對話",
        "chat_send" => "發送",
        "chat_placeholder" => "輸入訊息...",
        "chat_empty" => "尚無訊息",
        "chat_typing" => "輸入中...",
        "chat_not_connected" => "未連線",

        // Wave 7
        "install_nodejs" => "安裝 Node.js",
        "nodejs_required" => "需要 Node.js 但尚未安裝。",
        "nodejs_install_win" => "正在下載並安裝 Node.js LTS...",
        "nodejs_install_mac" => "請從 nodejs.org 下載並安裝 Node.js。",
        "nodejs_install_linux" => "執行: curl -fsSL https://deb.nodesource.com/setup_lts.x | sudo -E bash -",
        "installing" => "安裝中...",
        "install_failed" => "安裝失敗",
        "retry" => "重試",
        "tailscale_step_title" => "Tailscale（選用）",
        "tailscale_optional_desc" => "Tailscale 可讓您從任何地方安全存取閘道。",
        "tailscale_install_btn" => "安裝 Tailscale",
        "tailscale_skip" => "略過",
        "tailscale_disconnected_msg" => "Tailscale 已安裝但未連線。請先登入。",
        "tailscale_open_btn" => "開啟 Tailscale",
        "tailscale_connected_label" => "Tailscale: 已連線",
        "tailscale_select_gateway" => "選擇您的閘道主機：",
        "test_connection" => "測試連線",
        "connection_success" => "已連線",
        "connection_failed" => "無法連線到閘道",
        "connection_failed_hint" => "閘道是否正在運行？",
        "pairing_title" => "配對",
        "pairing_checking" => "正在檢查配對狀態...",
        "pairing_waiting" => "等待閘道管理員核准...",
        "pairing_approved" => "配對成功！",
        "pairing_timeout" => "配對逾時。請重試。",
        "pairing_already_paired" => "已配對",

        _ => return None,
    })
}

fn zh_cn(key: &str) -> Option<&'static str> {
    Some(match key {
        "status_unknown" => "未知",
        "status_online" => "在线",
        "status_offline" => "离线",
        "status_stopped" => "已停止",
        "status_crash_loop" => "崩溃循环",
        "status_gateway_down" => "网关断开",
        "status_auth_failed" => "认证失败",
        "status_reconnecting" => "重新连接中...",
        "status_checking" => "检查中...",
        "status_refreshing" => "刷新中...",
        "refresh" => "刷新",
        "restart_node" => "重新启动节点",
        "stop_node" => "停止节点",
        "auto_restart" => "自动重启",
        "auto_start" => "开机自动启动",
        "connection_details" => "连接",
        "gateway_version_label" => "网关: ",
        "node_name_label" => "节点: ",
        "uptime_label" => "运行时间: ",
        "last_error_label" => "最后错误: ",
        "last_connected_label" => "最后连接: ",
        "none" => "无",
        "na" => "无",
        "open_gateway_ui" => "打开网关界面",
        "view_logs" => "查看日志",
        "settings" => "设置",
        "setup_wizard" => "设置向导...",
        "check_for_updates" => "检查更新",
        "no_updates" => "目前没有更新",
        "copy_diagnostics" => "复制诊断信息",
        "diagnostics_copied" => "诊断信息已复制到剪贴板",
        "repair" => "修复",
        "uninstall" => "卸载",
        "exit" => "退出",

        "app_name" => "OpenClaw 节点小工具",
        "notif_node_offline" => "OpenClaw 节点已离线。节点进程已停止运行。",
        "notif_node_online" => "OpenClaw 节点已恢复上线。节点进程正常运行中。",
        "notif_crash_loop" => "检测到节点崩溃循环。自动重启已暂停。",
        "notif_update_available" => "有新版本可用：",
        "notif_up_to_date" => "您正在使用最新版本。",
        "notif_uninstalled" => "OpenClaw 节点小工具已卸载。",

        "settings_title" => "OpenClaw 节点小工具设置",
        "gateway_url" => "网关地址",
        "gateway_token" => "网关令牌",
        "check_interval" => "检查间隔（秒）",
        "notifications" => "通知",
        "notification_sound" => "通知音效",
        "save" => "保存",
        "settings_saved" => "设置已保存成功。",
        "close" => "关闭",

        "wizard_title" => "OpenClaw 节点小工具设置",
        "welcome" => "欢迎",
        "welcome_msg" => "欢迎使用 OpenClaw 节点小工具。",
        "welcome_desc" => "此向导将帮助您设置 OpenClaw 节点连接。",
        "detect_install" => "检测 / 安装节点",
        "gateway_config" => "网关配置",
        "gateway_host" => "网关主机",
        "gateway_port" => "网关端口",
        "gateway_token_optional" => "网关令牌（可选）",
        "node_command" => "节点命令",
        "autostart" => "自动启动",
        "start_on_login" => "登录时启动小工具",
        "complete" => "完成",
        "complete_msg" => "设置完成！小工具现在将开始监控您的节点。",
        "next" => "下一步",
        "back" => "上一步",
        "cancel" => "取消",
        "finish" => "完成",
        "done" => "完成",

        "npm_available" => "npm 可用。您可以立即安装 OpenClaw。",
        "npm_not_found" => "找不到 npm。请先安装 Node.js。",
        "install_openclaw" => "安装 OpenClaw 节点",
        "open_nodejs" => "打开 nodejs.org",
        "redetect" => "重新检测",
        "no_node_script" => "在 ~/.openclaw 中未找到节点脚本",

        "confirm_uninstall" => "确认卸载",
        "uninstall_msg" => "您确定要卸载吗？\n这将删除所有配置文件和自动启动项。",
        "yes_uninstall" => "是，卸载",

        "tooltip_node" => "OpenClaw 节点: ",
        "tooltip_gateway" => "网关: ",
        "gateway_not_configured" => "未配置",
        "gateway_connecting" => "连接中...",
        "gateway_connected" => "已连接",
        "gateway_node_offline" => "已连接（节点离线）",

        "connections_label" => "连接",
        "connection_name" => "名称",
        "add_connection" => "+ 添加连接",
        "remove" => "移除",

        "stats_sessions" => "会话: ",
        "stats_errors_24h" => "错误 (24h): ",
        "stats_last_activity" => "最后活动: ",

        "just_now" => "刚刚",

        "tailscale_peers_found" => "检测到 Tailscale 节点 — 选择网关主机：",
        "tailscale_manual_entry" => "手动输入",
        "tailscale_hint" => "提示：安装 Tailscale 可安全远程访问您的网关",
        "tailscale_connected" => "Tailscale: 已连接",
        "tailscale_disconnected" => "Tailscale: 已断开",
        "tailscale_not_installed" => "Tailscale: 未安装",
        "tailscale_warning" => "Tailscale 已断开，但网关使用 Tailscale IP",

        "latency_label" => "延迟: ",
        "latency_na" => "延迟: 无",
        "latency_warning" => "检测到高延迟",

        "chat" => "对话",
        "chat_send" => "发送",
        "chat_placeholder" => "输入消息...",
        "chat_empty" => "暂无消息",
        "chat_typing" => "输入中...",
        "chat_not_connected" => "未连接",

        // Wave 7
        "install_nodejs" => "安装 Node.js",
        "nodejs_required" => "需要 Node.js 但尚未安装。",
        "nodejs_install_win" => "正在下载并安装 Node.js LTS...",
        "nodejs_install_mac" => "请从 nodejs.org 下载并安装 Node.js。",
        "nodejs_install_linux" => "运行: curl -fsSL https://deb.nodesource.com/setup_lts.x | sudo -E bash -",
        "installing" => "安装中...",
        "install_failed" => "安装失败",
        "retry" => "重试",
        "tailscale_step_title" => "Tailscale（可选）",
        "tailscale_optional_desc" => "Tailscale 可让您从任何地方安全访问网关。",
        "tailscale_install_btn" => "安装 Tailscale",
        "tailscale_skip" => "跳过",
        "tailscale_disconnected_msg" => "Tailscale 已安装但未连接。请先登录。",
        "tailscale_open_btn" => "打开 Tailscale",
        "tailscale_connected_label" => "Tailscale: 已连接",
        "tailscale_select_gateway" => "选择您的网关主机：",
        "test_connection" => "测试连接",
        "connection_success" => "已连接",
        "connection_failed" => "无法连接到网关",
        "connection_failed_hint" => "网关是否正在运行？",
        "pairing_title" => "配对",
        "pairing_checking" => "正在检查配对状态...",
        "pairing_waiting" => "等待网关管理员批准...",
        "pairing_approved" => "配对成功！",
        "pairing_timeout" => "配对超时。请重试。",
        "pairing_already_paired" => "已配对",

        _ => return None,
    })
}
