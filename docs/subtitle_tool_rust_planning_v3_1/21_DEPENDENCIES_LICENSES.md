# 依赖与许可证审计

## 1. Core 允许原则
默认安装包核心优先 MIT/Apache/BSD/ISC/zlib 等宽松许可；GPL 组件采用外部程序/插件边界或在决定项目整体许可证后再集成。

## 2. GUI
Iced：锁版本并在发布前再次核对 license/dependency tree。

## 3. ASS
- libass：ISC，可作为 native core：https://github.com/libass/libass
- ass-rs / oxideav-ass：作为 parser bake-off 候选；进入 hard dep 前要跑 corpus。

## 4. PGS
OxideAV 的 Rust subtitle/image 项目可作为 MIT 参考/候选；但 Advanced PGS author 必须靠我们自己的验证，不能因为 crate 能 encode 就等价于 Blu-ray strict。

## 5. Image quantization
**libimagequant 4.x 不作为默认核心依赖。** 当前许可证是 GPL-3.0-or-later / commercial 双许可，除非项目整体许可/商业许可明确解决，否则只做可选 adapter 或完全不用。默认选择宽松许可 Rust quantizer 或自研可测试实现。

## 6. VSFilterMod
GPL-3.0；默认设计为用户提供/可选外部 renderer provider，通过 renderer-host/CSRI 调用。是否随安装包分发要在最终项目许可证确定后由法务/许可证审计决定。
来源：https://github.com/sorayuki/VSFilterMod

## 7. GPL/AGPL 研究对象边界
- SUPer：GPL-3.0，行为/测试参考；默认不嵌入非 GPL core。
- ass2bdnxml（Masaiki/cubicibo 系）：GPL-3.0-or-later，行为/测试参考。
- ALASS：GPL-3.0；Smart Sync 默认外部 `alass.exe` adapter，除非未来整体许可策略改变。
- SupMover：AGPL-3.0；只做行为参考和测试向量，不复制代码到 core，也不将其静态/动态链接进默认安装包。
- VSFilterMod：GPL-3.0；用户提供/可选 provider 的进程边界。

注意：把 GPL/AGPL 程序作为独立外部 executable 的具体分发方式仍需在发布前做最终许可证审计；“进程边界”不是自动免除许可义务的法律结论。

## 8. ffprobe/FFmpeg
FFmpeg 官方不直接提供 Windows EXE，只链接第三方 builds；如果随包分发，需要锁定 build 来源、配置与 LGPL/GPL feature set，并提供 license/source obligations。
来源：https://ffmpeg.org/

## 9. MKVToolNix
作为外部工具 adapter；随包分发与否单独审计许可证和更新策略。

## 10. 中文转换
优先 pure Rust `zhhz`（OpenCC dictionaries/reimplementation，需核对 Apache-2.0 数据与代码声明），避免 Windows 支持不确定的 FFI binding。

## 11. 依赖清单要求
`THIRD_PARTY_NOTICES.md` 自动生成；每个 native/exe dependency 记录 version、URL、license、hash、bundled/external、update policy。


## 12. 本次源码审计新增结论
- `libass`: ISC，可直接作为 renderer native dependency。
- `kotorimiku/assfonts`: MIT，但项目年轻；允许参考/候选复用，subset 正确性必须自行验证。
- `OxideAV` subtitle crates: MIT 候选；锁 commit/version + corpus/fuzz gate。
- 第三方源码的算法思想可参考，但不能把许可证不兼容源码“翻译成 Rust”后视为独立实现；实现记录必须保留 clean-room/来源边界。
