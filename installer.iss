; Clutch installer for Inno Setup: iscc /DAppVersion=1.2.3 installer.iss
; Installs for the current user, without elevation. A newer installer replaces the installed version.

#ifndef AppVersion
  #error Pass the version with /DAppVersion=x.y.z
#endif
#ifndef Executable
  #define Executable "target\release\clutch.exe"
#endif
#define AppName "Clutch"
#define AppExe "Clutch.exe"
#define AppUrl "https://github.com/danisommer/clutch"
#define DataDir "{localappdata}\Clutch"

[Setup]
AppId={{4AAECB58-2C7F-406E-8B8F-B812C026C114}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=danisommer
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
AppMutex=Local\Clutch.Window,Local\Clutch.Watcher
DefaultDirName={autopf}\{#AppName}
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
MinVersion=10.0
ShowLanguageDialog=auto
WizardStyle=modern
SetupIconFile=assets\icon.ico
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\{#AppExe}
VersionInfoVersion={#AppVersion}
OutputDir=target\release
OutputBaseFilename=Clutch-Setup
Compression=lzma2/max
SolidCompression=yes

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "ptbr"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"

[Messages]
en.SetupAppRunningError=%1 is open, or Game Mode or Eco Mode is on.%n%nClose the %1 window and, if a mode is on, end it first. Then click OK to continue, or Cancel to exit.
en.UninstallAppRunningError=%1 is open, or Game Mode or Eco Mode is on.%n%nClose the %1 window and, if a mode is on, end it first. Then click OK to continue, or Cancel to exit.
ptbr.SetupAppRunningError=O %1 está aberto, ou o Modo Jogo ou o Modo Economia está ligado.%n%nFeche a janela do %1 e, se um modo estiver ligado, encerre-o antes. Depois clique em OK para continuar ou em Cancelar para sair.
ptbr.UninstallAppRunningError=O %1 está aberto, ou o Modo Jogo ou o Modo Economia está ligado.%n%nFeche a janela do %1 e, se um modo estiver ligado, encerre-o antes. Depois clique em OK para continuar ou em Cancelar para sair.

[CustomMessages]
en.SessionActive=Game Mode or Eco Mode is still on.%n%nOpen {#AppName} and end it, so the computer goes back to the way it was. Then uninstall again.
en.RemoveData=Also remove the settings, history and custom cleanup rules of {#AppName}?
ptbr.SessionActive=O Modo Jogo ou o Modo Economia ainda está ligado.%n%nAbra o {#AppName} e encerre o modo, para o computador voltar a como estava. Depois desinstale de novo.
ptbr.RemoveData=Remover também as configurações, o histórico e as regras de limpeza personalizadas do {#AppName}?

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Executable}"; DestDir: "{app}"; DestName: "{#AppExe}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
function UserData(): String;
begin
  Result := ExpandConstant('{#DataDir}');
end;

function InitializeUninstall(): Boolean;
begin
  Result := not FileExists(UserData() + '\session.json');
  if not Result then
    SuppressibleMsgBox(CustomMessage('SessionActive'), mbError, MB_OK, IDOK);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usPostUninstall) and (not UninstallSilent()) and DirExists(UserData()) then
    if MsgBox(CustomMessage('RemoveData'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
      DelTree(UserData(), True, True, True);
end;
