# 实施检查表（开发者逐项勾选）

## 开工前
- [ ] Rust/VS Build Tools/Windows SDK 固定版本
- [ ] CI Windows runner 可重现
- [ ] third-party license table 建立
- [ ] test corpus 仓库/大文件存储策略建立

## ASS MVP
- [ ] Format 动态列
- [ ] Script Info unknown keys
- [ ] attachments
- [ ] override tokenizer
- [ ] byte/semantic roundtrip corpus
- [ ] 16:9 resolution transform golden
- [ ] HDR transform golden
- [ ] Rational FPS transform

## GUI MVP
- [ ] Iced FileDropped 多文件
- [ ] native open/save dialog
- [ ] Manual-first pages
- [ ] immutable JobSnapshot
- [ ] queue cancel
- [ ] no UI blocking E2E

## Renderer
- [ ] libass version probe
- [ ] font environment
- [ ] dynamic event sampling
- [ ] CSRI x64 proof
- [ ] CSRI x86 proof
- [ ] provider crash isolation
- [ ] pixel diff report

## BDN/PGS
- [ ] PNG alpha
- [ ] bbox preserves full-canvas position
- [ ] BDN TC correct
- [ ] third-party PGS parse
- [ ] Fast Path ODS invariant
- [ ] Basic encoder roundtrip
- [ ] 2160p strict profile blocked

## 多输出
- [ ] profile duplicate/edit
- [ ] shared render cache
- [ ] partial success
- [ ] deterministic filename template

## 发布
- [ ] portable smoke
- [ ] installer smoke
- [ ] uninstall smoke
- [ ] Chinese/emoji/long paths
- [ ] Authenticode signing pipeline
- [ ] SBOM/THIRD_PARTY_NOTICES
- [ ] diagnostic zip redaction


## V3.1 源码审计新增 Gate
- [ ] libass renderer 初始化同时设置 storage_size + frame_size + fonts。
- [ ] ASS 文件通过 Rust 读 bytes 后 `ass_read_memory`，不依赖窄字符串文件路径。
- [ ] OutputCanvas / PlayRes / LayoutRes / StorageSize 数据模型分开。
- [ ] 多输出先生成 Render Timeline Planner，再统一渲染。
- [ ] `detect_change` 仅作为 hint，事件合并基于 visible RGBA hash/diff。
- [ ] CSRI provider 与 legacy VSFilterMod adapter 分开 capability probe。
- [ ] PGS Fast Path 有 ODS payload hash invariant。
- [ ] PGS time transform 完成 DTS/decode schedule validator。
- [ ] Basic Encoder 通过大 ODS fragmentation golden test。
- [ ] Strict bitrate validator 与 stream shaping 功能名称分离。
- [ ] font subset 通过 GSUB/GPOS/复杂脚本/variable/TTC corpus，否则自动 full-font fallback。
- [ ] 所有 GPL/AGPL 组件发布边界再次审计并写 THIRD_PARTY_NOTICES。
