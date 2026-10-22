; PEQUOD Host installer (Inno Setup 6). Built by .github/workflows/build-windows.yml:
;   iscc /DAppVersion=1.0.57 Installer\PEQUOD-Host.iss
; Installs for the current user (no admin prompt), so PEQUOD can update itself.

#ifndef AppVersion
  #define AppVersion "1.0.0"
#endif

[Setup]
AppId={{1FDD5205-C5ED-41B1-9C40-EFDAE2E86431}
AppName=PEQUOD Host
AppVersion={#AppVersion}
AppVerName=PEQUOD Host {#AppVersion}
AppPublisher=PEQUOD
DefaultDirName={autopf}\PEQUOD Host
DefaultGroupName=PEQUOD
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayName=PEQUOD Host
UninstallDisplayIcon={app}\PEQUOD Host.exe
OutputDir=..\dist
OutputBaseFilename=PEQUOD-Host-Setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupIconFile=PEQUOD-Host.ico
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "Put a PEQUOD Host shortcut on the desktop"; GroupDescription: "Shortcuts:"

[Files]
Source: "..\build\PequodHost_artefacts\Release\PEQUOD Host.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\Resources\Fonts\*.txt"; DestDir: "{app}\Licences"; Flags: ignoreversion
Source: "..\fw\*"; DestDir: "{app}\Controller firmware"; Flags: ignoreversion
Source: "..\Controller\README.md"; DestDir: "{app}\Controller firmware"; Flags: ignoreversion
Source: "..\docs\controller-protocol.md"; DestDir: "{app}\Controller firmware"; Flags: ignoreversion
Source: "..\Controller\PEQUOD_FC\*"; DestDir: "{app}\Controller firmware\source"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{autoprograms}\PEQUOD Host"; Filename: "{app}\PEQUOD Host.exe"
Name: "{autodesktop}\PEQUOD Host"; Filename: "{app}\PEQUOD Host.exe"; Tasks: desktopicon

[Run]
; normal install: offer to start it; update from inside PEQUOD (silent): start it again by itself
Filename: "{app}\PEQUOD Host.exe"; Description: "Start PEQUOD Host"; Flags: nowait postinstall skipifsilent
Filename: "{app}\PEQUOD Host.exe"; Flags: nowait; Check: WizardSilent
