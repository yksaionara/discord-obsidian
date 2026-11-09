; Our installer: installs target\release\obsidian.exe as "Discord Obsidian" for the
; current user, without administrator rights. scripts\ship.ps1 compiles it
; with the version in Cargo.toml; by hand:
;   "C:\Program Files\Inno Setup 7\ISCC.exe" /DAppVersion=0.4.0 packaging\windows\personal-setup.iss
; The output goes to target\dist\personal.
;
; The app updates itself (crates/obsidian-core/src/update.rs) by running a
; newer one of these with /VERYSILENT: it closes the app, installs over it
; with the choices made the first time, and starts it again, with the
; arguments given as /LAUNCH=.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#define Root ".."
#define Src Root + "\..\target\release"

[Setup]
; The same AppId as before the rename, so an update replaces the old install.
AppId={{19FF8D53-8D7E-4E66-91BB-E2F5BE307E9A}
AppName=Discord Obsidian
AppVersion={#AppVersion}
AppVerName=Discord Obsidian {#AppVersion}
AppPublisher=dc
AppPublisherURL=https://github.com/yksaionara/dc
AppUpdatesURL=https://github.com/yksaionara/dc/releases
VersionInfoVersion={#AppVersion}
VersionInfoProductName=Discord Obsidian
VersionInfoDescription=Discord Obsidian Setup
PrivilegesRequired=lowest
; %LOCALAPPDATA%\Programs\Discord Obsidian, apart from the real Discord's
; folder; never the folder an install under the old name used.
DefaultDirName={autopf}\Discord Obsidian
UsePreviousAppDir=no
DisableProgramGroupPage=yes
DisableDirPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#Root}\..\target\dist\personal
OutputBaseFilename=Discord-Setup
SetupIconFile=discord.ico
UninstallDisplayIcon={app}\Discord Obsidian.exe
UninstallDisplayName=Discord Obsidian
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
CloseApplications=force
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Desktop shortcut"; GroupDescription: "{cm:AdditionalIcons}"
Name: "startonsignin"; Description: "Start automatically when Windows starts"; GroupDescription: "Startup:"

[Files]
Source: "{#Src}\obsidian.exe"; DestDir: "{app}"; DestName: "Discord Obsidian.exe"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Discord Obsidian"; Filename: "{app}\Discord Obsidian.exe"
Name: "{autodesktop}\Discord Obsidian"; Filename: "{app}\Discord Obsidian.exe"; Tasks: desktopicon

[InstallDelete]
; The Start menu folder the old name used.
Type: filesandordirs; Name: "{autoprograms}\Yapfast"

[Registry]
; The same Run value Settings > Advanced > "Start when you sign in" writes.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Discord Obsidian"; ValueData: """{app}\Discord Obsidian.exe"" --background"; Tasks: startonsignin
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"; ValueType: none; ValueName: "Discord Obsidian"; Flags: deletevalue dontcreatekey; Tasks: startonsignin
Root: HKCU; Subkey: "Software\Classes\obsidian"; ValueType: string; ValueName: ""; ValueData: "URL:Discord Obsidian Protocol"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\obsidian"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\obsidian\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\Discord Obsidian.exe"",0"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\obsidian\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\Discord Obsidian.exe"" --open-link ""%1"""; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Discord Obsidian"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"; ValueType: none; ValueName: "Discord Obsidian"; Flags: uninsdeletevalue dontcreatekey
; What the old name registered.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Yapfast"; Flags: deletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"; ValueType: none; ValueName: "Yapfast"; Flags: deletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Classes\yapfast"; ValueType: none; Flags: deletekey dontcreatekey

[Run]
Filename: "{app}\Discord Obsidian.exe"; Description: "Open Discord Obsidian now"; Flags: nowait postinstall skipifsilent
; A silent update starts the app again, as it was running.
Filename: "{app}\Discord Obsidian.exe"; Parameters: "{param:launch|}"; Flags: nowait; Check: WizardSilent

[UninstallRun]
Filename: "{app}\Discord Obsidian.exe"; Parameters: "--release-discord-links"; Flags: runhidden waituntilterminated; RunOnceId: "ReleaseDiscordLinks"

[Code]
// An install under the old name: its folder, which the update replaces.
function OldFolder(): String;
begin
  Result := ExpandConstant('{localappdata}\Programs\Yapfast');
end;

// Closes a copy running from the old folder, which the update then removes.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  if DirExists(OldFolder()) then
    Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM yapfast.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := '';
end;

// The old desktop shortcut, when it opens the old install rather than the
// real Discord.
procedure RemoveOldShortcut(Path: String);
var
  Shell, Shortcut: Variant;
begin
  if not FileExists(Path) then exit;
  try
    Shell := CreateOleObject('WScript.Shell');
    Shortcut := Shell.CreateShortcut(Path);
    if Pos('\programs\yapfast\', Lowercase(Shortcut.TargetPath)) > 0 then
      DeleteFile(Path);
  except
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    RemoveOldShortcut(ExpandConstant('{autodesktop}\Discord.lnk'));
    if DirExists(OldFolder()) then
      DelTree(OldFolder(), True, True, True);
  end;
end;
