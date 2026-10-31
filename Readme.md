# Proximity

Control your PC from any device with a web browser — touch, stylus, mouse and a
**working on-screen keyboard** on your phone.

A personal project focused on low-latency screen streaming, touch input and
typing from the local network.

## Features

- New `TextEvent` protocol message (browser -> server over websocket).
- **Redesigned desktop GUI**: a floating island sidebar (224px rail, 44px rows,
  accent icon on the selected entry), matte graphite surfaces with hairlines,
  Segoe UI + Consolas typography, tracked uppercase eyebrows, compact spring
  toggles, hover fades, a status card that cross-fades to green when the
  server starts, and 27 color palettes with a live theme picker.
- **Glass design on the Eclypsion tokens** (design.eclypsion.org/tokens): the
  web client and the default desktop palette share the same surfaces, text
  ramp, accent `#5960df`, radii (8/12/16 px), Inter + JetBrains Mono type and
  spring easings. The web UI uses translucent blurred panels over a soft
  aurora backdrop, a floating settings drawer / bottom sheet, a sliding
  segmented tab control and iOS-style switches; light mode follows the system.
- **Desktop window, glass edition**: a hero card with a static radar mark
  that turns into a scannable QR code (now on Windows too) once the server
  runs, a copy-address button, live tiles for port / access / codec, grouped
  settings rows with hairline dividers and a status card in the sidebar.
- **"Toggle Keyboard"** button in the web UI: opens a small text funnel on the
  device page. Whatever you type on your phone's *soft* keyboard (including
  Unicode/Cyrillic) is injected into the host:
  - Windows: `SendInput` with `KEYEVENTF_UNICODE`, so any character works
    without a physical layout mapping (batched down/up, surrogate pairs safe).
  - macOS/X11: characters are synthesized through `autopilot`.
  - Linux: uinput CTRL+SHIFT+U unicode insertion.
- Enter/Backspace from the soft keyboard are translated to proper key events.
- **Game mode** (web UI, Input tab): a Standoff-style touch layer for shooters.
  Dragging the screen turns the view like a real mouse (relative
  `MouseMoveEvent` → raw input on Windows), a joystick walks with held WASD
  and round buttons hold fire/aim (mouse buttons), jump/crouch/walk, reload,
  weapon slots, Tab/B/Esc. Sensitivity is adjustable and every held control
  is released when the page hides, the connection drops or the client quits.
  Ready-made button layouts ship for popular games (CS2, Valorant, Fortnite,
  CoD: Warzone, Apex Legends, PUBG, Overwatch 2, Dota 2, GTA V, Rust,
  Minecraft, League of Legends), sliders scale and fade every button, and an
  edit mode lets you move, resize, rebind, add, duplicate and delete buttons
  (and move the joystick) per game. Every button can act as hold, toggle
  (tap to latch) or turbo (autofire), the joystick has an adjustable
  deadzone and walk threshold, layouts export/import as JSON and the
  customized layouts persist in the browser.
- The host stays on your local network — no cloud account and no relay.

## Usage

1. Start Proximity on the PC and press **Start** (web port 1701, websocket
   port 9001 by default, bind address `0.0.0.0`).
2. On your phone/tablet open `http://<PC-IP>:1701` in the browser (same local
   network). A QR code is shown in the GUI for convenience.
3. Optionally set an **Access code** in the GUI.
4. For typing tap **Toggle Keyboard** in the settings panel and tap the text
   field — your device's keyboard opens and keystrokes land on the PC.

Graceful use tips: NVENC (NVIDIA) or Media-Foundation hardware encoding can be
enabled in the GUI before starting the server. Both TCP ports 1701/9001 must be
open in the host firewall.

## Building

Requirements: Rust, TypeScript, bash, make, a C compiler and nasm; ffmpeg/x264
are built from source automatically. Alternatively configure with the
`ffmpeg-system` feature.

### Windows / MSVC (local)

A native MSVC build works too; it needs an FFmpeg distribution with headers,
import libraries and DLLs. The `ffmpeg-system` feature links against
`deps/dist_windows`, so put the FFmpeg `include/`, `lib/` and `bin/` trees
there (a BtbN or gyan.dev Windows build works) and run:

```powershell
$env:RUSTFLAGS = "-L deps\dist_windows\lib"
cargo build --features ffmpeg-system,fltk/fltk-bundled
```

`fltk/fltk-bundled` compiles FLTK from source, so no system FLTK is needed.
Copy the FFmpeg DLLs next to the resulting `proximity.exe`. When iterating on
UI code, `PROXIMITY_SKIP_FFMPEG_CC=1` lets `cargo check` skip the C helpers
and the vendored FFmpeg build, but a full `cargo build` needs them.

Log level can be tuned with the `PROXIMITY_LOG_LEVEL` and `PROXIMITY_LOG_JSON`
environment variables.

## Brand

The app mark is [`assets/logo.svg`](assets/logo.svg); the desktop window uses
the same artwork and the transparent [`assets/logo-glyph.svg`](assets/logo-glyph.svg)
is tinted with the active palette in the sidebar.
[`assets/social-preview.svg`](assets/social-preview.svg) is the matching
1280x640 card for GitHub's social preview.
[`assets/logo.png`](assets/logo.png) and
[`assets/social-preview.png`](assets/social-preview.png) are ready-to-upload
PNG exports, and [`assets/proximity.ico`](assets/proximity.ico) is the Windows
shortcut icon.

GitHub does not take a repository icon from the source tree — the closest
repository-level image is the social preview under **Settings → General**.
Upload `assets/social-preview.png` there. On Linux the icon is installed to
the hicolor theme by the `.deb` packaging.

## License

AGPL-3.0-or-later — see [LICENSE](LICENSE).
