# SubtitleToolkit

SubtitleToolkit 是面向 Windows 的字幕转换与制作工具，提供 Rust CLI 和 Iced 桌面界面。它支持 SRT、ASS/SSA、PGS、BDN 工作流，处理时间、帧率、分辨率、HDR 颜色、字体和批量任务，并通过隔离的 renderer host 调用 libass。

> **项目状态：开发预览版。** 核心转换、真实 libass 渲染、PGS/BDN 基础链路、GUI 设置与 TOML 预设导入/导出、便携包和 Inno Setup 安装器已实现。Authenticode 签名、x86 真实渲染、完整 Strict Blu-ray 设备验收及字体复杂脚本图像语料仍未完成。功能与验证边界见[当前状态](docs/STATUS.md)和[功能矩阵](docs/specs/03_FEATURE_MATRIX.md)。

## 功能

- 读取、转换和保留 ASS/SSA 与 SRT 字幕数据；调整时间、帧率、分辨率和 HDR 颜色。
- 通过 libass 生成 BDN XML/PNG 或 PGS SUP，并使用独立 renderer host 隔离 native 渲染器。
- 批量执行多输入、多 profile 任务；提供字体扫描、字体子集、MKV 辅助、配置预设和诊断包命令。
- 使用 Windows Iced GUI 管理任务，可保存全局日志/帧率/画布设置，并导入或导出 TOML 输出预设；CLI 支持脚本化操作。

部分操作需要外部工具：视频探测需要 `ffprobe`，MKV 操作需要 MKVToolNix。PGS 严格设备兼容仍需更多真实播放器与设备语料；本项目不提供视频转码、烧录或完整 Blu-ray authoring。

## 运行要求

- Windows 10/11 x64。
- 从源码构建：Rust stable（最低版本见 `Cargo.toml`）、Visual Studio C++ Build Tools 和 Windows SDK。
- 构建 Inno Setup 安装包：Inno Setup 6。

便携包和安装包已附带经官方 MSYS2 UCRT64 包哈希核验的 libass 及 19 个依赖 DLL、fontconfig 配置和逐包许可证材料。程序会在包内检测到 `libass-runtime.flag` 时自动加载运行库。开发或测试时可以用 `STK_LIBASS_DLL` 指定外部 `libass-9.dll`。来源、版本、SHA-256 和许可证见[运行库锁](third_party/LIBASS_RUNTIME_LOCK.json)及[第三方说明](third_party/README.md)。

Rust crate 版本由 `Cargo.lock` 锁定，首次构建需要 Cargo 访问 crates.io。`ffprobe` 与 MKVToolNix 是可选外部工具，不包含在安装包中；可用 `STK_FFPROBE` 和 `STK_MKVTOOLNIX_DIR` 指定位置。

## 快速开始

### 启动开发 GUI

在仓库根目录使用 PowerShell：

```powershell
.\dev.ps1
```

脚本以 Cargo Debug 模式启动 GUI，默认输出 `debug` 级日志到 `target/dev-data/logs/`。提高日志详细度：

```powershell
.\dev.ps1 -LogLevel trace
```

如果需要在开发模式中启用 libass，传入包含 `libass-9.dll` 和 `etc/fonts` 的运行库目录：

```powershell
.\dev.ps1 -LibassRuntimeDir 'D:\dependencies\libass-runtime-x64'
```

### 构建与使用 CLI

在 PowerShell 中先初始化项目构建缓存，再构建并运行：

```powershell
. .\tools\set_build_cache.ps1
cargo build --workspace --release
cargo run -p cli -- doctor --json
cargo run -p cli -- --help
```

Release 可执行文件位于 Cargo 目标目录的 `release/`：`subtitletool-cli.exe`、`subtitletoolkit-gui.exe` 和 `subtitletool-render-host.exe`。使用 CLI 的具体参数请运行 `subtitletool-cli.exe --help`。

### 运行测试

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib --tests --no-fail-fast
cargo test --workspace --doc --no-fail-fast -j 1
```

Doctest 使用 `-j 1` 串行运行。需要真实 native 渲染的测试依赖 libass；需要视频探测或容器处理的场景还需要相应外部工具。缺少依赖时应将跳过项与通过项分开记录。

## 打包

初始化构建缓存后，运行以下 PowerShell 脚本：

```powershell
. .\tools\set_build_cache.ps1
.\tools\make_portable.ps1
.\tools\make_installer.ps1
```

便携包与安装包生成在 Cargo 目标目录下。打包脚本会核验运行库 DLL 和 fontconfig 文件哈希，并为包内文件生成 `SHA256SUMS.txt`；便携包还包含 SBOM、第三方声明和运行库来源锁。脚本参数与安装器检查见[工程工具说明](tools/README.md)。

## 仓库布局

| 路径 | 内容 |
|---|---|
| `crates/` | Rust workspace、各格式与转换模块、CLI、GUI、渲染宿主及测试素材 |
| `docs/` | 产品规格、架构、开发计划、验收记录和当前状态 |
| `tools/` | 开发缓存、构建打包、许可证与来源核验脚本 |
| `third_party/` | 随包 libass DLL、字体配置、来源/哈希锁、逐包许可证和研究归档 |
| `Cargo.lock` | 锁定 Rust 依赖版本，保证可复现解析 |

构建输出、缓存、本机日志和开发快捷方式不纳入 Git。第三方依赖按各自许可证使用；项目自身代码按 MIT 或 Apache-2.0 任一许可证使用，详见[许可证正文](#许可证)。

## 文档

- [当前实现状态](docs/STATUS.md)
- [功能矩阵与边界](docs/specs/03_FEATURE_MATRIX.md)
- [开发、构建与发布说明](CONTRIBUTING.md)
- [完整文档导航](docs/README.md)
- [第三方声明](THIRD_PARTY_NOTICES.md)
- [libass 运行库来源、哈希与许可证](third_party/README.md)

## 许可证

SubtitleToolkit 自有代码采用双许可证：你可以选择 [MIT](LICENSE-MIT) 或 [Apache-2.0](LICENSE-APACHE)。第三方 DLL、源码归档和 Rust 依赖仍受其各自许可证约束；请同时阅读 [第三方声明](THIRD_PARTY_NOTICES.md)。
