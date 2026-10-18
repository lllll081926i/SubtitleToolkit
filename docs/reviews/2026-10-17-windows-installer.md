# 裁决：Windows 安装器（28 §3）

日期：2026-10-17
范围：`tools/subtitletool.iss`、`tools/make_installer.ps1`、`tools/verify_installer.ps1`
规格：`docs/development/28_WINDOWS_BUILD_RELEASE.md` §3

## 1 交付了什么

一个 Inno Setup 脚本 + 一个驱动脚本 + 一个验证脚本：

| 文件 | 作用 |
| --- | --- |
| `tools/subtitletool.iss` | 安装器本体。五条 §3 要求全部在这里表达 |
| `tools/make_installer.ps1` | 找 ISCC、读版本、搭 stage、编译、报哈希。**编译不出就退非零** |
| `tools/verify_installer.ps1` | 把 28 §3 的五条要求真的跑一遍，逐项报 PASS/FAIL |

产物：`D:\rust\target\installer\SubtitleToolkit-0.1.0-setup.exe`，6.15 MiB。

`make_installer.ps1` 的两条硬要求：

1. **找不到 ISCC 就失败**，不回退到「跳过」—— 一个静默跳过的构建脚本会让人以为装
   机器出来了，而实际什么都没有。
2. **`ISCC` 退非零就 throw**，不吞返回值。

## 2 五条要求逐条怎么落

### 2.1 per-user 默认、不要求管理员

`DefaultDirName={autopf}\SubtitleToolkit` + `PrivilegesRequired=lowest`。

`{autopf}` 在 per-user 模式下展开为 `%LOCALAPPDATA%\Programs`。这一条直接决定
「用户能不能装上」—— 要求管理员的安装器在很多机器上根本走不完。

**验证**：静默安装（`/VERYSILENT`，不弹 UAC）退出码 0，三个 exe 全在。

不给 UAC 提示的静默安装能跑完，本身就是「没要求提权」的证据 —— 要提权的话
Inno 会直接失败退出（静默下无法应答 UAC）。

### 2.2 可选 all-users

`PrivilegesRequiredOverridesAllowed=dialog` 让安装页出现「为所有用户安装」。

**验证状态**：**只核对了静态声明**，没做实机 UAC 验证。静默环境应答不了 UAC
对话框，而提权安装会真的写 `HKLM` 与 `%ProgramFiles%` —— 那是对开发机的实质
改动。这条的验证强度低于其余四条，如实记在这里。

### 2.3 文件关联 opt-in

五类扩展名（ass/ssa/srt/sup/xml）各一个独立任务项，每项 `Flags: unchecked`。

「不抢」不只是「默认没勾」：`/TASKS=assoc\ass` 静默装时**只**写 `.ass`，
另外四类一个不写。这一条验过。

关联命令写成 `"{app}\subtitletoolkit-gui.exe" "%1"` —— 两处引号都必要。
不带引号的路径遇到含空格的目录会被拆成多个参数。

**卸载要清干**：`[Registry]` 里 `.ass` 这类扩展名键用 `uninsdeletekey`
（不是 `uninsdeletevalue`），progid 键（`Software\Classes\SubtitleToolkit.ass`）
**单独也写一条 `uninsdeletekey`**。

> 第一版漏了 progid 那条（只在 `...\shell\open\command` 上写了键），结果卸载后
> `SubtitleToolkit.ass` 这个**空壳 progid** 留在注册表里。它是 Windows 用来在
> 「打开方式」列表里显示一项的地方 —— 用户卸载之后，那个列表里还会出现一个
> 指向已删程序的条目。

### 2.4 legacy 32 位宿主可选

任务项 `legacy`，默认不勾。勾了但文件不存在时 **不静默跳过**（用户会以为装上了），
也 **不让整个安装失败**（为一个可选组件废掉主功能）—— 弹一条显式说明。

x86 宿主当前尚未构建（28 §2），所以这条现在是「会走到」的路径。

### 2.5 卸载保留用户数据由用户选择

安装页给两个单选：保留（默认）／删除。卸载器按当初的选择处理。

**这一条第一版是错的，而且是「看起来对」的错法。** 记下原因，因为它是一个通用陷阱：

第一版把选择存在一个 Pascal 变量里：

```pascal
var KeepUserData: Boolean;
procedure InitializeWizard();
begin
  KeepUserData := True;
  ...
end;
function NextButtonClick(CurPageID: Integer): Boolean;
begin
  if CurPageID = KeepDataPage.ID then
    KeepUserData := KeepDataPage.Values[0];
end;
```

`CurUninstallStepChanged` 跑在**卸载器进程**里，而 `InitializeWizard` 从来没在
那个进程里执行过 —— 变量是未初始化的 `False`，于是**卸载总是删数据**。

实测把它抳了出来：卸载前 `%LOCALAPPDATA%\subtitletool` 有 `fontdb-v1.json`，
静默卸载（用户没答那个问题，默认应当是「保留」）之后目录**不见了**。

修法是让选择**落盘**，因为卸载器要跨进程读到它。`HKCU\Software\{AppId}` 下的
`KeepUserDataOnUninstall`，安装时在 `ssPostInstall` 写。

三个附带决定：

1. **用 `HKCU` 而不是 `HKA`**：卸载时不知道当初装成了 per-user 还是 all-users，
   而 `HKCU` 两种情况下都写得到。
2. **读不到值就当「保留」**：旧版本装的、或注册表被清过的，应当保守地不删用户数据。
   这与安装页的默认一致，不会出现「默认保留但旧安装变成删」。
3. **写在 `ssPostInstall`**：那时安装已经成功，不会因为安装中途失败而留下一个
   「删数据」的选择。

**另一个真缺陷**：清那个注册表值的两行原本放在 `if keep then Exit;` **之后**，
于是「保留」那条分支永远走不到它们 —— 值留下来而没人知道它是干什么的。
现在两种分支下都清，并且顺带删掉空的容器键（`RegDeleteKeyIfEmpty`）。

> 顺带一提：第一版为「键为空才删」写了个自己的函数叫 `RegDeleteKey`，而 Inno 里
> 根本没有这个名字（编译报 `Unknown identifier`）。`RegDeleteKeyIfEmpty` 是
> **Inno 自带的**，不需要包装。

## 3 花括号陷阱（值得单独记）

`#define AppId "{{7E3A9C41-...}}"` 里那对双花括号是 **Inno `[Setup]` AppId 的转义
约定**：`{{` 表示 AppId 的字面值是 `{`。所以 `AppId={#AppId}` 得到的真实 AppId 是
`{7E3A...}`（单层）。

但 **Pascal 字符串里 `{{` 不转义**。`'Software\{#AppId}'` 拼出来的是
`Software\{{7E3A...}}` —— 一个与 `[Setup]` 毫无关系的键。

这个错法的**危险之处在于它自洽**：写和读都在同一个错位置，所以「能读回来」，
自检会通过。只有拿**安装器写的那个键**去比对才看得出不对。

修法：另立一个 `#define AppIdRaw "{7E3A...}"` 给 Pascal 用。

这一条也是 `verify_installer.ps1` 存在的理由 —— 手写的一次性核对没抓到它。

## 4 未完成项

- **Authenticode 签名（28 §5）没做**。未签名的 `setup.exe` 在 SmartScreen 上会
  警告。`make_installer.ps1` 结束时**每次**都把这条打出来，不让它被忘掉。
  做它需要一个代码签名证书，那不是代码能解决的问题。
- **Inno Setup 版本是 6.7.3，不是规格建议的 7**。用到的语言特性（
  `CreateInputOptionPage`、`RegDeleteKeyIfEmpty`、`PrivilegesRequiredOverridesAllowed`、
  `WizardIsTaskSelected`）在 6 里都有，编译与实跑都通过。**不假装用了 7**。
- **本包不含第三方运行库 DLL**（与 `make_portable.ps1` 同一立场）。需要渲染时
  在目标机上设 `STK_LIBASS_DLL`。
- **x86 宿主没构建**，见 §2.4。
- **all-users 模式未实机验证**，见 §2.2。

## 5 验证记录（可复现）

```
. .\tools\set_build_cache.ps1
.\tools\make_installer.ps1
pwsh -NoProfile -File .\tools\verify_installer.ps1
```

结果：**11 项检查，11 通过，0 失败**。

```
== 第一条：per-user 默认，不要求管理员 ==
  [PASS] 静默安装退出码 0（未触发 UAC）  — exit=0
  [PASS] 三个可执行文件都已安装
== 第二条：可选 all-users ==
  [PASS] 声明了 PrivilegesRequired=lowest + 可覆盖  — 静态声明，未做 UAC 实机验证
== 第三条：文件关联默认不抢（opt-in） ==
  [PASS] .ass 关联未被默认写入
  [PASS] .srt 关联未被默认写入
== 第四条：legacy 32 位宿主是可选 ==
  [PASS] 默认未装 x86 宿主
  [PASS] iss 里有 legacy 任务项且默不勾
== 第五条：卸载保留用户数据由用户选择 ==
  [PASS] 安装器记下了保留选择  — KeepUserDataOnUninstall=1
  [PASS] 默认卸载后用户数据仍在
  [PASS] 卸载器清掉了自己的注册表值
  [PASS] 选「删除」后用户数据被清掉
```

关联 opt-in 那一段由手工单独跑（`/TASKS=assoc\ass`），结果：

```
1. .ass -> SubtitleToolkit.ass
2. 打开命令: "...\subtitletoolkit-gui.exe" "%1"
3. 只勾 ass，其余四类均未写: True
4. 卸载后 .ass 键已清: True
5. 卸载后 progid 空壳也已清: True
6. 卸载后用户数据保留: True
```

### 验证脚本自己的三条纪律

写成脚本而不是一次性命令，是因为手工核对的版本没抓到花括号陷阱。脚本里有三条
刻意的设计：

1. **验证前备份并恢复 `%LOCALAPPDATA%\subtitletool`**。开发机上那份可能是用户真在
   用的数据，验证脚本没有资格毁掉它。
2. **查「默认不写关联」之前先清关联键**。上一次崩掉的验证脚本可能留下残留，
   在残留上查「没写」是假 PASS。
3. **安装目录用 `target\tmp\installer-verify-<随机>`**，不写真实的
   `%LOCALAPPDATA%\Programs`。

不把它写成 Rust 集成测试：安装/卸载要起真进程、写 `HKCU` 与 `%LOCALAPPDATA%`，
必须静默跑。Rust 测试跑这个会污染开发机的真实注册表与用户目录，而 CI 上又没有
装 Inno Setup。一个显式的、开发者手动跑的脚本，比一条会留下副作用的「自动化测试」
诚实。
