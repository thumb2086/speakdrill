; speakdrill Inno Setup installation script
; 用法：iscc /DVersion=0.2.47 installer\speakdrill.iss

#ifndef Version
  #define Version "0.0.0"
#endif

[Setup]
AppName=speakdrill
AppVersion={#Version}
AppPublisher=speakdrill contributors
DefaultDirName={autopf64}\speakdrill
DefaultGroupName=speakdrill
DisableDirPage=no
DisableProgramGroupPage=no
PrivilegesRequired=admin
OutputBaseFilename=speakdrill-setup-{#Version}
OutputDir=..\
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
Source: "..\target\release\speakdrill.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\speakdrill-gui.exe"; DestDir: "{app}"; Flags: ignoreversion

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional icons:"; Flags: unchecked

[Icons]
Name: "{group}\speakdrill"; Filename: "{app}\speakdrill.exe"
Name: "{group}\speakdrill GUI"; Filename: "{app}\speakdrill-gui.exe"
Name: "{group}\Uninstall speakdrill"; Filename: "{uninstallexe}"
Name: "{autodesktop}\speakdrill"; Filename: "{app}\speakdrill.exe"; Tasks: desktopicon

[Registry]
Root: HKLM; Subkey: "SOFTWARE\speakdrill"; ValueType: dword; ValueName: "Installed"; ValueData: 1; Flags: uninsdeletekey

[Run]
Filename: "{app}\speakdrill-gui.exe"; Description: "Launch speakdrill GUI"; Flags: postinstall skipifsilent nowait

[UninstallDelete]
Type: filesandordirs; Name: "{app}"