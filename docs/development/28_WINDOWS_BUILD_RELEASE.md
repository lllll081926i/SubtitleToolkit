# Windows 构建、安装、签名与发布

本文件是发布阶段的目标清单，不是当前发行物清单。当前 workspace 只定义 CLI 与 x64 `subtitletool-render-host.exe`；GUI、x86 host、安装包、签名和更新器均未实现。只有经过实际构建与验收的产物才能加入支持矩阵。

**已落地（2026-10-17）**：根目录许可证正文 `LICENSE-MIT` / `LICENSE-APACHE`（此前为缺失状态）；`tools/make_portable.ps1` 产出自包含 portable zip（含 `SHA256SUMS.txt`、许可/第三方声明副本、`sbom.cdx.json` 与 `portable.flag`）。已实测：解包后三个可执行文件可运、`data_dir.kind=portable`、缓存落在包内 `data/`、包内 9 个文件的哈希全部与 `SHA256SUMS.txt` 一致。安装器与 Authenticode 签名仍未完成。

**SBOM（2026-10-17）**：`tools/notices.py` 现在同时产出 `sbom.cdx.json`（CycloneDX 1.5，529 个组件）。
决策与纪律：

- 选 CycloneDX 而不是 SPDX：它原生表达“组件 + 哈希 + 许可证”这三件本仓库真正持有的事实
  （正是 21 §11 要求的三样），且字段少到能用一份自己的代码写对——不引第三方依赖。
- `scope` 把 `required` 与 `excluded` 分开：开发期工具（cargo-deny、vcpkg）与研究用源码归档
  不进安装包，列成 runtime 会让下游误判分发义务。口径与 `THIRD_PARTY_NOTICES.md` 一致。
- native/二进制产物带 `stk:hash-status`：`match`（本机实测相等）与 `absent-here`
  （这台机器上没有那个文件）**分开写**，混起来就是拿未实测的值当已核对。本机实测 23 match / 6 absent-here。
- 产品版本从根 `Cargo.toml` 读，不手抄；`serialNumber` 由 `Cargo.lock` 哈希派生，
  同一份依赖树两次生成得到同一标识。
- **门禁会查它过期**：组件数对不上或版本不一致就报错退 1。已实测：把 SBOM 改成 5 个组件，
  门禁报 `SBOM 过期：有 5 个组件，当前依赖树应该是 529 个`。

以下文件名是发行包候选命名；只有 `tools/make_portable.ps1` 实际生成的名字才算数。Cargo 产物名以各 crate 的 `[[bin]]` 配置为准。

当前构建命令见[开发说明](../../CONTRIBUTING.md)。PowerShell 先点源 `tools/set_build_cache.ps1`，所有构建、下载与临时缓存使用非 C 盘。原生 libass 开发期运行库已登记并实测，来源与产品分发边界见[第三方说明](../../third_party/README.md)。

## 1. Toolchain
- Rust stable MSVC x64；
- Visual Studio Build Tools + Windows SDK；
- native dependency build 用 CMake/Meson 时锁版本；
- `cargo-deny`, `cargo-audit`, `cargo-nextest`。

## 2. Artifacts
- `SubtitleToolkit.exe`
- `subtitletool-cli.exe`
- `subtitletool-render-host.exe`（当前 x64 Cargo 产物）
- `subtitletoolkit-gui.exe`（Iced 工作台；已接入真 batch 通路，见 [GUI 接通裁决](../reviews/2026-10-17-gui-backend-wiring.md)）
- `renderer-host-x86.exe`（可选组件）
- bundled permissive DLLs
- installer `.exe`
- portable `.zip`
- symbols 独立 archive
- checksums + SBOM + third-party notices

## 3. Installer
推荐 Inno Setup 7：
- per-user 默认，不要求管理员；
- 可选 all-users；
- 文件关联：ass/ssa/srt/sup/xml（默认不抢关联，安装页 opt-in）；
- 可选 legacy renderer support；
- 卸载保留用户 presets/logs 由用户选择。

## 4. Portable

**已实现**：`tools/make_portable.ps1`（用法见 [tools/README](../tools/README.md)）。所有程序文件放同目录，并带 `SHA256SUMS.txt` 与许可正文。

**已实现（2026-10-17）**：`portable.flag` → `./data` 的便携数据目录。

- 可执行文件旁边存在 `portable.flag` 时，运行期状态（字体库缓存 `fontdb-v1.json` 等）写在同级 `data/`；
  没有这个文件就退回 `%LOCALAPPDATA%\subtitletool`（安装版行为）。
- 用"标记文件"而不是嗅探可移动盘：便携是**用户的选择**，不是我们猜出来的属性；
  自动嗅探会让同一份 exe 在不同机器上落到不同数据目录，而用户以为两边看的是同一份设置。
- 优先级：`STK_DATA_DIR` > `portable.flag` > `%LOCALAPPDATA%\subtitletool`。
- 取的是 **exe 所在目录**而不是当前工作目录：从任意目录启动便携包，数据都落在 exe 旁边。
- `doctor --json` 的 `data_dir` 报出路径、来源 kind（`explicit`/`portable`/`user-profile`/`unavailable`）
  与判定理由，`font_cache` 报出缓存路径；"便携没生效"这类反馈能一眼定位。
- `make_portable.ps1` **默认在包里生成 `portable.flag`**，解包即用；
  删掉它就会退回安装版行为。校验和现在覆盖包内**所有文件**（含许可正文与 flag），不只是 exe。

已实测：解包目录内 `subtitletool-cli.exe doctor --json` 返回 `ok=true` 且
`data_dir.kind=portable`；`font scan` 后缓存确实落在 `<解包目录>/data/fontdb-v1.json`；
包内每个文件的 SHA-256 与 `SHA256SUMS.txt` 一致。回归用例：`crates/cli/tests/portable.rs`。

仍未实现：settings/presets/logs 的落盘（目前只有字体库缓存走数据目录）。

## 5. Signing
正式 release：所有 EXE/DLL/installer 用同一 publisher identity Authenticode；timestamp server；CI 验证签名后再上传。

## 6. Updates
第一版可只“检查更新并打开 release 页面”；成熟后做 signed manifest + staged download。不要在早期自写高权限 auto-updater。

## 7. CI
Windows x64 runner：fmt/clippy/test/nextest/fuzz smoke/build/E2E。x86 helper 单独 job。Nightly corpus test 可较重。

本节为 CI 目标，不代表已有 nextest/fuzz/GUI E2E 或 x86 job。仅将实际实现与验收的功能加入对应发行物清单；CLI portable 可独立于 GUI 安装包推进。
