; Our installer: installs target\release\yapfast.exe as "Discord" for the
; current user, without administrator rights. scripts\ship.ps1 compiles it
; with the version in Cargo.toml; by hand:
;   "C:\Program Files\Inno Setup 7\ISCC.exe" /DAppVersion=0.4.0 packaging\windows\personal-setup.iss
; The output goes to target\dist\personal.
;
; The app updates itself (crates/yapfast-core/src/update.rs) by running a
; newer one of these with /VERYSILENT: it closes the app, installs over it
; with the choices made the first time, and starts it again, with the
; arguments given as /LAUNCH=.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#define Root ".."
#define Src Root + "\..\target\release"

[Setup]
; The same AppId as Yapfast's own installer, so either replaces the other.
AppId={{19FF8D53-8D7E-4E66-91BB-E2F5BE307E9A}
AppName=Discord
AppVersion={#AppVersion}
AppVerName=Discord {#AppVersion}
AppPublisher=dc
AppPublisherURL=https://github.com/yksaionara/dc
AppUpdatesURL=https://github.com/yksaionara/dc/releases
VersionInfoVersion={#AppVersion}
PrivilegesRequired=lowest
; %LOCALAPPDATA%\Programs\Yapfast, apart from the real Discord's folder.
DefaultDirName={autopf}\Yapfast
DisableProgramGroupPage=yes
DisableDirPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#Root}\..\target\dist\personal
OutputBaseFilename=Discord-Setup
SetupIconFile=discord.ico
UninstallDisplayIcon={app}\yapfast.exe
UninstallDisplayName=Discord (dc)
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
CloseApplications=force
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Desktop shortcut named ""Discord"" (replaces one already there)"; GroupDescription: "{cm:AdditionalIcons}"
Name: "startonsignin"; Description: "Start automatically when Windows starts"; GroupDescription: "Startup:"

[Files]
Source: "{#Src}\yapfast.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; In a folder, so it doesn't replace the real Discord's Start menu entry.
Name: "{autoprograms}\Yapfast\Discord"; Filename: "{app}\yapfast.exe"
Name: "{autodesktop}\Discord"; Filename: "{app}\yapfast.exe"; Tasks: desktopicon

[Registry]
; The same Run value Settings > Advanced > "Start when you sign in" writes.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Yapfast"; ValueData: """{app}\yapfast.exe"" --background"; Tasks: startonsignin
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"; ValueType: none; ValueName: "Yapfast"; Flags: deletevalue dontcreatekey; Tasks: startonsignin
Root: HKCU; Subkey: "Software\Classes\yapfast"; ValueType: string; ValueName: ""; ValueData: "URL:Yapfast Protocol"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\yapfast"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\yapfast\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\yapfast.exe"",0"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\yapfast\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\yapfast.exe"" --open-link ""%1"""; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Yapfast"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"; ValueType: none; ValueName: "Yapfast"; Flags: uninsdeletevalue dontcreatekey

[Run]
Filename: "{app}\yapfast.exe"; Description: "Open Discord now"; Flags: nowait postinstall skipifsilent
; A silent update starts the app again, as it was running.
Filename: "{app}\yapfast.exe"; Parameters: "{param:launch|}"; Flags: nowait; Check: WizardSilent

[UninstallRun]
Filename: "{app}\yapfast.exe"; Parameters: "--release-discord-links"; Flags: runhidden waituntilterminated; RunOnceId: "ReleaseDiscordLinks"
