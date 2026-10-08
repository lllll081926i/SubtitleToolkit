# 实施与验收检查表

本表列正式验收条件，不是实时进度表。空框只表示本文件没有签署验收，不表示代码一定不存在；当前实现和本次验证以 [`../STATUS.md`](../STATUS.md) 为准。历史门阶日志不作为当前证据。

适用阶段见[26](26_DEVELOPMENT_PHASES.md)，任务 ID 见[27](27_WBS_DEPENDENCIES.md)，可执行验收步骤见[25](../specs/25_ACCEPTANCE_TESTS.md)。CSRI/x86、MKV、字体子集、GUI 与 Strict 不是 CLI 基线的共同前置条件。

| 验收主题 | WBS 责任节点 | 验收 ID 与证据要求 |
|---|---|---|
| ASS 保真与字段变换 | F021, T020, T030 | AT-ASS-001/002、AT-CANVAS-001/002；字节白名单与真实视觉语料分开 |
| 时间与探测 | T010, A001 | AT-TIME-001/002、AT-PROBE-001；精确分数、量化边界与真实 ffprobe |
| libass 与宿主 | R001, R010, R011 | AT-RENDER-001；真实 DLL/像素，缺依赖记未覆盖 |
| 可选特效 provider | R012, R020, R030, R031 | AT-RENDER-002/003；实际 provider/x86 产物，不用 Poison 代替 |
| BDN/PGS | B002, P002, P003, P004, V002 | AT-BDN-001、AT-PGS-001–004、AT-BD-001；独立解码与合法 Profile |
| 多输出与缓存 | R040, J003 | AT-MULTI-001/002、AT-FONT-002；分组隔离、计数与身份失效 |
| 输出事务与批处理 | J002, J004, J005 | AT-OUT-001、AT-BATCH-001、AT-PATH-001；真实文件失败路径与集合完整性 |
| 后续界面与容器 | QA002, A002 | AT-UI-001、AT-MKV-001；未实现时不签署验收 |

每次签署记录执行人、代码提交、日期、日志、依赖与未覆盖范围。新增文档审查建议的处理状态见[采纳记录](AUDIT_ADOPTION_20261008.md)；“规范已补”不自动勾选下面的行为验收。

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
- [ ] 多输出先按 RenderGroupKey 分组，再在同组内合并采样与渲染；跨组不复用 RGBA。
- [ ] `detect_change` 仅作为 hint，事件合并基于 visible RGBA hash/diff。
- [ ] CSRI provider 与 legacy VSFilterMod adapter 分开 capability probe。
- [ ] PGS Fast Path 有 ODS payload hash invariant。
- [ ] PGS time transform 完成 DTS/decode schedule validator。
- [ ] Basic Encoder 通过大 ODS fragmentation golden test。
- [ ] Strict bitrate validator 与 stream shaping 功能名称分离。
- [ ] font subset 通过 GSUB/GPOS/复杂脚本/variable/TTC corpus，否则自动 full-font fallback。
- [ ] 所有 GPL/AGPL 组件发布边界再次审计并写 THIRD_PARTY_NOTICES。
