# 测试、QA 与 Corpus

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
所有 P0/P1 acceptance + no critical fuzz crash + installer/portable smoke + signed artifact verification（正式渠道）。


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
