# Windows 构建、安装、签名与发布

## 1. Toolchain
- Rust stable MSVC x64；
- Visual Studio Build Tools + Windows SDK；
- native dependency build 用 CMake/Meson 时锁版本；
- `cargo-deny`, `cargo-audit`, `cargo-nextest`。

## 2. Artifacts
- `SubtitleToolkit.exe`
- `subtitletool-cli.exe`
- `renderer-host-x64.exe`
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
所有程序文件放同目录；settings 默认仍进 `%APPDATA%`，若发现 `portable.flag` 则使用 `./data`。

## 5. Signing
正式 release：所有 EXE/DLL/installer 用同一 publisher identity Authenticode；timestamp server；CI 验证签名后再上传。

## 6. Updates
第一版可只“检查更新并打开 release 页面”；成熟后做 signed manifest + staged download。不要在早期自写高权限 auto-updater。

## 7. CI
Windows x64 runner：fmt/clippy/test/nextest/fuzz smoke/build/E2E。x86 helper 单独 job。Nightly corpus test 可较重。
