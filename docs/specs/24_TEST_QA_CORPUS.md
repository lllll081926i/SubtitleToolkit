# 测试、QA 与 Corpus

本页定义测试目标，具体执行步骤见[25](25_ACCEPTANCE_TESTS.md)，实际结果见[STATUS](../STATUS.md)。未配置的 fuzz、GUI E2E、设备或 benchmark 不因为列在本页而视为已运行。

## 1. 测试层级
Unit → Property → Fuzz → Golden → Differential → Integration → GUI E2E → Real-device/manual gate。

## 2. ASS corpus
- simple；
- full styles；
- unknown tags；
- `\t/\move/\clip/\iclip/\org/\p/karaoke/fade`；
- VSFilterMod private effects；
- vertical fonts；
- embedded fonts；
- out-of-order events；
- Unicode SMP/CJK。

## 3. 16:9 golden
每个特效样本都有 720/1080/2160 golden coordinate assertions；验证 bbox 裁剪不会改变全画布位置。

## 4. Renderer differential
固定字体目录和 DPI；libass、CSRI provider 在关键 timestamps 输出 PNG/diff metrics。

## 5. PGS corpus
第三方真实 SUP：Blu-ray、UHD-BD、不同 palette/object reuse、forced、overlap、损坏文件。

## 6. Fast Path assertion
只改时间/位置时，ODS payload hash 必须保持完全一致。

## 7. Fuzz
ASS parser、SRT parser、BDN XML、PGS segment/RLE、filename template、IPC decoder。

## 8. GUI E2E
Iced 0.14 headless/E2E：页面导航、drop event、参数联动、queue cancel、错误卡、multi-output profile。

## 9. Windows matrix
至少 Win10 22H2、Win11 当前稳定；100/125/150/200% DPI；中文路径、emoji、长路径；无管理员权限。

## 10. 发布 Gate
针对该发行物声明的功能运行适用 P0/P1 acceptance；拒绝未实现的能力。GUI、特效 provider、设备 Strict 等只在对应发行范围中设置 Gate。正式渠道还需适用的 fuzz、installer/portable smoke 与签名验证；CLI 基线不以所有研究线完成作为前置条件。


## 源码审计后新增 Corpus / Golden Tests

### ASS/libass
- PlayRes-only；
- LayoutRes+PlayRes；
- anamorphic storage/PAR；
- 1080→2160 Preserve Script；
- 1080→2160 Rewrite Script；
- `\t/\move/\fad/\fade/\k/\clip/drawing` 动态样本；
- Unicode/超长/emoji/非 BMP；
- Windows 非 ASCII/特殊路径（主程序读内存）。

### Unified Render Timeline
- 同组采样并集与跨组 RGBA 隔离分别覆盖；
- BDN frame samples 与 SUP exact event boundaries 同时输出；
- 一个 event start/end 落在帧间；
- libass detect_change=0/1/2 与实际 RGBA 对比；
- identical visible frame 去重；
- 多 profile 共用 render count 断言。

### PGS
- ODS > 64 KiB fragmentation；
- palette-only updates；
- object redefinition/version；
- overlapping objects；
- empty/erase display set；
- Fast Path ODS SHA-256 invariant；
- time resync 后 PTS/DTS validator；
- white/gray Y-only brightness 与彩色 color-managed transform 分开；
- full 1920×1080 composition + tight bbox object placement。

### Fonts
- ligature、Arabic、Indic、combining、vertical、variable、TTC/OTC、CFF/CFF2；
- subset 前后 libass/VSFilter renderer visual equivalence；
- ambiguous localized family + weight/stretch selection。

## 语料使用记录

外部字幕、字体、真盘 SUP 与截图分别登记来源、SHA-256、版本、许可/使用范围、可否公开提交，以及独立判定依据与容差。不能将仅获准本机测试的字体或字幕复制为公共 fixture。生成素材也需记录生成参数；本项目往返仅证明自洽，不替代独立解码或真实 provider 对照。
