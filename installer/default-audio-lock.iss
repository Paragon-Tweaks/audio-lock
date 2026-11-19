#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif

#ifndef SourceDir
  #define SourceDir "..\target\release"
#endif

[Setup]
AppId={{8E9A7B2C-4D5E-4F1A-9B3C-2D1E0F4A5B6C}
AppName=Default Audio Lock
AppVersion={#AppVersion}
AppVerName=Default Audio Lock {#AppVersion}
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
DefaultGroupName=Default Audio Lock
AllowNoIcons=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=DefaultAudioLock-Setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\default-audio-lock.exe
; Uncomment once a code-signing certificate is configured (pass /Ssigntool="..." to iscc):
; SignTool=signtool
; SignedUninstaller=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "Start Default Audio Lock when Windows starts"

[Files]
Source: "{#SourceDir}\default-audio-lock.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Default Audio Lock"; Filename: "{app}\default-audio-lock.exe"
Name: "{group}\Uninstall Default Audio Lock"; Filename: "{uninstallexe}"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "DefaultAudioLock"; ValueData: """{app}\default-audio-lock.exe"""; Tasks: autostart; Flags: uninsdeletevalue

[UninstallDelete]
Type: files; Name: "{app}\*.old"
Type: filesandordirs; Name: "{localappdata}\default-audio-lock\staging"

[Run]
Filename: "{app}\default-audio-lock.exe"; Description: "Launch Default Audio Lock"; Flags: nowait postinstall skipifsilent

[Code]
const
  TrayMutexName = 'Local\DefaultAudioLockTrayDaemon';

procedure CloseRunningInstances;
var
  ResultCode: Integer;
  Attempts: Integer;
begin
  if not CheckForMutexes(TrayMutexName) then
    Exit;

  Exec('cmd.exe', '/c echo "QuitDaemon">\\.\pipe\default-audio-lock-ipc', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Attempts := 0;
  while CheckForMutexes(TrayMutexName) and (Attempts < 30) do
  begin
    Sleep(100);
    Attempts := Attempts + 1;
  end;

  if CheckForMutexes(TrayMutexName) then
    Exec('taskkill.exe', '/F /IM default-audio-lock.exe /T', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);

  Attempts := 0;
  while CheckForMutexes(TrayMutexName) and (Attempts < 20) do
  begin
    Sleep(100);
    Attempts := Attempts + 1;
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  CloseRunningInstances;
  Result := '';
end;

function InitializeUninstall: Boolean;
begin
  CloseRunningInstances;
  Result := True;
end;
