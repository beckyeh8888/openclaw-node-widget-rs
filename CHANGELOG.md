# Changelog

## [0.9.0] - 2026-03-20

### Added
- **WebView Chat UI**: Replaced egui chat with Telegram-style bubble interface (dark theme, CJK support)
- **Plugin Architecture**: Extensible AgentPlugin trait — add any AI backend via config
- **Ollama Plugin**: Connect to local Ollama instance for private AI chat
- **OpenAI-Compatible Plugin**: Works with OpenAI, LM Studio, vLLM, Together, Groq, and any OpenAI-compatible API
- **n8n Plugin**: Trigger n8n workflows as AI agents via webhook
- **Markdown Rendering**: Agent replies now render bold, italic, code blocks, lists, and links
- **Streaming Replies**: Messages appear token-by-token as they generate
- **Dashboard Page**: Real-time plugin status, connection latency, node info
- **Log Viewer Page**: Searchable, filterable live log stream
- **Settings Page**: Manage plugins and preferences from within the app
- **Chat History**: Conversations persist across restarts
- **Multi-Session**: Switch between different agent sessions per plugin
- **Image Upload**: Send images to agents via file picker or drag-and-drop
- **Improved Notifications**: Plugin disconnect/reconnect alerts

### Changed
- Chat window is now a full SPA (Chat, Dashboard, Logs, Settings sidebar)
- Plugin config format: `[[plugins]]` (auto-migrates from old `[[connections]]`)

### Fixed
- chat.send missing idempotencyKey (required by Gateway)
- chat.send missing sessionKey (required by Gateway)

## [0.8.0] - 2026-03-19

### Added
- **💬 Chat UI** — Talk to your AI agent directly from the system tray (egui window)
- **Agent selector** — Dropdown to switch between agents (main, divination, etc.)
- **Gateway chat protocol** — `chat.send`, `sessions.list`, `chat` event handling via WebSocket
- **Global hotkey** — Cmd+Shift+O (macOS) / Ctrl+Shift+O (Win/Linux) to open chat
- **Chat notifications** — Desktop notification when agent replies and chat window is closed

## [0.7.0] - 2026-03-19

### Added
- **Tailscale integration** — Setup Wizard auto-detects Tailscale peers, auto-fills Gateway URL
- **Tailscale status** — tray shows Tailscale connected/disconnected, warns if VPN drops
- **Connection latency** — WebSocket ping/pong monitoring, shows latency in tray + diagnostics
- **README Tailscale guide** — step-by-step remote access setup instructions

## [0.6.0] - 2026-03-19

### Added
- **Multi-Node support** — monitor multiple Gateways with `[[connections]]` config
- **Gateway Stats** — active sessions, error count (24h), last agent activity in tray
- **macOS .app bundle** — DMG packaging, proper notifications, drag to Applications
- **Windows installer** — NSIS installer with Start Menu shortcut + AUMID
- **Install to system** — `--install` flag or Setup Wizard option, installs to `%LOCALAPPDATA%`
- **macOS .app autostart** — launchd plist with `open -a` when running from .app bundle
- **Proper uninstall** — removes install dir, registry/plist/desktop, config, device keys
- **First-run notification** — "Right-click the tray icon to see options"
- **Settings multi-connection editor** — add/remove/edit connections in GUI

### Fixed
- NSIS installer MUI_ICON error (conditional .ico check)
- ICO auto-generation in CI from PNG

## [0.5.0] - 2026-03-19

### Added
- **Multi-level status** — GatewayDown, AuthFailed, Reconnecting states (not just Online/Offline)
- **Last Error display** — tray shows most recent error message
- **Last Connected time** — tray shows when gateway was last connected
- **Copy Diagnostics** — one-click copy system info to clipboard (token masked)
- **Log token masking** — gateway token never appears in plain text in logs
- **Config validation** — startup checks for invalid URL, empty token, missing paths
- **Download Update button** — appears when new version available, one-click install (Windows)

### Fixed
- View Logs falls back to .openclaw directory on Windows (no logs/ subfolder)
- Empty menu item placeholder removed

## [0.4.0] - 2026-03-18

### Added
- **GUI Settings window** — egui-based settings (Gateway URL/token, intervals, toggles)
- **Auto-update check** — startup + every 6h via GitHub Releases API (reqwest/rustls)
- **Multi-language** — English, 繁體中文, 简体中文 with auto-detection + manual selector
- **CJK font support** — loads system fonts for Chinese/Japanese in egui windows
- **Connection details** — tray shows Gateway version, Node name, uptime
- **Open Gateway UI** — opens browser to Gateway HTTP URL
- **View Logs** — opens OpenClaw log directory in file explorer
- **Check for Updates** — manual check from tray menu
- **Uninstall** — confirmation dialog, removes autostart + config
- **Windows native toast** — custom AUMID registration, shows "OpenClaw Node Widget"

### Changed
- Settings now opens GUI window instead of config.toml in editor
- All UI text internationalized via i18n module
- Notification sound toggle in config

## [0.3.0] - 2026-03-18

### Added
- **Gateway WebSocket integration** — real-time Node status via Gateway connection
- Device identity authentication (Ed25519 key pair, auto-generated)
- Node status polling via `node.list` API (30s interval)
- Exponential backoff reconnection (1s → 60s)

### Changed
- Node status now determined by Gateway `node.list` (single source of truth)
- Cleaner tray menu: `Node: Online` / `Node: Offline` / `Node: Stopped`
- Simplified tooltip: `OpenClaw Node: Online\nGateway: Connected`
- Reduced log verbosity — sensitive data (tokens) moved to debug level

### Fixed
- Device identity required for Gateway scopes (without it, scopes are cleared)
- Removed presence event parsing (only shows WS clients, not paired nodes)
- Removed snapshot presence parsing (caused status flicker on connect)
- Eliminated Online→Offline→Online status flicker during initial connection

## [0.2.0] - 2026-03-17

### Added
- **GUI Setup Wizard** — native egui window with 5-step guided setup
- Setup Wizard accessible from tray right-click menu
- Auto-detect `node.cmd`/`node.sh` and parse Gateway URL/token
- Built-in Node.js/npm detection with install prompt
- Autostart toggle in wizard

### Changed
- First launch without config now opens GUI wizard instead of CLI prompts
- Replaced CLI setup with GUI wizard throughout

## [0.1.0] - 2026-03-17

### Added
- System tray icon with status indicators (online/offline/unknown)
- Right-click menu: Refresh, Restart Node, Stop Node, Settings, Exit
- Process detection for OpenClaw Node
- Node start/stop/restart via system tray
- Auto-restart with crash loop protection
- Lock file to prevent multiple instances
- Autostart support (Windows registry, macOS LaunchAgent, Linux .desktop)
- Desktop notifications for status changes
- Settings opens config.toml in system editor
- No console window on Windows (windows_subsystem)
- Cross-platform: Windows, macOS (Intel + ARM), Linux
- GitHub Actions CI for all platforms
- Automated release builds with GitHub Releases
