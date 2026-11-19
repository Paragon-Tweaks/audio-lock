# Audio Lock Named Pipe IPC Reference

Audio Lock exposes a local, per-user Windows Named Pipe server so you can query live audio state and trigger any app action from PowerShell, AutoHotkey, Stream Deck, PTU, or custom scripts.

## Endpoint, Framing & Security

| Property | Value |
| :--- | :--- |
| **Pipe path** | `\\.\pipe\default-audio-lock-ipc` |
| **Encoding** | UTF-8 JSON (no BOM) |
| **Framing** | Newline-delimited: `\n`-terminated request (`\r\n` also accepted) -> `\n`-terminated response |
| **Connection model** | One request per connection; open a fresh pipe connection for each request |
| **Max request size** | 64 KiB (`65,536` bytes before `\n`); oversized or empty requests are dropped |
| **I/O timeout** | 2 seconds (`2,000 ms`) for reading the request and 2 seconds for writing the response |
| **Access control** | Local clients only (`PIPE_REJECT_REMOTE_CLIENTS`); DACL restricted to the current Windows user SID, Administrators (`BA`), and `SYSTEM` (`SY`) |

### Clean Shutdown Event (Installer Alternative)

In addition to the `"QuitDaemon"` IPC request, Audio Lock listens on the per-session named Win32 event:

```text
Local\DefaultAudioLockQuitEvent
```

Signaling this event (for example via `OpenEventW` + `SetEvent`) triggers the same clean tray daemon shutdown. The installer uses this event to exit a running instance before replacing files.

---

## Request Format (`IpcRequest`)

Requests use standard Serde externally tagged JSON. Unit variants serialize as JSON strings; tuple and struct variants serialize as single-key JSON objects.

| Request | Exact JSON | Response | Notes |
| :--- | :--- | :--- | :--- |
| `Ping` | `"Ping"` | `"Ok"` | Health check to verify the daemon is listening. |
| `GetState` | `"GetState"` | `{"State": {...}}` | Returns the full current `AppState` snapshot. |
| `ApplyAction` | `{"ApplyAction": <AppAction>}` | `{"State": {...}}` | Applies an `AppAction` (see below) and returns the updated `AppState`. |
| `OpenFlyout` | `{"OpenFlyout": {"anchor_x": null, "anchor_y": null}}` | `"Ok"` | Opens the flyout. Pass integer screen coordinates for `anchor_x` / `anchor_y`, or `null` to reuse the last saved anchor. |
| `QuitDaemon` | `"QuitDaemon"` | `"Ok"` | Cleanly exits the Audio Lock tray daemon. |

---

## Response Format (`IpcResponse`)

Every valid connection receives one `\n`-terminated JSON value:

1. **Success without payload**:
   ```json
   "Ok"
   ```
   Returned by `"Ping"`, `OpenFlyout`, and `"QuitDaemon"`.

2. **State snapshot**:
   ```json
   {
     "State": {
       "lock_enabled": true,
       "theme": "Dark",
       "separate_comms_role": false,
       "right_click_action": "Menu",
       "run_on_startup": false,
       "auto_update": true,
       "staged_update_version": null,
       "auto_disable_enhancements": false,
       "auto_disable_unused_devices": false,
       "active_profile_id": "gaming",
       "roles": {
         "default_output_id": "{0.0.0.00000000}.{...}",
         "comms_output_id": "{0.0.0.00000000}.{...}",
         "default_input_id": "{0.0.1.00000000}.{...}",
         "comms_input_id": "{0.0.1.00000000}.{...}"
       },
       "devices": [
         {
           "id": "{0.0.0.00000000}.{...}",
           "name": "Headphones",
           "subtitle": "USB Audio DAC",
           "flow": "Output",
           "volume": 65,
           "muted": false,
           "pinned": true,
           "disabled": false,
           "enhancements": {
             "sys_fx_enabled": false,
             "advanced_enabled": false,
             "spatial_sound_enabled": false
           },
           "previous_enhancements": null,
           "listen": null
         }
       ],
       "profiles": [
         {
           "id": "gaming",
           "name": "Gaming",
           "roles": {
             "default_output_id": "{0.0.0.00000000}.{...}",
             "comms_output_id": "{0.0.0.00000000}.{...}",
             "default_input_id": "{0.0.1.00000000}.{...}",
             "comms_input_id": "{0.0.1.00000000}.{...}"
           },
           "device_settings": []
         }
       ]
     }
   }
   ```
   Returned by `"GetState"` and `{"ApplyAction": ...}`. Input devices include `"listen": {"enabled": false, "target_device_id": null}` instead of `null`.

3. **Parse / protocol error**:
   ```json
   {"Error": "Invalid IPC request: ..."}
   ```
   Returned when the request line is not valid `IpcRequest` JSON.

---

## Supported Actions (`AppAction`)

Wrap every action in `{"ApplyAction": <action>}`. For example:

```json
{"ApplyAction": {"SetLockEnabled": true}}
```

### Enum Values

- `DeviceFlow`: `"Output"` | `"Input"`
- `ThemeMode`: `"Dark"` | `"Light"`
- `RightClickAction`: `"Menu"` | `"Flyout"`

### Locking & Device Roles

| Variant | Exact `<action>` JSON | Notes |
| :--- | :--- | :--- |
| `SetLockEnabled` | `{"SetLockEnabled": true}` | Enable (`true`) or pause (`false`) default endpoint locking. |
| `SetDefault` | `{"SetDefault": {"flow": "Output", "device_id": "{...}"}}` | Sets Default Output (`"Output"`) or Default Input (`"Input"`). Also updates the matching Communications role when `separate_comms_role` is `false`. |
| `SetComms` | `{"SetComms": {"flow": "Output", "device_id": "{...}"}}` | Sets Communications Output (`"Output"`) or Communications Input (`"Input"`). |
| `SetSeparateCommsRole` | `{"SetSeparateCommsRole": false}` | Splits (`true`) or unifies (`false`) Default and Communications roles. |

### Volume, Mute & Pinning

| Variant | Exact `<action>` JSON | Notes |
| :--- | :--- | :--- |
| `SetVolume` | `{"SetVolume": {"device_id": "{...}", "volume": 80}}` | Integer `0` to `100` (values above `100` are clamped to `100`). |
| `SetMute` | `{"SetMute": {"device_id": "{...}", "muted": true}}` | Explicitly mutes (`true`) or unmutes (`false`) a device. |
| `ToggleMute` | `{"ToggleMute": {"device_id": "{...}"}}` | Toggles mute state on a device. |
| `SetDevicePinned` | `{"SetDevicePinned": {"device_id": "{...}", "pinned": true}}` | Pins or unpins a device (pinned devices sort first and stay enabled under auto-disable). |
| `ToggleDevicePin` | `{"ToggleDevicePin": {"device_id": "{...}"}}` | Toggles pinned state on a device. |

### Device Disabling

| Variant | Exact `<action>` JSON | Notes |
| :--- | :--- | :--- |
| `SetDeviceDisabled` | `{"SetDeviceDisabled": {"device_id": "{...}", "disabled": true}}` | Disables or re-enables a single endpoint. Ignored while `auto_disable_unused_devices` is `true`, and refuses to disable the last remaining enabled device for a flow. |
| `ReenableAllDevices` | `"ReenableAllDevices"` | Unit variant (JSON string). Re-enables all disabled audio devices. |
| `SetAutoDisableUnusedDevices` | `{"SetAutoDisableUnusedDevices": false}` | Automatically disables devices that are not Default, Communications, pinned, or an active Listen target. |

### Windows Audio Enhancements

| Variant | Exact `<action>` JSON | Notes |
| :--- | :--- | :--- |
| `SetEnhancementsDisabled` | `{"SetEnhancementsDisabled": {"device_id": "{...}", "disabled": true}}` | Disables (`true`) `SysFx` and Spatial Sound while saving prior settings for one-click undo, or restores (`false`) prior settings. |
| `SetAutoDisableEnhancements` | `{"SetAutoDisableEnhancements": false}` | Automatically disables enhancements on current and newly connected devices. |
| `ReenableAllEnhancements` | `"ReenableAllEnhancements"` | Unit variant (JSON string). Restores prior enhancement settings across all devices. |

### Listen to This Device (Input Monitoring)

| Variant | Exact `<action>` JSON | Notes |
| :--- | :--- | :--- |
| `SetListenEnabled` | `{"SetListenEnabled": {"device_id": "{...}", "enabled": true}}` | Input devices only. Turns "Listen to this device" monitoring on or off. |
| `SetListenTarget` | `{"SetListenTarget": {"device_id": "{...}", "target_device_id": "__default_output__"}}` | Input devices only. Selects the playback target (see sentinel values below). |

**Special `target_device_id` values for `SetListenTarget`**:

- `null`: `None` (no monitoring target selected)
- `"__default_output__"`: Follow the current Default Playback Device
- `"__comms_output__"`: Follow the current Default Communications Playback Device
- `"{0.0.0.00000000}.{...}"`: Route monitoring to a specific playback device ID

### Profiles

| Variant | Exact `<action>` JSON | Notes |
| :--- | :--- | :--- |
| `SaveProfile` | `{"SaveProfile": {"name": "Gaming"}}` | Saves current roles and per-device settings. Overwrites an existing profile if `name` matches case-insensitively; otherwise creates a new slugified `id`. |
| `ApplyProfile` | `{"ApplyProfile": {"profile_id": "gaming"}}` | Applies a saved profile by its `id` (inspect `profiles` via `"GetState"`). |
| `DeleteProfile` | `{"DeleteProfile": {"profile_id": "gaming"}}` | Deletes a saved profile by its `id`. |

### App Settings & Updates

| Variant | Exact `<action>` JSON | Notes |
| :--- | :--- | :--- |
| `SetTheme` | `{"SetTheme": "Dark"}` | `"Dark"` or `"Light"`. |
| `SetRightClickAction` | `{"SetRightClickAction": "Menu"}` | `"Menu"` (native quick menu) or `"Flyout"`. |
| `SetRunOnStartup` | `{"SetRunOnStartup": true}` | Adds or removes the `HKCU\...\Run` startup entry. |
| `SetAutoUpdate` | `{"SetAutoUpdate": true}` | Enables or disables background GitHub Release checks. |
| `CheckForUpdatesNow` | `"CheckForUpdatesNow"` | Unit variant (JSON string). Triggers an immediate background update check and returns the current state snapshot. |
| `ApplyStagedUpdateNow` | `"ApplyStagedUpdateNow"` | Unit variant (JSON string). Applies a downloaded update immediately if `staged_update_version` is present. |

---

## Copy-Paste Automation Examples

### PowerShell Helper (`Send-AudioLockIpc`)

Works on both Windows PowerShell 5.1 and PowerShell 7+. Uses BOM-free UTF-8 encoding over `System.IO.Pipes.NamedPipeClientStream`.

```powershell
function Send-AudioLockIpc {
    param(
        [Parameter(Mandatory)]
        $Request,
        [int]$TimeoutMs = 2000
    )

    $json  = ($Request | ConvertTo-Json -Compress -Depth 8) + "`n"
    $utf8  = [System.Text.UTF8Encoding]::new($false)
    $bytes = $utf8.GetBytes($json)

    $pipe = [System.IO.Pipes.NamedPipeClientStream]::new(
        '.',
        'default-audio-lock-ipc',
        [System.IO.Pipes.PipeDirection]::InOut
    )
    try {
        $pipe.Connect($TimeoutMs)
        $pipe.Write($bytes, 0, $bytes.Length)
        $pipe.Flush()

        $reader = [System.IO.StreamReader]::new($pipe, $utf8, $false, 4096, $true)
        try {
            $line = $reader.ReadLine()
        } finally {
            $reader.Dispose()
        }

        if ([string]::IsNullOrWhiteSpace($line)) {
            throw "No response from Audio Lock IPC server."
        }
        return ($line | ConvertFrom-Json)
    } finally {
        $pipe.Dispose()
    }
}
```

### PowerShell One-Liners

**Get current state, list saved profiles, and inspect device IDs:**

```powershell
$state = (Send-AudioLockIpc "GetState").State
$state.profiles | Select-Object id, name
$state.devices  | Select-Object flow, name, id, volume, muted
```

**Switch to a saved profile by ID:**

```powershell
Send-AudioLockIpc @{ ApplyAction = @{ ApplyProfile = @{ profile_id = "gaming" } } }
```

**Toggle Lock on/off:**

```powershell
$locked = (Send-AudioLockIpc "GetState").State.lock_enabled
Send-AudioLockIpc @{ ApplyAction = @{ SetLockEnabled = (-not $locked) } }
```

**Mute/unmute or toggle mute on your active microphone:**

```powershell
# Toggle mute on the default input device
$micId = (Send-AudioLockIpc "GetState").State.roles.default_input_id
Send-AudioLockIpc @{ ApplyAction = @{ ToggleMute = @{ device_id = $micId } } }

# Explicitly set mute state (true = muted, false = live)
Send-AudioLockIpc @{ ApplyAction = @{ SetMute = @{ device_id = $micId; muted = $true } } }
```

**Open the Audio Lock flyout:**

```powershell
Send-AudioLockIpc @{ OpenFlyout = @{ anchor_x = $null; anchor_y = $null } }
```

### Stream Deck & AutoHotkey Integration

#### Stream Deck

Save `Send-AudioLockIpc` plus a small command script (for example `C:\Scripts\Switch-AudioProfile.ps1`), then bind a Stream Deck **System: Open** action to:

```text
powershell.exe -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File "C:\Scripts\Switch-AudioProfile.ps1" -ProfileId gaming
```

#### AutoHotkey v2 (Direct Pipe Write)

For instant hotkeys without PowerShell startup overhead, AutoHotkey v2 can write directly to the Named Pipe using `UTF-8-RAW` (no BOM):

```ahk
#Requires AutoHotkey v2.0

SendAudioLockIpc(json) {
    pipe := FileOpen("\\.\pipe\default-audio-lock-ipc", "rw", "UTF-8-RAW")
    pipe.Write(json "`n")
    response := pipe.ReadLine()
    pipe.Close()
    return response
}

; Ctrl+Alt+1: Apply the "gaming" profile
^!1::SendAudioLockIpc('{"ApplyAction":{"ApplyProfile":{"profile_id":"gaming"}}}')

; Ctrl+Alt+2: Open the Audio Lock flyout
^!2::SendAudioLockIpc('{"OpenFlyout":{"anchor_x":null,"anchor_y":null}}')
```
