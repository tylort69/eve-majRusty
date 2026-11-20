# Changelog

All notable changes to EVE-Maj Rusty.

## Unreleased

### Linux
- The Client List and History Panel are redesigned to match the settings window: rounded Material 3 cards, a titled
  header with a client count, pill highlights for the focused and alerting clients, softer stat colors, and filter
  chips. They follow the theme picked in Preferences, and change with it while the app runs.

### Windows
The Windows build reaches feature parity with Linux, and releases now include a portable Windows zip.
- Hotkeys and hotkey groups: every binding in the profile and global settings, mouse-button and wheel hotkeys,
  suspend/resume, profile switching, app and URL hotkeys, and `evemajpreview://hotkey/...` links.
- Chat-log and game-log monitoring: system names, alerts with their sounds and text-to-speech, and Travel Mode.
- DPS, mining, bounty and CPU/RAM/VRAM overlays, with the Taking Damage, Laser Idle and Mining Stopped alerts.
- The Client List view and the notification History Panel, with dragging, snapping, saved positions and category
  filters.
- Region selection for layouts, from the settings window.
- A Rust `config.exe`: the same settings window as on Linux, in WebView2, with the in-place updater, Identify
  Displays and start-on-login.
- `scripts/package-windows.sh` builds `eve-maj-rusty-<version>-windows-x86_64-portable.zip`, and the release workflow
  attaches it to tagged releases. [docs/windows-testing.md](docs/windows-testing.md) is the test checklist.

### Shared
- The region-selection geometry and the settings window's backend now live in shared code used by both platforms;
  Linux behaviour is unchanged.

## 2.0.0 - 2026-11-19

First public release of EVE-Maj Rusty, a Rust rewrite of EVE-Maj Preview. The major version marks the new
codebase and home; profiles from EVE-Maj Preview load unchanged.

This release ships Linux (Hyprland) binaries. Windows users should keep using the original EVE-Maj Preview until the
remaining Windows features are ported.

**Requirements:** Hyprland, and the runtime libraries listed in the README (Wayland, EGL/GBM, cairo, pango, GTK 3,
WebKitGTK 4.1).

**Install:** download `eve-maj-rusty-v2.0.0-linux-x86_64.tar.gz`, check it against its `.sha256` file, then:

```sh
tar -xzf eve-maj-rusty-v2.0.0-linux-x86_64.tar.gz
install -Dm755 -t ~/.local/bin eve-maj-rusty-v2.0.0-linux-x86_64/eve-maj-{preview,config}
```

### Linux (Hyprland)
- Live GPU thumbnails of EVE clients as always-on-top overlays, with dragging, snapping, layouts and region fitting.
- Hotkeys and hotkey groups, minimize/close all, auto-move to saved positions, tray icon and `evemajpreview://` links.
- Chat-log and game-log monitoring: system names, over 30 alert types, sounds, text-to-speech and Travel Mode.
- DPS, mining, bounty and CPU/RAM overlays.
- Client List view and the notification History Panel.
- The settings window (`eve-maj-config`) with region selection, Identify Displays and update checks.
- Faction themes and custom themes for the settings window, and a new app logo.

### Windows
- Thumbnails, clicking, dragging, snapping, layouts, notifications, tray and profile switching. Hotkeys, chat-log
  features and the settings window are not ported yet.
