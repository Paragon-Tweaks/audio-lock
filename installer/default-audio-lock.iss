#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif

#ifndef SourceDir
  #define SourceDir "..\target\release"
#endif

[Setup]
AppId={{8E9A7B2C-4D5E-4F1A-9B3C-2D1E0F4A5B6C}
AppName=Audio Lock
AppVersion={#AppVersion}
AppVerName=Audio Lock {#AppVersion}
AppPublisher=Paragon Tweaks
AppPublisherURL=https://paragontweaks.net
AppSupportURL=https://github.com/Paragon-Tweaks/audio-lock/issues
AppUpdatesURL=https://github.com/Paragon-Tweaks/audio-lock/releases
AppCopyright=@TheTulantro @ParagonTweaks
VersionInfoCompany=Tulantro LLC (Paragon Tweaks)
VersionInfoCopyright=@TheTulantro @ParagonTweaks
VersionInfoProductName=Audio Lock
VersionInfoDescription=Audio Lock Setup
DefaultDirName={localappdata}\Programs\DefaultAudioLock
DefaultGroupName=Audio Lock
AllowNoIcons=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=DefaultAudioLock-Setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupIconFile=..\assets\app.ico
UninstallDisplayIcon={app}\default-audio-lock.exe
; Uncomment once a code-signing certificate is configured (pass /Ssigntool="..." to iscc):
; SignTool=signtool
; SignedUninstaller=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "Start Audio Lock when Windows starts"
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"

[Files]
Source: "{#SourceDir}\default-audio-lock.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Audio Lock"; Filename: "{app}\default-audio-lock.exe"
Name: "{group}\Uninstall Audio Lock"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Audio Lock"; Filename: "{app}\default-audio-lock.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "DefaultAudioLock"; ValueData: """{app}\default-audio-lock.exe"""; Tasks: autostart; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "DefaultAudioLock"; Flags: dontcreatekey uninsdeletevalue

[UninstallDelete]
Type: files; Name: "{app}\*.old"
Type: filesandordirs; Name: "{localappdata}\default-audio-lock\staging"

[Run]
Filename: "{app}\default-audio-lock.exe"; Description: "Launch Audio Lock"; Flags: nowait postinstall skipifsilent

[Code]
const
  TrayMutexName = 'Local\DefaultAudioLockTrayDaemon';
  QuitEventName = 'Local\DefaultAudioLockQuitEvent';
  EVENT_MODIFY_STATE = $0002;
  SYNCHRONIZE = $00100000;

function OpenEventW(dwDesiredAccess: DWORD; bInheritHandle: BOOL; lpName: String): THandle;
  external 'OpenEventW@kernel32.dll stdcall';

function SetEvent(hEvent: THandle): BOOL;
  external 'SetEvent@kernel32.dll stdcall';

function OpenMutexW(dwDesiredAccess: DWORD; bInheritHandle: BOOL; lpName: String): THandle;
  external 'OpenMutexW@kernel32.dll stdcall';

function CloseHandle(hObject: THandle): BOOL;
  external 'CloseHandle@kernel32.dll stdcall';

function IsTrayMutexOpen: Boolean;
var
  MutexHandle: THandle;
begin
  MutexHandle := OpenMutexW(SYNCHRONIZE, False, TrayMutexName);
  Result := MutexHandle <> 0;
  if Result then
    CloseHandle(MutexHandle);
end;

procedure StopRunningInstance;
var
  EventHandle: THandle;
  ResultCode: Integer;
  Attempts: Integer;
begin
  if not IsTrayMutexOpen then
    Exit;

  EventHandle := OpenEventW(EVENT_MODIFY_STATE, False, QuitEventName);
  if EventHandle <> 0 then
  begin
    if SetEvent(EventHandle) then
    begin
      Attempts := 0;
      while IsTrayMutexOpen and (Attempts < 30) do
      begin
        Sleep(100);
        Attempts := Attempts + 1;
      end;
    end;
    CloseHandle(EventHandle);
  end;

  if IsTrayMutexOpen then
    Exec('taskkill.exe', '/F /IM default-audio-lock.exe /T', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);

  Attempts := 0;
  while IsTrayMutexOpen and (Attempts < 20) do
  begin
    Sleep(100);
    Attempts := Attempts + 1;
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopRunningInstance;
  Result := '';
end;

function InitializeUninstall: Boolean;
begin
  StopRunningInstance;
  Result := True;
end;
