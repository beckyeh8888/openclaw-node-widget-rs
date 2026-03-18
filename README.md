<p align="center">
  <img src="assets/icon_online.png" width="80" alt="OpenClaw Node Widget">
</p>

<h1 align="center">OpenClaw Node Widget</h1>

<p align="center">
  <strong>Lightweight system tray widget to monitor and control your OpenClaw Node</strong>
</p>

<p align="center">
  <a href="https://github.com/beckyeh8888/openclaw-node-widget-rs/actions"><img src="https://github.com/beckyeh8888/openclaw-node-widget-rs/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/beckyeh8888/openclaw-node-widget-rs/releases/latest"><img src="https://img.shields.io/github/v/release/beckyeh8888/openclaw-node-widget-rs" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/beckyeh8888/openclaw-node-widget-rs" alt="License"></a>
</p>

---

## ✨ Features

- 🟢 **Live Status** — System tray icon shows node status at a glance (green = online, red = offline)
- 🌐 **Gateway WebSocket** — Real-time remote monitoring via WebSocket connection to OpenClaw Gateway
- 🔄 **One-Click Control** — Start, stop, and restart your node from the right-click menu
- 🧙 **Setup Wizard** — GUI wizard auto-detects your node configuration on first launch
- ⚙️ **GUI Settings** — Opens GUI settings window to configure all options visually
- 🚀 **Autostart** — Optionally start on login (Windows, macOS, Linux)
- 🔔 **Native Notifications** — Desktop alerts when your node goes online or offline (native Windows toast)
- 🛡️ **Crash Protection** — Detects crash loops and pauses auto-restart
- 🔒 **Single Instance** — Lock file prevents multiple widgets from running
- 🔍 **Connection Details** — View gateway version, node name, and uptime at a glance
- 🆕 **Auto-Update Check** — Periodic check for new releases with one-click download
- 🌍 **Multi-language** — Supports English, Traditional Chinese (zh-TW), and Simplified Chinese (zh-CN)
- 🗑️ **Uninstall** — Clean removal of config, autostart entries, and shortcuts
- ⚡ **Lightweight** — ~9MB native binary, minimal resource usage

<!-- TODO: Add screenshot of tray menu with all features -->

## 📦 Installation

### Download (Recommended)

Grab the latest release for your platform:

| Platform | Download |
|----------|----------|
| Windows (x64) | [openclaw-node-widget-windows-x64.zip](https://github.com/beckyeh8888/openclaw-node-widget-rs/releases/latest) |
| macOS (Apple Silicon) | [openclaw-node-widget-macos-arm64.tar.gz](https://github.com/beckyeh8888/openclaw-node-widget-rs/releases/latest) |
| macOS (Intel) | [openclaw-node-widget-macos-x64.tar.gz](https://github.com/beckyeh8888/openclaw-node-widget-rs/releases/latest) |
| Linux (x64) | [openclaw-node-widget-linux-x64.tar.gz](https://github.com/beckyeh8888/openclaw-node-widget-rs/releases/latest) |
| Linux (deb) | [openclaw-node-widget-linux-x64.deb](https://github.com/beckyeh8888/openclaw-node-widget-rs/releases/latest) |

### Build from Source

```bash
git clone https://github.com/beckyeh8888/openclaw-node-widget-rs.git
cd openclaw-node-widget-rs
cargo build --release
```

## 🚀 Quick Start

1. **Download and run** the widget
2. **First launch** opens the Setup Wizard automatically
3. The wizard **auto-detects** your OpenClaw Node configuration
4. Click through the steps → **Done!**
5. The widget appears in your **system tray**

<!-- TODO: Add screenshot of setup wizard -->
<!-- TODO: Add screenshot of tray icon in system tray -->

## 🖱️ Tray Menu

Right-click the tray icon for:

| Action | Description |
|--------|-------------|
| **Refresh** | Re-check node status |
| **Restart Node** | Stop and restart the node process |
| **Stop Node** | Stop the node process |
| **Open Gateway UI** | Open the Gateway web interface in your browser |
| **View Logs** | Open the logs directory |
| **Auto-restart** | Toggle automatic restart on crash |
| **Auto-start** | Toggle start on login |
| **Settings** | Opens GUI settings window |
| **Setup Wizard** | Re-run the setup wizard |
| **Check for Updates** | Check GitHub for a newer release |
| **Connection Details** | Shows gateway version, node name, and uptime |
| **Uninstall** | Remove config, autostart entries, and exit |
| **Exit** | Quit the widget |

<!-- TODO: Add screenshot of tray menu -->

## ⚙️ Configuration

Config is stored at:
- **Windows:** `%APPDATA%\openclaw-node-widget\config.toml`
- **macOS:** `~/Library/Application Support/openclaw-node-widget/config.toml`
- **Linux:** `~/.config/openclaw-node-widget/config.toml`

```toml
[gateway]
url = "ws://100.104.6.121:18789"
token = ""

[node]
command = "cmd.exe"                    # Windows
args = ["/c", "C:\\Users\\you\\.openclaw\\node.cmd"]
working_dir = "C:\\Users\\you\\.openclaw"

[startup]
auto_start = true
```

## 🌍 Multi-language

The widget supports multiple languages:

| Language | Code |
|----------|------|
| English | `en` |
| Traditional Chinese | `zh-TW` |
| Simplified Chinese | `zh-CN` |

The language is auto-detected from your system locale. You can override it in **Settings** or by setting `language` in the config file under `[widget]`.

## ⚠️ Windows SmartScreen

Windows may show a SmartScreen warning because the binary isn't code-signed yet. Click **"More info" → "Run anyway"** to proceed. This is safe — you can verify the source code yourself.

## 🗺️ Roadmap

- [x] Phase 1: System tray + process detection
- [x] Phase 2: Setup wizard (CLI), autostart, crash protection
- [x] Phase 3: GitHub Actions CI + cross-platform releases
- [x] Phase 4: GUI Setup Wizard (egui)
- [x] Phase 5: Gateway WebSocket (remote monitoring)
- [x] Phase 6: Native notifications, auto-update, GUI settings, i18n
- [ ] Mobile: Companion widget for iOS/Android

## 📄 License

[MIT](LICENSE) © Beck Yeh

## 🔗 Links

- [OpenClaw](https://openclaw.ai) — The AI agent platform
- [OpenClaw Docs](https://docs.openclaw.ai)
- [OpenClaw Discord](https://discord.com/invite/clawd)
