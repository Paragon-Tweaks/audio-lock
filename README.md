<a href="https://paragontweaks.net"><img src="assets/banner.jpg" alt="Paragon Tweaks" width="100%" /></a>

> Built by **[Paragon Tweaks](https://paragontweaks.net)**. Want full-system FPS, latency, BIOS, and network tuning beyond audio? Check out **[PTU (Paragon Tweaking Utility)](https://paragontweaks.net/utilities)** at https://paragontweaks.net/utilities.

# Audio Lock

[![Latest Release](https://img.shields.io/github/v/release/Paragon-Tweaks/audio-lock?style=flat-square&color=1f1f1f&labelColor=111111&label=Release)](https://github.com/Paragon-Tweaks/audio-lock/releases/latest)
[![License](https://img.shields.io/badge/License-GPLv3-1f1f1f?style=flat-square&labelColor=111111)](LICENSE)
![Platform](https://img.shields.io/badge/Platform-Windows%2010%20%2F%2011-1f1f1f?style=flat-square&labelColor=111111)
![Binary Size](https://img.shields.io/badge/Binary%20Size-1.49%20MB-1f1f1f?style=flat-square&labelColor=111111)

Keep your Windows speakers and microphone locked where you want them, mute your mic from the taskbar, turn off hidden Windows sound effects, and hot-swap audio profiles in a single ~1.49 MB app that uses 0% CPU while you play.

**[Download Latest Release (`DefaultAudioLock-Setup-<version>.exe`)](https://github.com/Paragon-Tweaks/audio-lock/releases/latest)**

Per-user installer (`%LOCALAPPDATA%\Programs\DefaultAudioLock`). No Admin rights or UAC prompt required.

<table>
  <tr>
    <td align="center" colspan="2">
      <img src="assets/screenshots/flyout-dark.png" alt="Audio Lock flyout in Dark Mode" width="275" /><br />
      <sub><strong>Dark Mode</strong></sub>
    </td>
    <td align="center" colspan="2">
      <img src="assets/screenshots/flyout-light.png" alt="Audio Lock flyout in Light Mode" width="275" /><br />
      <sub><strong>Light Mode</strong></sub>
    </td>
    <td align="center" colspan="2">
      <img src="assets/screenshots/flyout-settings.png" alt="Audio Lock flyout Settings view" width="275" /><br />
      <sub><strong>Settings</strong></sub>
    </td>
  </tr>
  <tr>
    <td align="center" colspan="3">
      <img src="assets/screenshots/menu-dark.png" alt="Audio Lock right-click quick menu in Dark Mode" width="260" /><br />
      <sub><strong>Quick Menu (Dark)</strong></sub>
    </td>
    <td align="center" colspan="3">
      <img src="assets/screenshots/menu-light.png" alt="Audio Lock right-click quick menu in Light Mode" width="260" /><br />
      <sub><strong>Quick Menu (Light)</strong></sub>
    </td>
  </tr>
  <tr>
    <td align="center" colspan="3">
      <img src="assets/screenshots/tray-live.png" alt="Windows taskbar tray icon hover tooltip showing mic live" width="300" /><br />
      <sub><strong>Tray Hover (Mic Live)</strong></sub>
    </td>
    <td align="center" colspan="3">
      <img src="assets/screenshots/tray-muted.png" alt="Windows taskbar tray icon hover tooltip showing mic muted" width="300" /><br />
      <sub><strong>Tray Hover (Mic Muted)</strong></sub>
    </td>
  </tr>
</table>

## What It Does

- **Lock your audio defaults**: Stops Windows, game updates, monitor wakes, and USB headsets from randomly hijacking your speakers or mic.
- **One-click mic mute from your taskbar**: White mic outline means live, red inside means muted. Left-click mutes both your Default and Communications mics at once.
- **Hide unused audio devices (optional)**: Turn on the optional Settings toggle (off by default) to automatically disable devices you are not using (while keeping your pinned favorites enabled) so apps like Discord, OBS, or Krisp never grab the wrong mic.
- **Turn off Windows audio enhancements in one click**: Disables hidden Windows sound effects (`SysFx`) and turns off Spatial Sound so they never muddy game audio, with one-click undo that restores your prior Windows audio enhancement (`SysFx`) and Spatial Sound format (Windows Sonic, Dolby Atmos, DTS:X) settings, plus an optional Settings toggle (off by default) to disable them automatically on new devices.
- **Hot-swap audio profiles**: Switch between Gaming, Streaming, Speakers, or Work setups from the flyout or the instant right-click tray menu.
- **Safe "Listen to this device" monitoring**: Hear your mic through any playback device with a searchable dropdown that defaults to `None` so you never blast feedback by accident.
- **Stays out of your way while gaming**: Uses ~1.2 MB of RAM, 0% CPU when idle, and zero GPU memory. Auto-updates silently in the background and only restarts when your PC is genuinely idle for 5 minutes, never interrupting fullscreen games or active controller sessions.

## Measured Performance (`--release`)

| Metric | Measured Result |
| :--- | :--- |
| **Single Executable** | `1.49 MB` (`1,559,040` bytes) |
| **Idle RAM** | `1.23 MB` Working Set (`2.14 MB` Private) |
| **Idle CPU** | `0.000%` (zero polling, pure Win32 message loop + CoreAudio COM callbacks) |
| **Flyout Open** | `6.88 MB` Working Set (`2.49 MB` Private), `~23 ms` open, `0 MB` GPU/VRAM (no D3D11/DXGI loaded) |
| **Priority** | `BelowNormal` + Windows Efficiency Mode (`EcoQoS`) |
| **OS Dependencies** | Zero WebView2, zero .NET, zero Windows Services or Task Scheduler (works on stripped/tweaked Windows builds) |

## Technical Breakdown

- **CoreAudio event locking**: Hooks `IMMNotificationClient` and enforces defaults via `IPolicyConfig` as soon as Windows, a USB hot-plug, or a driver update switches endpoints. Defaults to unified Default + Communications locking, with an optional split Default/Comms toggle in Settings (off by default).
- **Pure-Win32 UI**: Rendered directly to GDI/DIB buffers with ClearType text (no WebView2, no D3D11/DXGI, no GPU context). Includes a monochrome Dark/Light flyout with live search, per-device pinning, and 26 px volume bars, alongside an instant native right-click tray menu.
- **Endpoint and enhancement control**: Optionally disables unused endpoints via Settings (off by default) while keeping Default, Communications, pinned devices, and active Listen targets enabled. Disables Windows audio enhancements (`PKEY_AudioEndpoint_Disable_SysFx`) and turns off Windows Spatial Sound, with one-click undo restoring prior `SysFx` and Spatial Sound format (Windows Sonic, Dolby Atmos, DTS:X) settings, bulk re-enable, and an optional auto-disable toggle in Settings (off by default).
- **Zero-hook silent updater**: Checks GitHub Releases using native `WinHTTP`, validates archives with `BCrypt` SHA-256 verification, and restarts staged updates via `GetLastInputInfo` and `SHQueryUserNotificationState` only when the PC is genuinely idle for 5 minutes, never interrupting fullscreen games or active controller sessions.
- **Local automation IPC**: Exposes a per-user Named Pipe JSON interface (`\\.\pipe\default-audio-lock-ipc`) for Stream Deck, AutoHotkey, PowerShell, PTU, and custom hotkey automation. See [`docs/IPC.md`](docs/IPC.md) for the full protocol reference and copy-paste scripts.
- **Per-user install and startup**: Installs to `%LOCALAPPDATA%\Programs\DefaultAudioLock` and registers optional autostart via `HKCU\...\Run`. Requires no elevation, no Windows Services, and no Task Scheduler entries.

## Building from Source & Releasing

Requires Rust 1.85+ (stable toolchain, `x86_64-pc-windows-msvc`).

### Unsigned Local Test Build

Double-click `scripts\build-unsigned.bat` in Windows Explorer or run `scripts\build-unsigned.ps1` in PowerShell for fast local iteration without a code-signing token. It runs `clippy` and unit tests (skip with `-SkipChecks`), clears Authenticode signature requirements for test builds, and builds `target\release\default-audio-lock.exe`:

```powershell
# Double-click scripts\build-unsigned.bat, or run in PowerShell:
.\scripts\build-unsigned.ps1

# Fast iteration (skip clippy/tests)
.\scripts\build-unsigned.ps1 -SkipChecks

# Also package unsigned zip, manifest.json, and installer into dist\unsigned\
.\scripts\build-unsigned.ps1 -Package
```

### Signed Release & Publish

Official releases are Authenticode-signed locally using a Sectigo hardware code-signing token (`signtool.exe` from the Windows 10/11 SDK), Inno Setup 6 (`winget install JRSoftware.InnoSetup`), and GitHub CLI (`gh`).

You can either double-click the batch launchers in Windows Explorer (`scripts\build-signed-local.bat` to build, sign, and verify locally without publishing; `scripts\build-and-release.bat` to build, sign, tag, and publish after a confirmation prompt) or invoke `scripts\build-release.ps1` directly in PowerShell with custom flags.

`scripts\build-release.ps1` validates the version against `Cargo.toml`, runs `clippy` and unit tests, compiles with strict Authenticode updater verification enabled, signs and verifies both `default-audio-lock.exe` and `DefaultAudioLock-Setup-<version>.exe`, generates `dist\default-audio-lock-win64-update.zip` and `dist\manifest.json`, pushes the `v<version>` tag, and creates the GitHub Release:

```powershell
# Double-click scripts\build-signed-local.bat, or run in PowerShell:
# Build, sign, package, and verify locally in dist\ without tagging or publishing
.\scripts\build-release.ps1 -NoPublish

# Publish as a draft GitHub Release
.\scripts\build-release.ps1 -Draft

# Double-click scripts\build-and-release.bat, or run in PowerShell:
# Full signed release & publish (optional: pass -CertSubject "Tulantro LLC" or -CertSha1 <thumbprint>)
.\scripts\build-release.ps1
```

### Continuous Integration

GitHub Actions ([`.github/workflows/release.yml`](.github/workflows/release.yml)) automatically runs `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` on every push to `main` and pull request. Tag pushes (`v*`) do not trigger unsigned CI releases, keeping signed releases owned cleanly by `scripts\build-release.ps1`. A manual `workflow_dispatch` option is also available for unsigned CI packaging.

## License

Copyright (c) 2026 Tulantro LLC (Paragon Tweaks) - @TheTulantro @ParagonTweaks.

Licensed under the **GNU General Public License v3.0** (`GPL-3.0-only`). See [LICENSE](LICENSE) for the full text.
