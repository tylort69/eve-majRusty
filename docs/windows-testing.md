# Testing a Windows build

A checklist for trying a Windows build of EVE-Maj Rusty on a real PC. Each section says what to do and what you
should see. Everything the app does is written to `eve-maj.log` in the folder you extracted it to, so if something
doesn't work, note which step it was and send that file along.

The pilot names below ("Pilot Alpha", "Pilot Bravo") stand in for your own characters.

## Before you start

1. You need Windows 10 or 11 and at least two EVE clients you can log in.
2. Extract `eve-maj-rusty-<version>-windows-x86_64-portable.zip` into a new folder you can write to, such as
   `Documents\EVE-Maj Rusty` (not Program Files). You should have `eve-maj-preview.exe`, `config.exe`,
   `WebView2Loader.dll`, `README.md` and the two licence files.
3. If you already use EVE-Maj Preview, close it first (only one can run at a time). You can copy its `profiles`
   folder into the new folder to keep your settings.
4. Some settings below are on tabs that only show in Advanced Mode: in the settings, **Preferences** tab, turn on
   **Advanced Mode**.
5. For more detail in the log, set **System** tab → **Log Level** to **Debug** and save. A console window then shows
   the log live too.

## 1. Start-up and thumbnails

- Run `eve-maj-preview.exe` with your EVE clients open.
- **Expect:** a tray icon, and one thumbnail per client. Clicking a thumbnail brings that client forward.
  Right-dragging moves a thumbnail, and Ctrl+right-drag moves them all.
- **Log:** `EVE-Maj Preview v...` and `Registered N of M hotkey(s)`.

## 2. Settings window (config.exe)

- Right-click the tray icon and choose the settings entry.
- **Expect:** the settings window opens, shows your profile, and its theme matches what you picked before. Change a
  thumbnail colour: the thumbnails change live. Click **Save**, close, reopen: the change is still there.
- Close the window without saving after a change: the thumbnails go back.
- **If it shows an error about WebView2:** install the "Evergreen Bootstrapper" from
  https://developer.microsoft.com/microsoft-edge/webview2/ and try again.
- On the **Layout** tab, the displays section lists your monitors, and **Identify Displays** shows a number on each.
- Check the **Updates** tab: **Check for updates** answers without an error.

## 3. Hotkeys and hotkey groups

- In the settings, **Hotkey Groups** tab: create a group with Pilot Alpha and Pilot Bravo, give it a forward key (for example F1)
  and a backward key (F2), and save.
- **Expect:** F1 and F2 cycle between the two clients. With **Require EVE Focus** on, the keys do nothing while
  another app is in front.
- Bind a mouse side button (XButton1) to the group's forward key: it cycles too.
- Shift+click a thumbnail: it says "Excluded", and cycling skips that client. Shift+click again: "Included".
- Set a **Suspend hotkeys** key: pressing it shows "Hotkeys suspended" and the other keys stop working; pressing it
  again resumes them. The tray menu's **Suspend Hotkeys** does the same.
- Also try **Minimize all**, **Next/previous profile** and a per-character hotkey if you use them.
- While the settings window records a key (click a hotkey field), your hotkeys should not fire.
- **Log:** `Registered hotkey F1 ...`. If a key "is probably already in use by another program", another app (or a
  second binding in the same profile) has that key.

## 4. Chat logs, system names and alerts

- In the settings, **Log Monitoring** tab: turn on **Enable Log Monitoring**. The log folders should already point at
  `Documents\EVE\logs\Chatlogs` and `Gamelogs`; if EVE writes its logs elsewhere (for example OneDrive), browse to
  them. Save.
- **Expect:** each thumbnail shows its system name within a few seconds, and it updates when that pilot jumps.
- Trigger an alert you can safely cause, such as a fleet invite from one of your own pilots to another.
- **Expect:** the thumbnail flashes the alert. If that alert type has sound or text-to-speech on, you hear it.
- In the settings, **Notifications** tab: testing an alert type shows and plays it on every thumbnail.
- Turn on **Travel Mode** from the tray, jump with all but one pilot: the one left behind gets "Left behind in ...".
- **Log:** `Chat-log monitoring started: ...`, then `Watching logs for <pilot>` for each logged-in client. If it
  says the log folders weren't found, the folder paths are wrong.

## 5. DPS, mining, bounty and CPU/RAM overlays

- In the settings, **Overlays** tab (Advanced Mode): turn on the combat, mining, bounty and resource overlays, and
  save.
- **Expect:**
  - Shooting something (or being shot) shows `IN` and `OUT` damage per second.
  - Mining shows m3/min and ISK/hr.
  - Bounties show ISK/hr.
  - Each thumbnail shows its client's CPU% and RAM, and usually VRAM.
- Taking damage with the Taking Damage alert on flashes "Taking damage". A mining laser left idle gives "Laser idle".
- **Log:** `Resource sampling started (... VRAM available)` or `VRAM unavailable`. VRAM missing on some PCs is
  expected; CPU and RAM should always show.

## 6. Client List and History Panel

- In the settings, **Layout** tab: set **View Mode** to **Client List** and save.
- **Expect:** the thumbnails go away and a list appears with one row per client, each with a coloured dot, the
  name, and the system or live stats. Clicking a row brings that client forward, and Shift+click excludes it from
  cycling. Drag the list by its header: it snaps to screen edges. With "remember position" on, it comes back in
  the same place after a restart.
- Turn on the History Panel (**Notifications** tab → **Show History Panel**, or the tray's **Show History Panel**).
- **Expect:** a panel listing recent alerts, newest first. Clicking an entry brings that client forward. The footer
  buttons filter by category. The tray's **Clear Notification History** empties it.
- Both panels are rounded cards in the settings window's colours. In the settings, **Preferences** tab, pick another
  theme and save: within a second or so both panels change to the new colours.
- Switch the view mode back to **Thumbnails** afterwards.

## 7. Region selection

- In the settings, **Layout** tab: turn on region fit, click **New Thumbnail Region** (or **Edit Region**), and
  choose to draw it on screen.
- **Expect:** the screen dims and you can draw a rectangle. Drag its edges to resize and its middle to move it, then
  press **Save** or Enter. The settings window fills in the region and the thumbnails fit inside it after you save.
- Start again and press Esc or right-click: it cancels and nothing changes.
- **Log:** `Region selection started ...` and `Region selection finished: Success ...`.

## 8. Links and other bits

- Run `eve-maj-preview.exe` a second time: it should exit quietly (only one copy runs).
- If you use Stream Deck or scripts with `evemajpreview://` links, try one, for example
  `evemajpreview://hotkey/minimize_all`.
- In the settings, **Preferences** tab: turn on **Run on Startup**, save, sign out and back in: EVE-Maj starts by itself.
  Turn it off again if you don't want that.

## Sending results back

For each section, note whether it worked and anything that looked wrong. Send `eve-maj.log` (and
`eve-maj-crash.dmp`, if there is one). Check the log first: it contains your character names and folder paths, so
remove anything you don't want to share.
