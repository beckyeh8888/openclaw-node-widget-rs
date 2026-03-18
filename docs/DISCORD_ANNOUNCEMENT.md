# Discord Announcement — OpenClaw Node Widget v0.7.0

> Copy-paste the message below into Discord.

---

🚀 **OpenClaw Node Widget v0.7.0 — System Tray Monitor for Your OpenClaw Node**

A lightweight, cross-platform system tray widget to monitor and control your OpenClaw Node. No terminal, no CLI — just a tray icon.

**✨ Key Features:**
- 🟢 **Live Status** — Green = online, red = offline, at a glance
- 🔄 **One-Click Control** — Start, stop, restart your node from right-click menu
- 🌐 **Gateway WebSocket** — Real-time remote monitoring via WebSocket
- 🔒 **Tailscale Integration** — Auto-detects peers, secure remote access out of the box
- 📊 **Diagnostics** — Connection latency, error tracking, one-click copy diagnostics
- 🔔 **Smart Notifications** — Node status changes, Tailscale drops, update alerts
- ⬇️ **Auto-Update** — One-click download + auto-restart when new version available
- 🌍 **Multi-language** — English, 繁體中文, 简体中文
- 🖥️ **Multi-Node** — Monitor multiple Gateways from one widget
- 🧙 **Setup Wizard** — GUI wizard auto-detects your node config on first launch

**📦 Downloads:**
| Platform | File |
|----------|------|
| Windows | `.zip` + `.exe` installer |
| macOS (Apple Silicon) | `.dmg` |
| macOS (Intel) | `.dmg` |
| Linux | `.tar.gz` + `.deb` |

👉 **Download:** <https://github.com/beckyeh8888/openclaw-node-widget-rs/releases/latest>

**🔒 Remote Access:**
Install [Tailscale](https://tailscale.com/download) on both machines → the widget auto-detects your Gateway during setup. No port forwarding needed.

**🛠️ Build from source:**
```
git clone https://github.com/beckyeh8888/openclaw-node-widget-rs.git
cd openclaw-node-widget-rs
cargo build --release
```

Feedback & issues: <https://github.com/beckyeh8888/openclaw-node-widget-rs/issues>
