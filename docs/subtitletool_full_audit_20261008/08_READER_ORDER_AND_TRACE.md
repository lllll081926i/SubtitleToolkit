# 重新编排的审阅顺序与需求追踪方案

> 原V3.1物理编号**00–41完整且无重复**，无需重新命名全部文件；但“先读什么/谁覆盖谁”不同于物理字母顺序。以下为建议的**逻辑阅读顺序**，将历史变更文档保持归档性质。

## 审查时应采用的逻辑顺序（覆盖所有42份）

| 主题 | 依阅读优先级排列的V3.1文件 | 目的 |
|---|---|---|
| 索引与边界 | 00 → 01 → 02 → 03 | 先锁定位与“不做编辑器” |
| 用户操作与界面 | 04 → 05 → 35 → 36 | 页面、状态、Profile Schema一次读完 |
| 技术与架构 | 06 → 07 → 08 → 09 | Windows/Iced、IR、16:9/RenderGroup |
| 字幕算法 | 10 → 11 → 12 → 13 → 14 | 时间、HDR、渲染器、PGS与转换链 |
| 调度与辅助 | 15 → 16 → 17 → 18 → 19 → 20 | 批处理、视频辅助、字体/MKV、OCR、CLI |
| 品质与安全 | 21 → 22 → 23 → 24 → 25 → 37 → 38 | 许可、日志、安全、测试、性能、恢复 |
| 实施交付 | 26 → 27 → 28 → 39 | 阶段、WBS、发布、执行检查 |
| 外部研究 | 29 → 30 → 40 | 开源调研、在线来源、源码审计 |
| 历史与裁决 | 31 → 32 → 33 → 34 → 41 | 原审计、ADR、术语、变更日志 |

文档数量确认：4+4+4+5+6+7+4+3+5=42。若文件所述互相冲突，不允许“取较新的文件名自动获胜”；应开一条ADR，明确最新验证证据、修改受影响文件，并在总纲中列出 superseded 条款。

## 建议新增机器可追踪清单字段

推荐将每个功能单独登记：

```yaml
id: REQ-TIME-001
name: 手动源/目标FPS速度转换
owner: time-core
status: specified
milestone: M1
ui: ["FPS/时间 -> SpeedChange"]
plan_fields: ["time_mode", "source_fps", "target_fps"]
core: ["T010"]
acceptance: ["AT-TIME-SPEED-001", "AT-TIME-24000-1001-002"]
evidence: ["10_TIME_FPS_SYNC.md", "03_IMPLEMENTATION_CONTRACTS.md#c-04-timefps-与pgs时序规则"]
```

状态必须区分 `idea / specified / implemented / tested / blocked / released`。不能以“UI有”推导“核心已实现”，不能以“CI绿”推导特效滤镜已经在Windows宿主实机上验证。

## 第一批端到端映射示例

| 需求 | 输入/输出 | UI | WBS责任节点 | Gate |
|---|---|---|---|---|
| REQ-FORMAT-ASS2SRT | ASS→SRT | 转换主页 | F020,F010,T040,K001,U002 | 绘图/换行/样式损失golden |
| REQ-HDR-ASS | SDR ASS→调光ASS | ASS HDR | F020,F021,T020,C005 | RGBA字节/alpha/内联\t测试 |
| REQ-FPS-SPEED | sourcefps→targetfps | FPS/时间 | C002,T010,U002 | 帧率有理数/120分钟误差 |
| REQ-CANVAS-16-9 | 1080/2160 16:9 | ASS/PGS | C003,T030,R001 | full-frame坐标/bbox分离 |
| REQ-RENDER-FILTER | 特效DLL→RGBA | 渲染器 | H001,H002,H003,H004 | DLL位数/ABI/崩溃隔离 |
| REQ-MULTI-OUTPUT | one ASS→BDN/SUP | 多Profile | C004,R002,R003,J001,J002,J003 | Cache分组/路径冲突 |
| REQ-SUP-FAST | SUP→offset/move | SUP处理 | F040,P002,V001,V002 | ODS不变、时间有效 |
| REQ-PGS-BASIC | ASS→basic SUP | BDN/SUP | R003,I002,P003,V001 | FFmpeg decode/ODS fragments |
| REQ-PGS-STRICT | PGS严格兼容 | SUP/报告 | V001,V002,V003,P010,V004 | buffer模型+设备样本，注明未覆盖 |
| REQ-OUTPUT-SAFE | 批量输出 | 全部 | J001,J002,J004,J005 | 目录事务/断电/回滚 |
| REQ-MKV-ASSIST | MKV提轨/重封装 | 字体/MKV | A002,FNT01,MKV01 | metadata保留/原视频不改 |

## 编号本身是否错？

原目录 `00..41` 没有缺号或重号，错误主要是**后补的35/36/37/38属于核心架构，放到最后会让读者漏读**。建议保留原物理名、在`00_README.md`置顶`READER_ORDER`表，并在相应早期文档页头加上“本章节必须联合阅读：35/36/...”。

## 版本与外部证据

- 每篇页头建议：`DocStatus: Draft/Validated`、`BaselineVersion`、`Owner`、`LastVerifiedDate`、`RelatedADR`、`ImplementationTaskIds`、`AcceptanceIds`。
- 外部源码需保存commit/hash/SPDX和本地Windows验证结果，别只写GitHub master URL。
- 本轮 `05_VERIFIED_SOURCES.md` 不等同于重新下载构建所有工具；“来源核查”和“软件运行验证”单独标状态。
