; Discord Obsidian's Windows installer. scripts/package-windows.py compiles it with
; Inno Setup 7 under Wine, from the same files as the portable ZIP.
;
; It installs for the person running it, without administrator rights, and can
; start Discord Obsidian in the tray when they sign in. That is the Run key value
; Settings > Advanced > "Start when you sign in" writes (src/autostart.rs), so
; the installer and the setting turn the same thing on and off.

#ifndef AppVersion
  #error Compile through scripts/package-windows.py, which defines AppVersion, VersionNumber and SourceDir
#endif
; The full preview defines Edition and FileSuffix; the Discord release neither.
; Both share the AppId, so installing one replaces the other.
#ifdef Edition
  #define DisplayName "Discord Obsidian (" + Edition + ")"
#else
  #define DisplayName "Discord Obsidian"
  #define FileSuffix ""
#endif

[Setup]
; Windows knows an installed Discord Obsidian by this; never change it.
AppId={{19FF8D53-8D7E-4E66-91BB-E2F5BE307E9A}
AppName=Discord Obsidian
AppVersion={#AppVersion}
AppVerName={#DisplayName} {#AppVersion}
AppPublisher=Discord Obsidian
AppPublisherURL=https://obsidian.com
AppSupportURL=https://github.com/obsidian/obsidian/issues
AppUpdatesURL=https://github.com/obsidian/obsidian/releases
VersionInfoVersion={#VersionNumber}
; Per user: %LOCALAPPDATA%\Programs\Discord Obsidian, with no UAC prompt.
PrivilegesRequired=lowest
DefaultDirName={autopf}\Discord Obsidian
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputBaseFilename=Discord Obsidian-{#AppVersion}{#FileSuffix}-windows-x64-setup
SetupIconFile=obsidian.ico
UninstallDisplayIcon={app}\Discord Obsidian.exe
UninstallDisplayName={#DisplayName}
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
; Discord Obsidian keeps running in the tray with its window closed, so an update or
; uninstall closes it rather than asking.
CloseApplications=force
RestartApplications=no
#ifdef LicenseFile
LicenseFile={#LicenseFile}
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
english.StartOnSignIn=Start Discord Obsidian automatically when Windows starts
english.StartOnSignInGroup=Startup:

[Tasks]
Name: "startonsignin"; Description: "{cm:StartOnSignIn}"; GroupDescription: "{cm:StartOnSignInGroup}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; README.txt explains the ZIP; the installer needs no instructions.
Source: "{#SourceDir}\*"; Excludes: "README.txt"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Discord Obsidian"; Filename: "{app}\Discord Obsidian.exe"
Name: "{autodesktop}\Discord Obsidian"; Filename: "{app}\Discord Obsidian.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Discord Obsidian"; ValueData: """{app}\Discord Obsidian.exe"" --background"; Tasks: startonsignin
; Chosen here, it starts even if Task Manager had turned it off before.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"; ValueType: none; ValueName: "Discord Obsidian"; Flags: deletevalue dontcreatekey; Tasks: startonsignin
; Discord Obsidian's own links open here; claiming Discord's stays the person's choice.
Root: HKCU; Subkey: "Software\Classes\obsidian"; ValueType: string; ValueName: ""; ValueData: "URL:Discord Obsidian Protocol"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\obsidian"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\obsidian\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\Discord Obsidian.exe"",0"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\obsidian\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\Discord Obsidian.exe"" --open-link ""%1"""; Flags: uninsdeletekey
; Settings keeps the app discord:// links opened before claiming them here;
; uninstalling gives them back (below) and drops what was kept.
Root: HKCU; Subkey: "Software\Discord Obsidian"; Flags: uninsdeletekeyifempty dontcreatekey
Root: HKCU; Subkey: "Software\Discord Obsidian\DiscordProtocol"; Flags: uninsdeletekey dontcreatekey
; Uninstalling stops starting at sign-in, however it was turned on.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Discord Obsidian"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"; ValueType: none; ValueName: "Discord Obsidian"; Flags: uninsdeletevalue dontcreatekey

[Run]
Filename: "{app}\Discord Obsidian.exe"; Description: "{cm:LaunchProgram,Discord Obsidian}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Before the files go: discord:// links open in the app that had them again,
; if Settings claimed them for this Discord Obsidian.
Filename: "{app}\Discord Obsidian.exe"; Parameters: "--release-discord-links"; Flags: runhidden waituntilterminated; RunOnceId: "ReleaseDiscordLinks"
