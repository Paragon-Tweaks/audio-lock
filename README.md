# Audio Lock

by [Paragon Tweaks](https://paragontweaks.net)

[![Latest Release](https://img.shields.io/github/v/release/Paragon-Tweaks/audio-lock?style=flat-square&color=1f1f1f&labelColor=111111&label=Release)](https://github.com/Paragon-Tweaks/audio-lock/releases/latest)
[![License](https://img.shields.io/badge/License-GPLv3-1f1f1f?style=flat-square&labelColor=111111)](LICENSE)
![Platform](https://img.shields.io/badge/Platform-Windows%2010%20%2F%2011-1f1f1f?style=flat-square&labelColor=111111)
![Binary Size](https://img.shields.io/badge/Binary%20Size-1.39%20MB-1f1f1f?style=flat-square&labelColor=111111)

Lock your Windows default audio devices, mute both microphones from the tray, disable audio enhancements, and hot-swap profiles in a single 1.39 MB pure-Rust Win32 executable with 0.00% idle CPU and zero GPU usage.

## Download

**[Download Latest Release (`DefaultAudioLock-Setup.exe`)](https://github.com/Paragon-Tweaks/audio-lock/releases/latest)**

Per-user installer (`%LOCALAPPDATA%\Programs\DefaultAudioLock`). No Admin rights or UAC prompt required.

<!-- SCREENSHOTS: Add Dark and Light flyout showcase screenshots here once captured -->

## Measured Performance (`--release`)

| Metric | Measured Result |
| :--- | :--- |
| **Single Executable** | `1.39 MB` (`1,455,616` bytes) |
| **Idle RAM** | `1.23 MB` Working Set (`2.14 MB` Private) |
| **Idle CPU** | `0.000%` (zero polling, pure Win32 message loop + CoreAudio COM callbacks) |
| **Flyout Open** | `6.88 MB` Working Set (`2.49 MB` Private), `~23 ms` open, `0 MB` GPU/VRAM (no D3D11/DXGI loaded) |
| **Priority** | `BelowNormal` + Windows Efficiency Mode (`EcoQoS`) |
| **OS Dependencies** | Zero WebView2, zero .NET, zero Windows Services or Task Scheduler (works on stripped/tweaked Windows builds) |

## Features

- **Event-driven Default & Communications audio locking**: CoreAudio COM callbacks catch Windows, USB hot-plug, and driver updates immediately. Uses unified Default + Communications locking by default, with an optional separate Default/Comms mode in Settings.
- **Instant mic-mute tray icon**: Crisp microphone status in the tray (white outline when live, red capsule fill when muted). Left-click toggles mute on both Default and Communications microphones at once.
- **Automatic unused-device disabling**: Keeps your Default, Comms, pinned devices, and active Listen targets enabled while disabling unused endpoints so apps like Krisp never grab the wrong mic.
- **One-click and automatic audio enhancement disabling**: Turns off Enhancements tab SysFx, the Advanced tab checkbox, and Windows Spatial Sound, with undo support and bulk re-enable.
- **`Listen to this device` routing**: Searchable playback dropdown defaulting safely to `None`, with `Default Playback Device` and `Default Communications Device` pinned at the top.
- **Instant pure-Win32 menu and flyout**: Responsive right-click context menu and monochrome Dark/Light flyout with hot-swap profiles, per-device pinning, live search, and 26 px tactile volume bars.
- **Zero-hook automatic GitHub Releases updater**: Uses native `WinHTTP` and `BCrypt` SHA-256 verification. Staged updates restart automatically only after 5 minutes of mouse and keyboard inactivity via `GetLastInputInfo`.
- **Non-elevated per-user install and startup**: Installs per-user and registers optional autostart via `HKCU\...\Run` with no background service or scheduled task.

## Built by Paragon Tweaks

Built by [Paragon Tweaks](https://paragontweaks.net). Want full-system FPS, latency, BIOS, and network tuning beyond audio? Check out [PTU (Paragon Tweaking Utility)](https://paragontweaks.net/utilities).

Audio Lock is the first tool in Paragon Tweaks' free, open-source QoL suite.

## Building from Source & Releasing

Requires the stable Rust toolchain (`x86_64-pc-windows-msvc`).

```powershell
# Build release binary (target\release\default-audio-lock.exe)
cargo build --release --workspace

# Run tests and lints
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

### Local Release Packaging

Packages `dist\default-audio-lock-win64-update.zip`, `dist\manifest.json` (with SHA-256 hash), and `dist\DefaultAudioLock-Setup-<version>.exe` (when Inno Setup 6 is installed):

```powershell
.\scripts\build-release.ps1 -GithubRepo Paragon-Tweaks/audio-lock
```

### Automated GitHub Releases

Pushing a version tag (`v*`) runs [`.github/workflows/release.yml`](.github/workflows/release.yml) to test the workspace, compile with `AUDIO_LOCK_GITHUB_REPO` set for auto-updates, build the Inno Setup installer, and publish the release assets:

```powershell
git tag v0.1.0
git push origin v0.1.0
```

## License

Copyright (C) 2026 Paragon Tweaks.

Licensed under the **GNU General Public License v3.0** (`GPL-3.0-only`). See [LICENSE](LICENSE) for the full text.
