# Changelog

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
