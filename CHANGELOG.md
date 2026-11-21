# Changelog

All notable changes to EVE-Maj Rusty.

## 2.3.0 - 2026-11-21

On Linux, EVE clients can now be stacked full screen on one monitor, with hotkeys bringing the chosen one to the
front. Thumbnail regions can be moved to another display or redrawn without setting them up again, and F13 to F24
hotkeys now work on Linux.

**Windows install:** download `eve-maj-rusty-v2.3.0-windows-x86_64-portable.zip`, check it against its `.sha256` file,
and extract it to a folder you can write to (such as `Documents\EVE-Maj Rusty`, not Program Files). Run
`eve-maj-preview.exe`. If you already run 2.1.0 or later, you can update from the settings window's Updates tab
instead. The settings window, `config.exe`, needs the Microsoft Edge WebView2 Runtime, which Windows 11 includes and
Edge installs on Windows 10.

**Linux install:** download `eve-maj-rusty-v2.3.0-linux-x86_64.tar.gz`, check it against its `.sha256` file, then:

```sh
tar -xzf eve-maj-rusty-v2.3.0-linux-x86_64.tar.gz
install -Dm755 -t ~/.local/bin eve-maj-rusty-v2.3.0-linux-x86_64/eve-maj-{preview,config}
```

### Added
- Linux: **Stack Clients Full Screen** (Behavior > Window Position) floats every EVE client over a whole monitor,
  stacked on top of each other. Activating a client, with a hotkey, an account key binding or a thumbnail or Client List
  click, brings it to the front. This is how borderless full-screen clients already behave on Windows; on a tiling
  compositor such as Hyprland, the clients otherwise tile side by side. Pick the monitor, or leave each client on the
  one it's on. Characters excluded from auto-move are left alone.
- Every thumbnail region now has an edit button (✎), including whole-display ones. It can point the region at another
  display, or redraw it starting from where it is now. Only its placement changes: its grid, and the characters and
  accounts assigned to its cells, are kept. Before, a display region had to be removed and set up again.

### Fixed
- Linux: hotkeys on F13 to F24 now work. The standard keyboard map turns most of those keys into other keys (F22
  becomes "touchpad on", F20 "mic mute", Ctrl+F24 "touchpad toggle"), so a hotkey on them never fired. They are now
  bound by the physical key.

## 2.2.0 - 2026-11-21

Importing an EVE-Maj Preview profile export now brings over your accounts and account key bindings, not only the
profile's settings. It works the same on Linux and Windows.

**Windows install:** download `eve-maj-rusty-v2.2.0-windows-x86_64-portable.zip`, check it against its `.sha256` file,
and extract it to a folder you can write to (such as `Documents\EVE-Maj Rusty`, not Program Files). Run
`eve-maj-preview.exe`. If you already run 2.1.0, you can update from the settings window's Updates tab instead. The
settings window, `config.exe`, needs the Microsoft Edge WebView2 Runtime, which Windows 11 includes and Edge installs
on Windows 10.

**Linux install:** download `eve-maj-rusty-v2.2.0-linux-x86_64.tar.gz`, check it against its `.sha256` file, then:

```sh
tar -xzf eve-maj-rusty-v2.2.0-linux-x86_64.tar.gz
install -Dm755 -t ~/.local/bin eve-maj-rusty-v2.2.0-linux-x86_64/eve-maj-{preview,config}
```

### Added
- The Import dialog has two new sections for EVE-Maj Preview profile exports:
  - **Accounts & Characters:** your accounts, and which characters belong to each.
  - **Account Key Bindings:** the hotkeys that switch to an account's clients.
- Accounts are shared by all profiles, so they are saved straight away.
- An imported account with the same name as one you already have is merged into it, and its key bindings move with
  it.
- Importing the same file again updates what it imported, without duplicates.

### Under the hood
- CI now fails if a new commit carries an email address other than a GitHub no-reply one.

## 2.1.0 - 2026-11-19

The first Windows release of EVE-Maj Rusty: Windows now has every feature the Linux build has, including the settings
window. The Client List and History Panel are also redesigned on both platforms.

The Windows build is new and has had less testing than Linux. If something doesn't work, please
[open an issue](https://github.com/tylort69/eve-majRusty/issues/new/choose) and attach `eve-maj.log`, after removing
character names and anything else private.

**Windows install:** download `eve-maj-rusty-v2.1.0-windows-x86_64-portable.zip`, check it against its `.sha256` file,
and extract it to a folder you can write to (such as `Documents\EVE-Maj Rusty`, not Program Files). Run
`eve-maj-preview.exe`. The settings window, `config.exe`, needs the Microsoft Edge WebView2 Runtime, which Windows 11
includes and Edge installs on Windows 10. To keep your EVE-Maj Preview settings, copy its `profiles` folder into the
new one.

**Linux install:** download `eve-maj-rusty-v2.1.0-linux-x86_64.tar.gz`, check it against its `.sha256` file, then:

```sh
tar -xzf eve-maj-rusty-v2.1.0-linux-x86_64.tar.gz
install -Dm755 -t ~/.local/bin eve-maj-rusty-v2.1.0-linux-x86_64/eve-maj-{preview,config}
```

### Linux and Windows
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
- The portable zip is what the in-app updater installs, so later Windows releases can update in place.
  [docs/windows-testing.md](https://github.com/tylort69/eve-majRusty/blob/main/docs/windows-testing.md) is a
  checklist for trying it out.

### Under the hood
- Region selection, the settings window's backend and its theme colors are shared code used by both platforms.

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
