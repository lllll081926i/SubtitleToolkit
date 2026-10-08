# 修订版 WBS / 依赖关系 / 阶段 Gate（建议）

新增可审查的任务共 **59 项**。ID唯一、依赖仅引用已声明的任务，已运行有向图环检查通过。里程碑M0–M5为建议，不与已有文档中Phase0–Phase9自动等同。

## 正确依赖原则

- **结构/时序PGS验证器要能独立开发和调用**：`F040 -> V001 -> V002 -> V003`，Basic Encoder与Advanced Author再消耗这些validator，不能要求先有Advanced Author才能开始Strict核心检查。
- **Renderer timeline是独立工作包**：`R001 + T010 + C003/C005 -> R002 -> R003`；多个输出只有相同RenderGroupKey才能共享RGBA。
- **多文件输出必须有事务**：`J004`独立于高级PGS，接入BDN/MKV后重新测试目录提交。
- **每个Milestone有可交付/退出标准**，无法满足则留在实验分支，不能贴Verified/Strict标签。

## 机器可解析WBS

| Task | Milestone | Depends | 交付 | 验收 |
|---|---|---|---|---|
| `C001` | M0 | - | Result/ErrorCode/Report types | typed errors+JSON snapshots |
| `C002` | M0 | C001 | Rational Time i128 + rounding | 23.976 long-run and overflow fixtures |
| `C003` | M0 | C001 | Canvas/PlayRes/LayoutRes/Storage/PAR model | 16:9 and anamorphic unit tests |
| `C004` | M0 | C001, C002, C003 | Profile/Preset schema + validation | TOML parser test / migration |
| `C005` | M0 | C001 | ColorPolicy domain model | color assumption & HDR restrictions |
| `F001` | M0 | C001 | File signature/probe | text/PG/BDN/IDX probes |
| `F002` | M0 | F001 | Source encoding with BOM & no silent loss | GB18030/Shift-JIS/UTF16 fixtures |
| `F010` | M1 | C002, F002 | SRT reader/writer | SRT Golden and invalid encoding |
| `F020` | M0 | C002, F002 | ASS token/span parser bakeoff | 100+ ASS byte exact corpus |
| `F021` | M1 | F020 | ByteRangePatch ASS serializer | byte-identical unchanged roundtrip |
| `T010` | M1 | C002, F020 | FPS/offset/two-point mappings | time error below chosen tick bound |
| `T020` | M1 | C005, F020, F021 | ASS SDR/HDR color transforms | unit/color/alpha/override corpus |
| `T030` | M1 | C003, F020, F021 | ASS preserve/rewrite resolution | PlayRes/LayoutRes differential |
| `T040` | M1 | F020, F010 | ASS→SRT text projection | drawing/linebreak/overlap semantics |
| `T050` | M2 | F020, F021 | ASS multi-file merge | StyleMap and inline r override |
| `U001` | M0 | C001 | Iced 0.14 Windows PoC | drag-drop+native dialog+build reproducible |
| `U002` | M1 | U001, F001, C004 | Manual-first tool shell & params | input validation/value origins |
| `U003` | M2 | U002, J002 | Queue drawer + progress/error cards | headless E2E cancel partial success |
| `U004` | M2 | U002, C004, J001 | Multi-output profile table | collision/errors/disabled combos |
| `A000` | M0 | C001 | Absolute-path process adapter & cancellation | timeout/exit/status/stdout bounds |
| `A001` | M2 | A000 | ffprobe Windows adapter | FPS/HDR metadata info only |
| `A002` | M4 | A000 | mkvmerge/mkvextract adapter | track/attachment metadata preserved |
| `A003` | M4 | F020 | Windows DirectWrite font resolver | localized/TTC/face matches |
| `H001` | M0 | C001 | versioned named-pipe host protocol | fuzz + bounded IPC |
| `H002` | M0 | H001 | PE bitness & export/capability probe | actual x86/x64 provider smoke |
| `H003` | M3 | H001, H002 | Isolated renderer-host x64/x86 | host crash no GUI crash |
| `H004` | M3 | H003 | CSRI production adapter | diff corpus and capability fail-closed |
| `H005` | M4 | H003, A000 | Legacy VSScript/AviSynth adapters | official ABI, clean diagnostics |
| `R001` | M0 | F020, C003 | libass native memory & fonts wrapper | native build and memory lifetime tests |
| `R002` | M2 | R001, T010, C003, C005 | RenderGroupKey & Timeline planner | no cross-profile RGBA reuse |
| `R003` | M2 | R001, R002 | Frame composite + bbox/hash | animated frame/detect_change corpus |
| `R004` | M3 | R003, H004 | Renderer differential harness | libass vs special provider metrics |
| `I001` | M2 | R003 | RGBA/frame/PNG encoder | lossless RGBA/alpha golden |
| `I002` | M3 | I001 | quantizer policy + palette vectors | palette coverage/no transparent block |
| `B001` | M2 | C002, C003, I001 | BDN XML+PNG writer | frame TC, actual PNG size correct |
| `J001` | M1 | C004 | Immutable JobSnapshot & Plan schema | CLI/Preset snapshot equivalence |
| `J002` | M2 | J001, C001 | Bounded task scheduler & cancellation | 100-job queue no UI blocking |
| `J003` | M2 | J002, R002 | Cache with dependency hashes & memory budgets | cache invalidation/capacity tests |
| `J004` | M2 | J002 | Verified output transaction & collision detection | staged file/folder output atomicity tests |
| `J005` | M4 | J003, J004 | Resume / partial success recovery | power loss/retry/cancel integration |
| `K001` | M1 | J001, F010, F021, T010, T020 | CLI core verbs/repro JSON contract | golden command outputs and exit codes |
| `F030` | M3 | C002, C003 | BDN parser/validator IR | XML XXE off and coords/PNG checks |
| `F040` | M0 | C002 | PGS segment parser/state model | third-party SUP and fuzz |
| `P001` | M3 | F040 | PGS RLE/stateful decoder | independent decode/diff |
| `P002` | M3 | F040, C002 | PGS no-ODS-reencode transforms | ODS invariant+PTS/DTS checks |
| `V001` | M3 | F040 | PGS structural validator | PDS/ODS/PCS/WDS golden vectors |
| `V002` | M3 | V001, C002 | PGS time/lifecycle validator | PTS/DTS/erase/palette case fixtures |
| `P003` | M3 | I002, F040, V001 | PGS basic encoder | fragmentation+player smoke |
| `V003` | M5 | V002 | Decoder/buffer/bitrate model validator | golden inputs and model source trace |
| `P010` | M5 | P003, V003, R003 | Advanced PGS author policies | epoch/acquisition/ahead/buffer regression |
| `V004` | M5 | V003, P010 | Device-corpus validation report | device models/firmware outcomes logged |
| `FNT01` | M4 | A003, F020, R001 | Font coverage/subset/embedding | font license + shaping equivalence |
| `MKV01` | M4 | A002, FNT01, J004 | New MKV subtitle/attachment assistant | source MKV unchanged incl chapters/tracks |
| `SEC01` | M0 | H001, A000 | Renderer native/DLL threat model | job object != sandbox; permission policy |
| `QA001` | M0 | C001 | Golden fixtures+licensing manifest | each fixture has oracle and usage rights |
| `QA002` | M2 | QA001, U002, J002 | GUI E2E Windows DPI & IME | 1366x768/200%DPI keyboard paths |
| `QA003` | M3 | QA001, R003, P001 | PGS/Renderer differential suite | byte and pixel reference reports |
| `REL01` | M4 | K001, QA002, J004 | Portable & Inno Setup smoke | fresh Win11 VM verify + notices |
| `REL02` | M5 | REL01, V004, QA003 | Advanced Blu-ray release gate | strict caveats, signing and SBOM |

## 发布里程碑与建议切线

### M0 技术可行性（不面向用户）
- Rust/Iced Windows x64 PoC、ASS parser字节回写、libass渲染、PGS parser样例、外部滤镜Host协议PoC、来源许可台账。
- 任一关键原生ABI不能在真实WinVM复现，则不推进它作为正式依赖。

### M1 文本工具内测版
- GUI/CLI支持ASS/SRT转换、FPS、offset、只在明确假设下的HDR RGB dim、手动多profile schema。
- M1不能宣传PGS或Blu-ray strict，因为PGS writer未交付。

### M2 渲染/BDN/批处理 Beta
- renderer分组缓存、BDN图片输出、批量队列、取消、事务与多输出（受限于已实现格式），ffprobe仅辅助。
- 必须完成多组缓存错误复用回归测试，以及BDN目录发布完整性测试。

### M3 PGS与外部特效稳定版候选
- PGS parser/decoder、no-ODS-reencode Fast Path、Basic SUP、CSRI-host兼容，PGS结构/时间验证器。
- UI只允许称“Basic PGS output / 已通过结构与时间校验”，不能称严格过水。

### M4 字体/MKV/安装包
- 字体许可与subset安全、MKV轨道助手、可恢复批处理、Win11/Win10兼容策略、InnoSetup和portable烟测。
- 实际公开发布前再校验第三方分发许可证、Windows证书签名可用性。

### M5 严格PGS与扩展版
- Buffer model validator、Advanced Author、硬件/软件播放器报告、明确设备覆盖。
- 若缺权威规则/独立对照，就只保留“实验性兼容化/模型未验证”，不得虚称严格认证。

## CI自动Gate例子

- `cargo fmt --check`, `cargo clippy -D warnings`, `cargo nextest run`, parser/PGS/IPC fuzz短测，Windows native依赖hash校验。
- 输出 `capabilities.json` 以及 `requirements-trace.json`，每个UI action必须有对应task、transform、test。
- 4K动态样本跑受控RAM与文件输出容量压力测试，不以“GUI仍能点”为性能唯一判据。