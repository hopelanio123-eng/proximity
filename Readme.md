# KoshkaRemote

Control your PC from any device with a web browser — touch, stylus, mouse and a
**working on-screen keyboard** on your phone.

KoshkaRemote is an AGPL-3.0 fork of
[Weylus](https://github.com/H-M-H/Weylus) by H-M-H with additional features.

## What was added compared to upstream Weylus

- New `TextEvent` protocol message (browser -> server over websocket).
- **"Toggle Keyboard"** button in the web UI: opens a small text funnel on the
  device page. Whatever you type on your phone's *soft* keyboard (including
  Unicode/Cyrillic) is injected into the host:
  - Windows: `SendInput` with `KEYEVENTF_UNICODE`, so any character works
    without a physical layout mapping (batched down/up, surrogate pairs safe).
  - macOS/X11: characters are synthesized through `autopilot`.
  - Linux: uinput CTRL+SHIFT+U unicode insertion.
- Enter/Backspace from the soft keyboard are translated to proper key events.
- Upstream Weylus explicitly does *not* support on-screen keyboards; this fork
  exists to fill exactly that gap.

## Usage

1. Start KoshkaRemote on the PC and press **Start** (web port 1701, websocket
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

Requirements (same as upstream): Rust, TypeScript, bash, make, a C compiler and
nasm; ffmpeg/x264 are built from source automatically. Alternatively configure
with the `ffmpeg-system` feature.

Easiest is the bundled GitHub Actions workflow (manual `workflow_dispatch`):
it builds the Windows binary (mingw cross-compile), the Linux binary and a deb
inside the official `hhmhh/weylus_build:latest` container and uploads artifacts.

Log level can be tuned with the `KOSHKA_LOG_LEVEL` and `KOSHKA_LOG_JSON`
environment variables.

## License

AGPL-3.0-or-later, unchanged from upstream — see [LICENSE](LICENSE).
All credit for the original Weylus goes to H-M-H and contributors.
