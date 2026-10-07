# Windows-only 技术选型

## 1. GUI：Iced 0.14
最终选择 Iced，而不是 V2 的 Slint。

原因：
- 纯 Rust；
- 0.14 已有 hot reload、time-travel debugging、headless/E2E testing、table/grid；
- winit/iced window event 能自然承接 Windows 文件拖放；
- 状态机和 Task 模型适合任务队列型工具；
- 不需要额外 UI DSL 与归属许可处理。

来源：https://github.com/iced-rs/iced/blob/master/CHANGELOG.md

## 2. 平台
- `x86_64-pc-windows-msvc` 唯一主 target。
- Windows 10 22H2 / Windows 11 作为测试基线。
- 不做 Linux/macOS 条件编译；减少 path/font/plugin/installer 分支。
- x86 只用于 `renderer-host-x86.exe` 兼容 32-bit CSRI/VSFilter DLL。

## 3. 文件对话框
使用 Windows 原生 `IFileDialog`，可通过 rfd 封装；不自绘文件选择器。

## 4. 字体
优先 Windows DirectWrite/系统字体集合 + 自建缓存；ASS font matching 逻辑与 subset 可以参考 Rust assfonts。

## 5. 媒体探测
首版使用 `ffprobe.exe` 外部 adapter：版本固定、JSON 解析、超时、hash。以后如有必要再切 libav* FFI，避免早期引入巨量 native ABI。

## 6. MKV
不重写 Matroska muxer；首版 adapter 调用 `mkvmerge.exe`/`mkvextract.exe`，解析 JSON identification。

## 7. 安装器
V3 推荐：**Inno Setup 7 + Portable ZIP 双轨**。
- Inno Setup 7.1.0 已于 2026-08 发布；
- 对传统 Win32/Rust 桌面工具、文件关联、可选组件、升级最简单；
- 商业分发时需重新核对其商业许可条款。
来源：https://jrsoftware.org/isdl.php

MSIX 作为后续可选渠道，不作为开发期默认，因为签名/包身份和外部 helper/plugin 路径会增加调试成本。

## 8. 代码签名
正式公开发行时签主 EXE、helper、installer。Windows SmartScreen 会综合 publisher/file reputation；签名不能保证新版本第一次运行绝不提示，但可以累积 publisher reputation。
来源：https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation
