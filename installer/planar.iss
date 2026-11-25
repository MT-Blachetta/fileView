; Inno Setup script for the Planar Windows installer.
; Built by scripts\build-windows.ps1, which passes the version and, when a
; certificate is configured, the signing command:
;   ISCC /DAppVersion=0.1.0 [/DSign /Splanarsign=...] installer\planar.iss
;
; Installs per user into %LOCALAPPDATA%\Programs\Planar without asking for
; administrator rights; the first page lets the user switch to an
; all-users install instead.

#ifndef AppVersion
  #error Pass the version: ISCC /DAppVersion=x.y.z installer\planar.iss
#endif
#ifndef FileVersion
  #define FileVersion AppVersion
#endif
#ifndef SourceExe
  #define SourceExe "..\target\release\planar.exe"
#endif

[Setup]
AppId={{50F8CC37-4BF3-449E-BEC2-C61B9E517250}
AppName=Planar
AppVersion={#AppVersion}
AppVerName=Planar {#AppVersion}
AppPublisher=Michael Blachetta
AppPublisherURL=https://github.com/MT-Blachetta/fileView
AppSupportURL=https://github.com/MT-Blachetta/fileView/issues
AppUpdatesURL=https://github.com/MT-Blachetta/fileView/releases
AppCopyright=Copyright (c) 2026 Michael Blachetta. MIT License.
VersionInfoVersion={#FileVersion}
VersionInfoProductName=Planar
VersionInfoDescription=Planar Setup
VersionInfoCompany=Michael Blachetta

DefaultDirName={autopf}\Planar
DefaultGroupName=Planar
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0

WizardStyle=modern
SetupIconFile=..\assets\planar.ico
UninstallDisplayIcon={app}\planar.exe
UninstallDisplayName=Planar
CloseApplications=yes

OutputDir=..\dist
OutputBaseFilename=Planar-{#AppVersion}-x64-setup
Compression=lzma2/max
SolidCompression=yes

#ifdef Sign
SignTool=planarsign
SignedUninstaller=yes
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "german"; MessagesFile: "compiler:Languages\German.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "planar.exe"; Flags: ignoreversion
Source: "..\assets\Inter-LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Planar"; Filename: "{app}\planar.exe"
Name: "{autodesktop}\Planar"; Filename: "{app}\planar.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\planar.exe"; Description: "{cm:LaunchProgram,Planar}"; Flags: nowait postinstall skipifsilent
