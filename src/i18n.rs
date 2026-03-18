use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    ZhTw,
    ZhCn,
}

static LANG: OnceLock<Lang> = OnceLock::new();

pub fn init() {
    LANG.get_or_init(detect_lang);
}

pub fn current_lang() -> Lang {
    *LANG.get_or_init(detect_lang)
}

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

fn en<'a>(key: &'a str) -> &'a str {
    match key {
        // Tray menu
        "status_unknown" => "Unknown",
        "status_online" => "Online",
        "status_offline" => "Offline",
        "status_stopped" => "Stopped",
        "status_crash_loop" => "Crash Loop",
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
        "na" => "N/A",
        "open_gateway_ui" => "Open Gateway UI",
        "view_logs" => "View Logs",
        "settings" => "Settings",
        "setup_wizard" => "Setup Wizard...",
        "check_for_updates" => "Check for Updates",
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

        // Duration
        "hours_short" => "h",
        "minutes_short" => "m",
        "just_now" => "Just now",

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
        "na" => "無",
        "open_gateway_ui" => "開啟閘道介面",
        "view_logs" => "檢視日誌",
        "settings" => "設定",
        "setup_wizard" => "設定精靈...",
        "check_for_updates" => "檢查更新",
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

        "just_now" => "剛剛",

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
        "na" => "无",
        "open_gateway_ui" => "打开网关界面",
        "view_logs" => "查看日志",
        "settings" => "设置",
        "setup_wizard" => "设置向导...",
        "check_for_updates" => "检查更新",
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

        "just_now" => "刚刚",

        _ => return None,
    })
}
