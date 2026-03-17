# OpenClaw Node Widget - Technical Specification

## 1. Overview & Motivation

**Type**: cross-platform system tray utility | **Rewrite**: AHK v1.4 → Rust

A lightweight system tray widget that monitors and controls individual OpenClaw Node instances. Unlike the full gateway dashboard, this tool focuses solely on Node process management with minimal resource footprint.

**Motivation**: The existing AHK v1.4 version is Windows-only and lacks the reliability needed for production use. A Rust rewrite provides:
- Cross-platform support (Windows/macOS/Linux)
- Single static binary with no runtime dependencies
- Better process management and error handling
- Modern configuration format (TOML vs INI)
- Proper WebSocket integration for real-time status

**Scope**: Each machine runs its own widget instance to manage its own Node. One widget = one Node.

**Deployment model**: Download binary → double-click → setup wizard asks Gateway URL + token → done.

---

## 2. Network Architecture

### Connection Model

```
┌──────────────┐    WebSocket (WS/WSS)    ┌──────────────┐
│  Widget      │ ◄──────────────────────► │  Gateway     │
│  (per machine)│    + operator token      │  (central)   │
└──────────────┘                          └──────────────┘
       │                                         │
       ▼                                         ▼
┌──────────────┐                          ┌──────────────┐
│  Node Process│                          │  Other Nodes │
│  (local)     │                          │  (remote)    │
└──────────────┘                          └──────────────┘
```

### Scenarios

| Scenario | Gateway URL | Transport | Token |
|----------|------------|-----------|-------|
| Same machine | `ws://localhost:3000` | Plain WS | Optional |
| LAN / Tailscale | `ws://100.x.x.x:3000` | Plain WS | Required |
| Public / Internet | `wss://gateway.example.com` | WSS (TLS) | Required |

### Connection Behavior

- **Startup**: Connect to Gateway WebSocket as operator role
- **Auth**: Send gateway token on handshake (read from config or auto-discovered from `~/.openclaw/openclaw.json`)
- **Heartbeat**: Gateway sends periodic pings; widget responds with pong
- **Reconnect**: On disconnect → exponential backoff (1s, 2s, 4s, 8s… max 60s)
- **Offline fallback**: If Gateway unreachable for 30s+ → fall back to local process detection (`sysinfo` crate, check for `openclaw node run` in command line)

### Status Detection Priority

1. **WebSocket**: Gateway reports Node registered + connected → Online
2. **Process scan**: `node.exe`/`node` process with `openclaw node run` in args → Online (degraded)
3. **Both fail** → Offline

---

## 3. BDD Scenarios

### First Launch

```gherkin
Scenario: First time setup
  Given the user downloads and runs the widget binary
  And no config.toml exists
  When the widget starts
  Then a setup wizard window appears
  And asks for Gateway URL (default: ws://localhost:3000)
  And asks for Gateway token (with "paste here" field)
  And has a "Test Connection" button
  When the user fills in valid values and clicks "Save"
  Then config.toml is created at ~/.config/openclaw-node-widget/config.toml
  And the widget minimizes to system tray
  And begins monitoring
```

### Normal Operation

```gherkin
Scenario: Node is running normally
  Given the widget is connected to Gateway
  And the Node is registered and online
  Then the tray icon shows green (online)
  And the tooltip shows "OpenClaw Node: Online"

Scenario: Node goes offline
  Given the widget detects Node offline
  When 3 consecutive checks fail (45 seconds)
  And auto-restart is enabled
  Then the widget restarts the Node silently
  And shows a notification "Node restarted"
  And the tray icon changes to green after Node comes back

Scenario: User manually stops Node
  Given the user right-clicks → "Stop Node"
  Then the Node process is killed
  And a 120-second cooldown starts
  And auto-restart is suppressed during cooldown
  And the tray icon shows red (offline)
```

### Network Failures

```gherkin
Scenario: Gateway connection lost
  Given the widget was connected to Gateway
  When the WebSocket connection drops
  Then the widget switches to process-scan fallback
  And the tooltip shows "OpenClaw Node: Online (no gateway)"
  And reconnection attempts start with exponential backoff

Scenario: Gateway unreachable on startup
  Given the Gateway URL is configured but not reachable
  When the widget starts
  Then it falls back to process-scan mode immediately
  And shows a notification "Gateway unreachable, using local detection"
  And retries Gateway connection every 60 seconds in background
```

### Settings

```gherkin
Scenario: Change settings via tray menu
  Given the user right-clicks → "Settings"
  Then the setup wizard window reopens with current values
  When the user changes Gateway URL and clicks "Save"
  Then config.toml is updated
  And the widget reconnects to the new Gateway
```

### Installation (per platform)

```gherkin
Scenario: Windows MSI install
  Given the user downloads the .msi file
  When they double-click and follow the wizard (Next → Next → Finish)
  Then the binary is installed to C:\Program Files\OpenClaw Node Widget\
  And a Start Menu shortcut is created
  And the widget launches automatically after install
  And the setup wizard appears (first run)

Scenario: Windows portable install
  Given the user downloads the .zip file
  When they extract and double-click openclaw-node-widget.exe
  Then the setup wizard appears
  And no system-level installation is needed

Scenario: macOS DMG install
  Given the user downloads the .dmg file
  When they open it and drag the app to Applications
  And launch it from Applications
  Then macOS may warn "unidentified developer" (unsigned build)
  When the user right-clicks → Open → confirms
  Then the setup wizard appears
  And no dock icon is shown (LSUIElement)

Scenario: Linux AppImage install
  Given the user downloads the .AppImage file
  When they run chmod +x and execute it
  Then the setup wizard appears
  And the tray icon appears (requires libappindicator)

Scenario: Linux deb install
  Given the user runs dpkg -i or apt install on the .deb file
  Then the binary is installed to /usr/bin/
  And a .desktop file is placed in /usr/share/applications/
  When the user launches from app menu or terminal
  Then the setup wizard appears

Scenario: Uninstall (all platforms)
  Given the widget is installed
  When the user uninstalls (Add/Remove Programs / trash .app / apt remove)
  Then the binary is removed
  And auto-start entries are cleaned up (Registry / launchd plist / XDG desktop)
  But config.toml is preserved (user data, not deleted)
```

### CLI Mode

```gherkin
Scenario: Interactive setup via terminal
  Given the user runs "openclaw-node-widget setup" in a terminal
  Then prompts appear for Gateway URL, token, auto-restart, auto-start
  When the user fills in values
  And the tool tests the connection and shows ✅ or ❌
  And the user confirms
  Then config.toml is written

Scenario: Daemon mode (headless server)
  Given the user runs "openclaw-node-widget daemon"
  Then no tray icon is created
  And the widget monitors Node via WebSocket + process scan
  And auto-restart works the same as GUI mode
  And logs go to stdout (or file if configured)
  And the process runs in foreground (user manages with systemd/screen/etc)

Scenario: One-shot status check
  Given the user runs "openclaw-node-widget status"
  Then it prints Node status, Gateway connection, PID, and uptime
  And exits with code 0 (online) or 1 (offline)

Scenario: CLI stop/restart
  Given the user runs "openclaw-node-widget stop"
  Then the Node process is killed
  And exit code 0 is returned
  Given the user runs "openclaw-node-widget restart"
  Then the Node is stopped then started silently
  And exit code 0 is returned after Node is confirmed running
```

### Authentication & Errors

```gherkin
Scenario: Wrong gateway token
  Given the user enters an invalid token in setup wizard
  When they click "Test Connection"
  Then the connection is rejected by Gateway
  And the wizard shows "❌ Authentication failed — check your token"
  And does not save config

Scenario: Gateway port occupied / unreachable
  Given the configured gateway URL points to a non-responsive port
  When the widget tries to connect
  Then after 5 seconds timeout it shows "❌ Cannot reach Gateway"
  And falls back to process-scan mode
  And retries in background with exponential backoff

Scenario: Node fails to start (crash loop)
  Given auto-restart is enabled
  And the Node process crashes immediately after starting
  When the widget detects 5 consecutive failed restarts
  Then auto-restart is paused
  And a notification shows "Node keeps crashing — auto-restart paused"
  And the tray tooltip shows "Node: Error (restart paused)"
  And the user can manually retry via right-click → "Restart Node"

Scenario: Insufficient permissions
  Given the widget tries to kill a Node process owned by another user/SYSTEM
  When TerminateProcess / kill fails with access denied
  Then a notification shows "Cannot stop Node — permission denied"
  And suggests running as administrator (Windows) or with sudo
```

### Edge Cases

```gherkin
Scenario: Duplicate widget instance
  Given the widget is already running
  When the user launches a second instance
  Then the second instance detects the first (via lock file or named mutex)
  And shows "Widget is already running" notification
  And brings the existing tray icon to focus (if possible)
  And the second instance exits

Scenario: Config file corrupted
  Given config.toml exists but contains invalid TOML
  When the widget starts
  Then it shows an error "Config file corrupted"
  And offers to "Reset to defaults" or "Open config file"
  And does not crash

Scenario: Node binary not found
  Given the configured node command is not in PATH
  When the user clicks "Restart Node"
  Then a notification shows "Cannot find 'openclaw' — is it installed?"
  And the tray icon stays red
```

### Auto-start

```gherkin
Scenario: Enable auto-start on Windows
  Given the user toggles "Auto-start on login" to ON
  Then a Registry key is created at HKCU\...\Run\OpenClawNodeWidget
  And the value points to the widget exe path
  When Windows starts and the user logs in
  Then the widget starts automatically in tray (no visible window)

Scenario: Enable auto-start on macOS
  Given the user toggles "Auto-start on login" to ON
  Then a LaunchAgent plist is created at ~/Library/LaunchAgents/com.openclaw.node-widget.plist
  When macOS starts and the user logs in
  Then the widget starts automatically (no dock icon)

Scenario: Enable auto-start on Linux
  Given the user toggles "Auto-start on login" to ON
  Then a .desktop file is created at ~/.config/autostart/openclaw-node-widget.desktop
  When the user logs into a desktop session
  Then the widget starts automatically in tray

Scenario: Disable auto-start
  Given auto-start is currently ON
  When the user toggles it OFF
  Then the platform-specific startup entry is removed
  And the widget no longer starts on login

Scenario: Widget moved after auto-start set
  Given auto-start points to /old/path/widget
  And the user moves the binary to /new/path/widget
  When the system tries to auto-start on login
  Then it fails silently (file not found)
  When the user manually launches from the new path
  Then the widget detects stale auto-start path
  And offers to update it
```

### Upgrade

```gherkin
Scenario: Upgrade preserves config
  Given widget v2.0 is installed with config.toml
  When the user installs v2.1 (MSI overwrite / brew upgrade / apt upgrade)
  Then the existing config.toml is preserved
  And the widget starts with the existing settings

Scenario: Config migration (new fields)
  Given widget v2.0 config.toml has no [log] section
  When the user upgrades to v2.1 which adds [log] section
  Then the widget uses default values for missing fields
  And does not error on unknown/missing sections
  And on next "Save" from Settings, the new fields are written

Scenario: Downgrade compatibility
  Given widget v2.1 config.toml has [log] section
  When the user downgrades to v2.0
  Then v2.0 ignores the unknown [log] section
  And loads the rest normally
```

---

## 4. Setup Wizard

A minimal native window (single panel, not multi-tab) shown on:
- First launch (no config found)
- Right-click → "Settings"

### Fields

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| Gateway URL | text input | `ws://localhost:3000` | Validates URL format |
| Gateway Token | password input | (empty) | "Paste from `openclaw status`" hint |
| Auto-restart | checkbox | ✅ on | |
| Auto-start on login | checkbox | ☐ off | |

### Buttons
- **Test Connection** — attempts WebSocket handshake, shows ✅/❌
- **Save** — writes config.toml, closes window, starts monitoring
- **Cancel** — exits app (first launch) or closes window (settings)

### Implementation
- Use `native-dialog` or `rfd` crate for simple cross-platform dialogs
- Or minimal `egui` window (adds ~1MB to binary but gives full control)
- Decision: defer to implementation phase, try `native-dialog` first

---

## 5. CLI Interface

The widget supports both GUI (default) and CLI modes.

### Usage

```bash
# GUI mode (default) — starts tray widget, shows setup wizard if no config
openclaw-node-widget

# CLI flags override config.toml (useful for scripting / one-off)
openclaw-node-widget --gateway ws://100.68.12.51:3000 --token abc123

# Setup wizard from CLI (writes config.toml interactively in terminal)
openclaw-node-widget setup
  Gateway URL [ws://localhost:3000]: ws://100.68.12.51:3000
  Gateway Token: ********
  Testing connection... ✅ Connected (Node: Online)
  Auto-restart? [Y/n]: Y
  Auto-start on login? [y/N]: y
  Config saved to ~/.config/openclaw-node-widget/config.toml

# Headless mode — no tray, just monitor + auto-restart (for servers / SSH)
openclaw-node-widget daemon

# One-shot status check (for scripts / health checks)
openclaw-node-widget status
  Node: Online (PID 12345)
  Gateway: Connected (ws://localhost:3000)
  Uptime: 3d 14h 22m

# Stop / restart node from CLI
openclaw-node-widget stop
openclaw-node-widget restart

# Show current config
openclaw-node-widget config
```

### Startup Logic

```
START
  ├─ Has CLI flags (--gateway/--token)? → Use flags, skip config
  ├─ Has config.toml? → Load config → Start tray widget
  └─ No config?
       ├─ Is TTY (terminal)? → Run interactive `setup` in terminal
       └─ Is GUI (double-click)? → Show setup wizard window
```

---

## 6. Features & Priority

### P0 - Core (Must Have)
- [ ] **Status indicator**: System tray icon showing Node online/offline state
- [ ] **Process monitoring**: Detect Node status via WebSocket connection to gateway
- [ ] **Manual controls**: Start/stop Node via right-click menu
- [ ] **Auto-restart**: Restart Node after N consecutive offline checks
- [ ] **Cross-platform**: Windows/macOS/Linux support with single binary

### P1 - Important (Should Have)
- [ ] **Auto-start**: Register with OS startup (registry/launchd/systemd)
- [ ] **Silent startup**: Spawn Node without visible terminal window
- [ ] **Configurable intervals**: Check interval (default 15s), restart threshold (default 3)
- [ ] **Process detection fallback**: When WebSocket unavailable, check for running Node process

### P2 - Nice to Have
- [ ] **Native notifications**: Desktop notifications on status change
- [ ] **Multi-node monitoring**: Support for multiple Node instances
- [ ] **Custom icons**: User-provided icon sets
- [ ] **Gateway token auto-discovery**: Read from OpenClaw config automatically
- [ ] **Dark mode icon variants**: Themed icons for dark/light modes
- [ ] **Stats overlay**: Right-click menu shows basic stats

## 7. Architecture

```
┌─────────────────────────────┐
│      Main Thread            │
│  ┌───────────────────────┐  │
│  │   Tray Event Loop     │◄─┼─┐            ┌────────────────────┐
│  │   (tray-icon crate)   │  │ │            │    Node Process    │
│  └───────────────────────┘  │ │            │ (openclaw node)  │
│            │                │ │            └────────────────────┘
│            ▼                │ │                    ▲        │
│  ┌───────────────────────┐ │ │                    │        │
│  │   Monitor Task        │ │ │                    │        │
│  │   (tokio runtime)     │ │ │             ┌──────┴────┐   │
│  │   - Gateway WS check  │ │ │             │  Gateway   │   │
│  │   - Process fallback  │ │ └───────────┤ WebSocket │   │
│  │   - Auto-restart      │ │             │  (status)  │   │
│  │   - Config reload     │ │             └────────────┘   │
│  └───────────────────────┘ │                                │
│            │                │                                │
│            ▼                │                                │
│  ┌───────────────────────┐ │                                │
│  │   Config (TOML)       │ │                                │
│  │   - gateway URL       │ │                                │
│  │   - autostart settings│ │                                │
│  │   - check interval    │ │                                │
│  └───────────────────────┘ │                                │
└─────────────────────────────┘                                │
                                                               │
┌─────────────────────────────┐                                │
│   Platform Services         │◄───────────────────────────────┘
│   - Windows Registry        │
│   - macOS Launchd         │
│   - Linux Systemd/XDG      │
└─────────────────────────────┘
```

**Threading Model**:
- **Main thread**: GTK/Cocoa/Win32 event loop (via tray-icon crate)
- **Background task**: Tokio runtime for WebSocket and periodic checks
- **Process spawning**: Platform-specific (CreateProcessW on Windows, fork/exec on *nix)

## 8. Configuration File Specification

**Location**: `~/.openclaw/config.toml` (reads automatically) or `~/.config/openclaw-node-widget/config.toml`

```toml
[gateway]
# WebSocket URL for OpenClaw Gateway
url = "ws://localhost:3000"
# Optional: gateway token (reads ~/.openclaw/config.toml if empty)
token = ""
# Timeout for WebSocket connection
connect_timeout_secs = 5

[node]
# Command to start Node
command = "openclaw node run"
# Optional: working directory (default: ~/.openclaw)
working_dir = ""
# Additional arguments passed to node
args = []
# Environment variables
env = { "DEBUG" = "openclaw:*" }

[widget]
# Check interval in seconds
check_interval_secs = 15
# Auto-restart on failure
auto_restart = true
restart_threshold = 3
restart_cooldown_secs = 120
max_restart_attempts = 5

[startup]
# Register with OS startup
auto_start = false
# Platform-specific paths (auto-detected if empty)
xdg_desktop_path = ""        # Linux: ~/.config/autostart/openclaw-node-widget.desktop
launchd_plist_path = ""     # macOS: ~/Library/LaunchAgents/com.openclaw.node-widget.plist
registry_key = ""           # Windows: HKCU\Software\Microsoft\Windows\CurrentVersion\Run

[appearance]
# Optional: custom icon paths (embedded PNG overrides this)
online_icon = ""
offline_icon = ""
unknown_icon = ""
# Tray tooltip format (status variables: {status}, {pid})
tooltip_format = "OpenClaw Node: {status}"

# Advanced
[log]
level = "info"      # trace, debug, info, warn, error
file = ""          # Optional: log file path
syslog = false     # Use system logger on *nix
```

## 9. Platform-Specific Behavior

| Feature               | Windows              | macOS                | Linux (Gnome/KDE)    | Linux (headless)     |
|---------------------|---------------------|---------------------|---------------------|---------------------|
| **Tray Framework**  | winapi/shell32      | Cocoa NSStatusItem  | libappindicator     | libappindicator      |
| **Process Spawn**   | CreateProcessW      | posix_spawn         | fork + exec         | fork + exec          |
| **Auto-start**      | Registry key        | LaunchAgent plist   | XDG autostart       | systemd user service |
| **No-window flag**  | CREATE_NO_WINDOW    | LSBackgroundOnly      | setsid + nohup      | --                  |
| **Icon format**     | ICO/PNG             | PNG/ICNS            | PNG                 | PNG                  |
| **Path separator**  | \                    | /                   | /                   | /                   |
| **Process kill**    | TerminateProcess    | kill(pid, SIGTERM)  | kill(pid, SIGTERM)  | kill(pid, SIGTERM)   |

## 10. Repository Structure

```
openclaw-node-widget/
├── Cargo.toml                  # Core dependencies + platform targets
├── README.md                   # Quick setup guide
├── CONTRIBUTING.md             # Build/dev instructions
├── LICENSE                     # MIT
├── CHANGELOG.md               # Version history
├── config.example.toml         # Configuration template
├── justfile                    # Build commands (optimized for cross-compile)
├── build.rs                    # Embed icons + platform detection
├── src/
│   ├── main.rs                 # Entry point, argument parsing
│   ├── tray.rs                 # Cross-platform tray icon and menu
│   ├── monitor.rs              # Node status monitoring + auto-restart
│   ├── gateway.rs              # WebSocket client for gateway status
│   ├── process.rs              # Process start/stop/detection (impl per platform)
│   ├── autostart.rs            # OS startup registration
│   ├── config.rs               # TOML config + validation
│   └── error.rs                # Error types
├── assets/
│   ├── icon_online.png         # Green tray icon
│   ├── icon_offline.png        # Red tray icon
│   └── icon_unknown.png        # Gray tray icon
└── scripts/                    # Legacy AHK scripts (reference only)
    └── v1.4/
```

## 11. Build, Packaging & Release

### Dependencies (Cargo.toml)

| Crate | Purpose |
|-------|---------|
| `tray-icon` | Cross-platform system tray (Tauri team) |
| `tokio` | Async runtime |
| `tokio-tungstenite` | WebSocket client |
| `serde` + `toml` | Config deserialization |
| `sysinfo` | Process detection fallback |
| `clap` | CLI argument parsing |
| `tracing` | Structured logging |
| `dirs` | XDG/platform path resolution |

### Cross-compile

```bash
# Native
cargo build --release

# Cross-compile (requires `cross` or platform SDK)
cross build --release --target x86_64-pc-windows-gnu
cross build --release --target x86_64-unknown-linux-gnu
cross build --release --target aarch64-apple-darwin
```

### Platform Packaging

Each platform gets a "download and run" experience:

| Platform | Package Format | Contents | Install Experience |
|----------|---------------|----------|-------------------|
| **Windows** | `.msi` installer + portable `.zip` | `openclaw-node-widget.exe` | MSI: double-click → Next → Finish (adds to Start Menu + optional auto-start). ZIP: extract anywhere, double-click exe |
| **macOS** | `.dmg` + portable binary | `OpenClaw Node Widget.app` | DMG: drag to Applications. Binary: `brew install --cask openclaw-node-widget` (future) |
| **Linux** | `.deb` + `.rpm` + `.AppImage` + portable binary | `openclaw-node-widget` | AppImage: download, chmod +x, run. Deb/RPM: package manager install. Adds .desktop file |

#### Windows MSI
- Built with `cargo-wix` (WiX Toolset)
- Installs to `C:\Program Files\OpenClaw Node Widget\`
- Creates Start Menu shortcut
- Optional: add to startup (checkbox in installer)
- Uninstall via Add/Remove Programs

#### macOS DMG
- Built with `create-dmg` in CI
- `.app` bundle with `Info.plist` (LSUIElement=true for no dock icon)
- Code-signed + notarized (requires Apple Developer account, can defer)
- Unsigned build works with right-click → Open

#### Linux
- **AppImage**: universal, no install needed — `chmod +x` and run
- **Deb/Rpm**: built with `cargo-deb` / `cargo-generate-rpm`
- Installs to `/usr/bin/`, adds XDG autostart `.desktop` file
- Depends on `libappindicator3` (most distros have it)

### GitHub Actions CI

- On push to `main`: build + test all 3 platforms
- On tag `v*`: build → package → create GitHub Release with all artifacts
- Matrix: `windows-latest`, `macos-latest`, `ubuntu-latest`
- Release artifacts:
  - `openclaw-node-widget-v{version}-windows-x64.msi`
  - `openclaw-node-widget-v{version}-windows-x64.zip`
  - `openclaw-node-widget-v{version}-macos-arm64.dmg`
  - `openclaw-node-widget-v{version}-macos-x64.dmg`
  - `openclaw-node-widget-v{version}-linux-x64.AppImage`
  - `openclaw-node-widget-v{version}-linux-x64.deb`
  - `openclaw-node-widget-v{version}-linux-x64.rpm`

## 12. Migration Path from AHK v1.4

| AHK v1.4 | Rust v2.0 |
|-----------|-----------|
| `ProcessExist("node.exe")` | WebSocket status check (primary) + `sysinfo` process scan (fallback) |
| `A_ComSpec /c taskkill` | `TerminateProcess` / `kill(SIGTERM)` |
| `wscript.exe node-hidden.vbs` | `CreateProcessW(CREATE_NO_WINDOW)` / `posix_spawn` |
| Registry `HKCU\...\Run` | Platform-specific autostart module |
| Hardcoded paths | `config.toml` + `dirs` crate |
| `SetTimer` with negative ms | Tokio interval timer |
| ICO from System.Drawing | Embedded PNG via `include_bytes!` |

**Transition**: Ship Rust v2.0 alongside AHK v1.4. AHK scripts moved to `scripts/v1.4/` for reference. No migration tool needed — just replace the binary and create `config.toml`.

## 13. Future Ideas (P2+)

- **Multi-node dashboard**: Monitor multiple nodes from one widget (remote gateways)
- **Desktop notifications**: Toast/banner on status change (online→offline, restart events)
- **Gateway token auto-discovery**: Read `~/.openclaw/openclaw.json` to extract gateway token
- **Log viewer**: Right-click → "View Logs" opens tail of node output
- **Update checker**: Notify when new release available on GitHub
- **Phone companion**: Pair with mobile app for remote monitoring
- **CLI mode**: `openclaw-node-widget --status` for headless/scripting use
