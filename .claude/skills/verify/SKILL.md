---
name: verify
description: Build, launch and drive the Reveille Tauri app on Windows to observe a change at its real surface.
---

<!-- SPDX-License-Identifier: GPL-2.0-only -->

# Verifying a Reveille change

The shell is a Tauri desktop app. `frontendDist` is a static directory, so there is no
frontend build step — editing `crates/reveille-app/ui` and re-running is enough.

## Handle

    cargo build -p reveille-app     # ~first run is slow; afterwards seconds
    cargo run -p reveille-app       # or `just app`

The CLI (`just cli --help`, `just discover`, `just scan PATH`) is a second surface, but it
does **not** cover the engine-install commands — those exist only as Tauri commands
(`crates/reveille-app/src/main.rs`) reached from `ui/views/setup.js`.

## Driving the window

There is no WebDriver here. Capture and click the native window from PowerShell: find the
process whose `MainWindowTitle` is `Reveille`, `GetWindowRect` it, `CopyFromScreen` into a
bitmap for a screenshot, and `SetCursorPos` + `mouse_event` for clicks. Window-relative
coordinates from a screenshot map straight onto click coordinates. `SendKeys::SendWait`
types into a focused input (`^a` first to replace its contents).

Re-capture after every click: the setup card re-renders and **button rows move vertically**
between states (a result note or a progress meter inserts a paragraph). Clicking a
remembered coordinate hits the gap and silently does nothing.

## Reaching the setup view

On a machine with a remembered install the app opens straight into the server browser.
Click the install chip at the top right to get to the setup card. From there
"Choose another folder" gives a manual path field.

## A safe install target

`install::identify` accepts any directory containing `main/`, `mainta/` or `maintt/` — a
client binary is optional. So

    mkdir -p <scratch>/fakegame/main

is enough to make a scratch folder identify as an Allied Assault install. Use it whenever
you drive `install_openmohaa`: that command **overlays real files into the game root**, so
never point it at the player's actual install.

## Faking a running client

`platform::openmohaa_client_activity` matches on process image name only. Copying any
long-lived exe to `omohaaded.exe` (or `openmohaa.exe`, or a `launch_openmohaa_*.exe`) and
running it is enough to drive the "currently open" branch:

    cp /c/Windows/System32/ping.exe <scratch>/probe/omohaaded.exe
    # Start-Process ... -ArgumentList '-n','90','127.0.0.1' -WindowStyle Hidden

## Gotchas

- The OpenMoHAA release archive is ~5 MB and downloads in well under a second on a fast
  link. The "Stop download" path is effectively impossible to hit by clicking; do not
  report it verified from a UI click alone.
- `openmohaa_status` and `install_openmohaa` each hit the GitHub API. Unauthenticated
  requests are rate-limited per IP — do not sit on Refresh.
- `tasklist` output is localised (a French machine prints `4 228 Ko`); the parser only
  reads the first quoted field, so this is fine, but do not grep the rest of the row.
