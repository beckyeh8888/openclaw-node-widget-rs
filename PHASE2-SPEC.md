# Phase 2 Spec — OpenClaw Node Widget

## Overview
Implement all Phase 2 features in one pass. The project is a Rust tray app at `/tmp/openclaw-node-widget-rs/`.
Current code: 1009 lines across 8 modules. `cargo build` must pass on both macOS and Windows.

## 1. Setup Wizard (First-Run Auto-Config)

### Behavior
- On `Commands::Run`: before entering tray loop, call `maybe_run_setup(&mut config)`.
- If `config_path()` does NOT exist (first run), run the setup wizard.
- Setup wizard is **CLI-based** (stdout/stdin), not GUI. Keep it simple.

### Setup Steps
1. Print welcome banner: "OpenClaw Node Widget - First-Time Setup"
2. **Auto-detect node.cmd**: Search these paths in order:
   - `~/.openclaw/node.cmd` (Windows) or `~/.openclaw/node.sh` (Unix)
   - If found, parse it to extract `--host` and `--port` values for gateway URL.
3. **Gateway URL**: If auto-detected, confirm with user. Otherwise ask:
   `Enter Gateway URL (e.g. ws://192.168.1.100:18789):`
4. **Gateway Token**: Try to extract from node.cmd (`OPENCLAW_GATEWAY_TOKEN=...`).
   If not found, ask: `Enter Gateway Token (leave blank if none):`
5. **Node command**: If node.cmd found, set command to `cmd.exe` with args `["/c", "<path-to-node.cmd>"]` on Windows,
   or the shell script path on Unix. Otherwise ask user for the command.
6. Write config via `config.save()` and print "Setup complete! Starting widget..."

### Code Location
- New file: `src/setup.rs`
- Called from `main.rs` in `run_with_tray()` before tray creation.

## 2. CLI Commands (Make them work)

### `status` subcommand
- Current: just calls `detect_node()`. 
- Add: also read config, show gateway URL, show auto_restart setting, show config path.
- Format:
  ```
  OpenClaw Node Widget v0.1.0
  Config: C:\Users\beck8\AppData\Roaming\openclaw-node-widget\config.toml
  Node: Online (PID 1234)
  Gateway: ws://100.104.6.121:18789
  Auto-restart: off
  Auto-start: off
  ```

### `setup` subcommand
- Force-run the setup wizard (even if config exists), then save.

### `config` subcommand
- Current: prints TOML. Good enough, keep it.

### `stop` subcommand
- Current: calls `process::stop_node()`. Good enough.

### `restart` subcommand  
- Current: stop then start. Should load config properly (it does). Good enough.

## 3. Autostart (Windows Registry + Unix)

### Windows
- Use `winreg` crate to read/write `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
- Key name: `OpenClawNodeWidget`
- Value: full path to current exe (`std::env::current_exe()`)
- `set_autostart(true)` → write registry key
- `set_autostart(false)` → delete registry key
- `is_autostart_enabled()` → check if key exists

### macOS
- Write/remove LaunchAgent plist at `~/Library/LaunchAgents/ai.openclaw.node-widget.plist`
- Plist content: standard launchd with `RunAtLoad=true`, pointing to current exe.

### Linux
- Write/remove `.desktop` file at `~/.config/autostart/openclaw-node-widget.desktop`

### Cargo.toml
- Add `winreg = "0.55"` under `[target.'cfg(windows)'.dependencies]`

## 4. Lock File (Single Instance)

### Behavior
- On startup (in `run()`, before anything else), try to create a lock file.
- Lock file path: `<config_dir>/openclaw-node-widget/widget.lock`
- Write PID to lock file.
- On startup, if lock file exists:
  - Read PID from it.
  - Check if that PID is still running (use `sysinfo`).
  - If running → print "Widget is already running (PID xxx)" and exit.
  - If not running → stale lock, delete and continue.
- On clean exit (after tray loop returns), delete lock file.
- Use `Drop` trait on a `LockGuard` struct for cleanup.

### Code Location
- New file: `src/lock.rs`

## 5. Crash Loop Protection

### Behavior
- Already partially in monitor.rs: `max_restart_attempts` (default 5).
- Enhance: if `restart_failures >= max_restart_attempts`, set a "crash loop" flag.
- When crash loop detected:
  - Update tray tooltip to "Node crash loop detected - auto-restart paused"
  - Log a warning.
  - Do NOT attempt more restarts until user manually clicks "Restart Node" in menu.
  - After successful manual restart, reset crash loop flag.
- Add `crash_loop_secs` config (default 300): if node has been offline for this long continuously, consider it crash-looped.

### Code Changes
- In `monitor.rs`: add crash loop state tracking.
- In `StatusUpdate`: add `crash_loop: bool` field.
- In `tray.rs`: show crash loop state in tooltip and status menu item.

## 6. Reconnect Logic (Process Monitor Improvements)

### Current behavior
- `check_interval_secs` ticker polls `detect_node()`.
- Good enough for Phase 2.

### Improvements
- After Stop command, set a cooldown (already exists: `restart_cooldown_secs`). 
- After manual stop, change tray tooltip to "Node stopped (manual)" instead of just "Offline".
- Track stop reason: `enum StopReason { Manual, CrashLoop, Unknown }`.

## 7. Desktop Notifications (Windows Toast)

### Behavior
- When node goes from Online → Offline: show notification "OpenClaw Node went offline"
- When node goes from Offline → Online: show notification "OpenClaw Node is online"  
- When crash loop detected: show notification "Node crash loop detected"
- On Windows: use `notify-rust` crate for toast notifications.
- On macOS/Linux: also `notify-rust`.
- Add config: `widget.notifications = true` (default true).

### Cargo.toml
- Add `notify-rust = "4"` to dependencies.

## 8. Code Cleanup

- Remove `gateway.rs` entirely (not used in Phase 1/2, will be rewritten in Phase 3).
- Remove `mod gateway;` from main.rs.
- Fix all warnings (unused imports, dead code).
- Remove the debug `tracing::info!` in `start_node` (or keep as `debug!`).

---

## Build Requirements
- `cargo build` must pass on macOS (the dev machine).
- `cargo build` must pass on Windows with `stable-x86_64-pc-windows-msvc`.
- Use `#[cfg(windows)]`, `#[cfg(target_os = "macos")]`, `#[cfg(target_os = "linux")]` for platform code.
- Do NOT add any GUI dependencies (no winit, no egui). Setup wizard is CLI only.

## Testing Checklist
After implementation, these should work:
1. First run without config → setup wizard runs, creates config
2. `./widget status` → shows node status + config info
3. `./widget setup` → re-runs setup wizard
4. Tray icon with right-click menu (all items work)
5. Stop Node → offline → Restart Node → online
6. Auto-restart toggle works
7. Auto-start toggle writes/removes autostart entry
8. Running two instances → second one exits with error
9. Node offline for too long → crash loop message
10. Notifications on state change
