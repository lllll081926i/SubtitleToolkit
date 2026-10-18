; SubtitleToolkit 安装器（28 §3）
;
; 用法（由 tools/make_installer.ps1 驱动，不要手工改路径）：
;   ISCC.exe /DStageDir=<打包目录> /DAppVersion=<版本> /DOutDir=<输出目录> subtitletool.iss
;
; 立场（写清楚，避免把"能编译出 setup.exe"当成"已发布"）：
; * 本脚本**不做 Authenticode 签名**（28 §5 仍未实现），所以产出的 setup.exe
;   在 SmartScreen 上会有警告。这不是缺陷，是未完成项，不许在文档里含糊过去。
; * 默认**不打包任何第三方运行库 DLL**：libass 那组 MSYS2 DLL 的来源/许可/分发义务
;   仍在审计中（third_party/README.md），未完成前不许混进发行物。
; * 安装器与便携包必须收**同一批文件**：两处各列一份清单，迟早会漂移，
;   而"安装版能跑、便携版缺文件"这类缺陷没人会立刻发现。清单由
;   tools/make_installer.ps1 从 make_portable.ps1 的同一条规则生成。

#define AppName "SubtitleToolkit"
; AppId 是**升级身份的根**：改了它，新版本会被当成另一个产品装第二份。
; 用 GUID 而不是产品名：产品名会改，GUID 不该改。
;
; 外层那对双花括号是 Inno [Setup] AppId 的**转义约定**：
; 写成 `{{...}}` 表示 AppId 的字面值是 `{...}`（单层）。
; 所以下面再单独定义一个**不带外壳**的宏给 Pascal 用 ——
; 在 Pascal 字符串里 `{{` 不会被转义，直接拼进键名会得到
; `Software\{{7E3A…}}` 这个与 [Setup] 无关的键。
#define AppId "{{7E3A9C41-5B2D-4F18-9A6E-1C0D8B7F4A52}"
#define AppIdRaw "{7E3A9C41-5B2D-4F18-9A6E-1C0D8B7F4A52}"
#define AppPublisher "SubtitleToolkit Dev"

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef StageDir
  #error StageDir 未定义：请用 tools/make_installer.ps1 驱动本脚本
#endif
#ifndef OutDir
  #define OutDir "."
#endif

[Setup]
AppId={#AppId}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
; per-user 默认（28 §3 第一条）：装到 %LOCALAPPDATA%\Programs 下，不弹 UAC。
; 这一条直接决定"用户能不能装"——要求管理员的安装器在很多机器上根本走不完。
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
; PrivilegesRequiredOverridesAllowed=dialog 让安装页出现"为所有用户安装"选项，
; 对应 28 §3 的"可选 all-users"：默认不需要管理员，但用户可以显式选。
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
; 卸载时保留用户数据由用户在卸载页选择（见 [Code] 的 UninstallKeepData）。
Uninstallable=yes
OutputDir={#OutDir}
OutputBaseFilename={#AppName}-{#AppVersion}-setup
Compression=lzma2/max
SolidCompression=yes
; 只有 64 位（ADR-001 的唯一正式 target）。
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; 许可正文随包：安装页显示 MIT（双许可里选一个给用户看，另一个随包附带）
LicenseFile={#StageDir}\LICENSE-MIT
; 安装器自身的最低 Windows 版本：与 Rust 1.85 的 msvc target 要求一致（Win10 1809+）
MinVersion=10.0.17763
WizardStyle=modern
; 未签名的安装器不该假装有签名信息
VersionInfoVersion={#AppVersion}
VersionInfoCompany={#AppPublisher}
VersionInfoProductName={#AppName}

[Languages]
Name: "chinese"; MessagesFile: "compiler:Default.isl"

[Tasks]
; 文件关联**默认不勾**（28 §3 第三条："默认不抢关联，安装页 opt-in"）。
; 抢关联是最容易招致反感的行为，而"让用户主动勾"是唯一的正当做法。
Name: "assoc"; Description: "把 .ass / .ssa / .srt / .sup / .xml 关联到 {#AppName}"; GroupDescription: "文件关联："; Flags: unchecked
Name: "assoc\ass"; Description: ".ass 高级字幕脚本"; Flags: unchecked
Name: "assoc\ssa"; Description: ".ssa 字幕脚本"; Flags: unchecked
Name: "assoc\srt"; Description: ".srt 纯文本字幕"; Flags: unchecked
Name: "assoc\sup"; Description: ".sup PGS 字幕流"; Flags: unchecked
Name: "assoc\xml"; Description: ".xml（BDN XML 字幕）"; Flags: unchecked
; legacy renderer support（28 §3 第四条）：装上 x86 宿主供 32 位 VSFilterMod 用。
; 默认不勾：它是可选组件。
; 这里保留任务项而不是删掉：删掉会让"想用的人找不到入口"，保留但在缺失时明确报错
; 才是诚实的做法（见 [Code] 的 LegacyRendererRequested）。即使勾了也不会因为
; 缺文件而让整个安装失败，而是写一条警告并跳过 —— 安装器不该因为一个可选组件而失败。
Name: "legacy"; Description: "安装 32 位渲染宿主（供 VSFilterMod 等 legacy 过滤器）"; GroupDescription: "可选组件："; Flags: unchecked
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "快捷方式："; Flags: unchecked

[Files]
; 程序文件：整个 stage 目录。清单由 make_installer.ps1 用与便携包**同一条规则**生成，
; 所以两处分发形态收的是同一批文件（见本文件顶部注释）。
; `Excludes` 必须与 `Source` 同一条目录项（Inno Setup 不支持续行）：
; portable.flag **不装** —— 安装版就该落 %LOCALAPPDATA%（28 §4）。
; 把它装进去会让安装版静默变成便携行为，而用户以为自己装了一个正常程序。
Source: "{#StageDir}\*"; DestDir: "{app}"; Excludes: "portable.flag"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\subtitletoolkit-gui.exe"
Name: "{group}\命令行工具"; Filename: "{app}\subtitletool-cli.exe"
Name: "{group}\卸载 {#AppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\subtitletoolkit-gui.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\subtitletoolkit-gui.exe"; Description: "启动 {#AppName}"; Flags: nowait postinstall skipifsilent

[Registry]
; 文件关联写 HKA（=HKCU 在 per-user 安装下、HKLM 在 all-users 下）。
; 用 HKA 而不是硬编码 HKLM：per-user 安装去写 HKLM 会静默失败（没权限），
; 于是"勾了关联但没生效"而安装器不报错。
Root: HKA; Subkey: "Software\Classes\.ass"; ValueType: string; ValueName: ""; ValueData: "{#AppName}.ass"; Flags: uninsdeletekey; Tasks: assoc\ass
Root: HKA; Subkey: "Software\Classes\.ssa"; ValueType: string; ValueName: ""; ValueData: "{#AppName}.ssa"; Flags: uninsdeletekey; Tasks: assoc\ssa
Root: HKA; Subkey: "Software\Classes\.srt"; ValueType: string; ValueName: ""; ValueData: "{#AppName}.srt"; Flags: uninsdeletekey; Tasks: assoc\srt
Root: HKA; Subkey: "Software\Classes\.sup"; ValueType: string; ValueName: ""; ValueData: "{#AppName}.sup"; Flags: uninsdeletekey; Tasks: assoc\sup
Root: HKA; Subkey: "Software\Classes\.xml"; ValueType: string; ValueName: ""; ValueData: "{#AppName}.xml"; Flags: uninsdeletekey; Tasks: assoc\xml

; 「用 X 打开」菜单项。命令里带上 %1 与引号：不带引号时含空格的路径会被拆成多个参数。
;
; progid 键本身也要用 `uninsdeletekey`（不是 `uninsdeletevalue`）：
; 只删值会把 `SubtitleToolkit.ass` 这个**空壳 progid** 留在注册表里，
; 而它正是 Windows 用来在“打开方式”列表里显示一项的地方 ——
; 卸载后那个列表里还会出现一个指向已删程序的条目。
Root: HKA; Subkey: "Software\Classes\{#AppName}.ass"; Flags: uninsdeletekey; Tasks: assoc\ass
Root: HKA; Subkey: "Software\Classes\{#AppName}.ass\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\subtitletoolkit-gui.exe"" ""%1"""; Tasks: assoc\ass
Root: HKA; Subkey: "Software\Classes\{#AppName}.ssa"; Flags: uninsdeletekey; Tasks: assoc\ssa
Root: HKA; Subkey: "Software\Classes\{#AppName}.ssa\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\subtitletoolkit-gui.exe"" ""%1"""; Tasks: assoc\ssa
Root: HKA; Subkey: "Software\Classes\{#AppName}.srt"; Flags: uninsdeletekey; Tasks: assoc\srt
Root: HKA; Subkey: "Software\Classes\{#AppName}.srt\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\subtitletoolkit-gui.exe"" ""%1"""; Tasks: assoc\srt
Root: HKA; Subkey: "Software\Classes\{#AppName}.sup"; Flags: uninsdeletekey; Tasks: assoc\sup
Root: HKA; Subkey: "Software\Classes\{#AppName}.sup\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\subtitletoolkit-gui.exe"" ""%1"""; Tasks: assoc\sup
Root: HKA; Subkey: "Software\Classes\{#AppName}.xml"; Flags: uninsdeletekey; Tasks: assoc\xml
Root: HKA; Subkey: "Software\Classes\{#AppName}.xml\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\subtitletoolkit-gui.exe"" ""%1"""; Tasks: assoc\xml

[UninstallDelete]
; 只删安装目录下的**可执行文件与随包文档**之外的东西不做处理：
; 用户数据（presets/logs）默认**保留**，由卸载页的问题决定（见 [Code]）。
; 这里不写 {app} 整目录：那会把用户放进安装目录的东西一起删掉。
; 真正需要清的是运行期产物（如果它们恰好落在安装目录）—— 由 [Code] 处理。

[Code]
// ---------------------------------------------------------------------------
// 28 §3 的第五条：**卸载保留用户 presets/logs 由用户选择**。
//
// ## 为什么那份选择必须写进注册表
//
// 第一版把结果存在一个 Pascal 变量里，而那是**安装器进程**的变量：
// `CurUninstallStepChanged` 跑在**卸载器进程**里，`InitializeWizard` 从来没在
// 那个进程里执行过——于是变量是未初始化的默认值（False），卸载**总是删数据**。
//
// 实测把这条抳了出来：卸载前 `%LOCALAPPDATA%\subtitletool` 有
// `fontdb-v1.json` 与日志，静默卸载（用户没答那个问题，默认应当是“保留”）之后
// 目录**不见了**。
//
// 所以选择必须落盘，而且卸载器要能读到它——注册表是唯一两进程都能访问且
// Inno 原生支持的地方。用 `HKCU` 而不是 `HKA`：卸载时不知道当初装成了
// per-user 还是 all-users，而 `HKCU` 两种情况下都写得到。
// ---------------------------------------------------------------------------
const
  KeepDataValue = 'KeepUserDataOnUninstall';

var
  KeepDataPage: TInputOptionWizardPage;

function UserDataDir(): String;
begin
  // 与 core_model::DataDir 的 UserProfile 分支一致（28 §4）：
  // %LOCALAPPDATA%\subtitletool
  Result := ExpandConstant('{localappdata}\subtitletool');
end;

function KeepDataKey(): String;
begin
  // 用 `AppIdRaw`（单花括号）而不是 `AppId`：
  //
  // `AppId` 是给 [Setup] 的，値里那对双花括号是 Inno 的转义约定，
  // 在 Pascal 字符串里不会被转义 —— 拼出来的会是 `Software\{{7E3A…}}`，
  // 一个与 [Setup] 毫无关系的键。写在那个位置是自洽的（写与读都在同一处），
  // 所以“能读回来”骗过了第一次自检；只有**卸载**时比对安装器写的那个键才看得出不对。
  Result := 'Software\' + '{#AppIdRaw}';
end;

// 读回当初的选择。**读不到就是“保留”**（与安装页的默认一致）：
// 旧版本装的、或注册表被清过的，都应当保守地不删用户数据。
function ShouldKeepUserData(): Boolean;
var
  V: String;
begin
  if not RegQueryStringValue(HKCU, KeepDataKey(), KeepDataValue, V) then
    Result := True
  else
    Result := (V <> '0');
end;

procedure InitializeWizard();
begin
  KeepDataPage := CreateInputOptionPage(wpSelectTasks,
    '用户数据', '卸载时如何处理用户数据？',
    '设置、日志与字体库缓存默认保留，便于以后重装时接着用。',
    True, False);
  KeepDataPage.Add('保留用户数据（设置、日志、字体库缓存）');
  KeepDataPage.Add('删除用户数据');
  KeepDataPage.Values[0] := True;
  KeepDataPage.Values[1] := False;
end;

// 安装时把选择写下来（卸载器没有安装页，只能靠这个）。
// 放在 ssPostInstall：那时安装已经成功，不会因为安装失败而留下一个“删数据”的选择。
procedure CurStepChanged(CurStep: TSetupStep);
var
  LegacyHost: String;
  Keep: String;
begin
  if CurStep <> ssPostInstall then
    Exit;

  Keep := '1';
  if KeepDataPage.Values[1] then
    Keep := '0';
  RegWriteStringValue(HKCU, KeepDataKey(), KeepDataValue, Keep);

  // 28 §3 的第四条：legacy renderer support。
  //
  // 这个警告的存在理由：x86 宿主是**可选组件**，将来可能因为目标机缺 i686 工具链
  // 而没随包提供。勾了却拿不到文件时：不静默跳过（用户以为装上了），
  // 也不让整个安装失败（为一个可选组件废掉主功能）—— 写一条显式警告。
  if WizardIsTaskSelected('legacy') then
  begin
    LegacyHost := ExpandConstant('{app}\subtitletool-render-host-x86.exe');
    if not FileExists(LegacyHost) then
      MsgBox('32 位渲染宿主（subtitletool-render-host-x86.exe）未随本安装包提供，' +
             '该项已跳过。64 位渲染不受影响。', mbInformation, MB_OK);
  end;
end;

// 卸载时按当初的选择处理。
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  DataDir: String;
begin
  if CurUninstallStep <> usUninstall then
    Exit;

  if not ShouldKeepUserData() then
  begin
    DataDir := UserDataDir();
    if DirExists(DataDir) then
      DelTree(DataDir, True, True, True);
  end;

  // 选择本身在卸载后已无意义，**两种分支下都要清掉**：
  // 不留一个指向已卸载产品的注册表值。
  // （第一版把这两行放在 `if keep then Exit` 之后，于是“保留”那条分支
  //   永远走不到它们 —— 值留下来而没人知道它是干什么的。）
  //
  // `RegDeleteKeyIfEmpty` 是 Inno 自带的（键非空时返回 False 而不报错），
  // 所以不需要自己写包装。
  RegDeleteValue(HKCU, KeepDataKey(), KeepDataValue);
  RegDeleteKeyIfEmpty(HKCU, KeepDataKey());
end;
