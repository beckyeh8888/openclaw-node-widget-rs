# Phase 4 Spec: GUI Setup Wizard + Node Installation

## Overview
Replace the CLI-based setup wizard with a native GUI wizard using `egui` (via `eframe`).
The wizard runs on first launch (no config found) or when user selects "Setup Wizard" from tray menu.

## Dependencies to Add
```toml
eframe = "0.31"  # egui framework with native window
```

## Architecture

### New file: `src/wizard.rs`
GUI wizard with these pages/steps:

#### Step 1: Welcome
- "Welcome to OpenClaw Node Widget"
- "This wizard will help you set up your OpenClaw Node connection."
- [Next] button

#### Step 2: Detect / Install Node
- Auto-detect: look for `node.cmd` / `node.sh` in `~/.openclaw/`
- If found: show path, show detected gateway URL/port from parsing the script
- If NOT found: 
  - Check if `npm` is available (run `npm --version`)
  - If npm available: offer to run `npm install -g openclaw` then `openclaw node setup`
  - If npm NOT available: show message "Please install Node.js first" with link to nodejs.org
- [Back] [Next] buttons

#### Step 3: Gateway Configuration  
- Pre-fill with detected values from Step 2
- Fields:
  - Gateway Host (text input, e.g. "100.104.6.121")
  - Gateway Port (text input, e.g. "18789") 
  - Gateway Token (text input, optional)
- Node command (text input, pre-filled with detected path)
- [Back] [Next] buttons

#### Step 4: Autostart
- Checkbox: "Start widget on login" (default: checked)
- [Back] [Finish] button

#### Step 5: Complete
- "Setup complete! The widget will now start monitoring your node."
- [Done] button → close wizard, start tray icon

### Changes to `src/main.rs`
- If no config exists AND not running `--daemon` or `--status`: launch wizard
- After wizard completes, save config and continue to tray mode
- Add "Setup Wizard" menu item to tray right-click menu

### Changes to `src/tray.rs`
- Add `TrayCommand::SetupWizard` variant
- Add "Setup Wizard..." menu item between Settings and Quit

### Changes to `src/setup.rs`
- Keep the detection logic (find_node_script, parse_node_script)
- Make functions pub so wizard.rs can use them
- Remove the CLI stdin/stdout wizard code

## GUI Style
- Simple, clean, native-looking
- Window size: ~500x400 pixels
- Use egui's built-in widgets (no custom rendering)
- Dark mode by default (egui default)

## Platform Notes
- egui/eframe works on Windows, macOS, and Linux
- No additional system dependencies needed
- Windows: `windows_subsystem = "windows"` already set (no console flash)

## Error Handling
- If npm install fails: show error in wizard, let user retry or skip
- If config write fails: show error dialog
- Network timeouts: 10s for npm operations

## Testing Checklist
1. [ ] First launch with no config → wizard appears
2. [ ] Wizard detects existing node.cmd
3. [ ] Gateway fields pre-filled correctly
4. [ ] Finish → config saved to correct path
5. [ ] Tray icon appears after wizard
6. [ ] "Setup Wizard" from tray menu re-opens wizard
7. [ ] Back/Next navigation works
8. [ ] Window closes cleanly on [X] or Cancel
9. [ ] Autostart checkbox works
10. [ ] No console window on Windows
