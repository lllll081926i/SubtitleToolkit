# 文档导航

首次了解项目，先读[当前状态](STATUS.md)、[功能矩阵](specs/03_FEATURE_MATRIX.md)和[开发阶段](planning/26_DEVELOPMENT_PHASES.md)。准备构建时读[开发说明](../CONTRIBUTING.md)。处理新增审查建议时读[采纳记录](planning/AUDIT_ADOPTION_20261008.md)。

## 文档职责与适用范围

| 文档 | 回答的问题 | 使用边界 |
|---|---|---|
| [当前状态](STATUS.md) | 现在有哪些实现，最近验证覆盖了什么？ | 核验日期与依赖条件决定证据范围 |
| `specs/` | 功能应如何工作，如何拒绝错误输入？ | 目标规格不等于已有实现 |
| `planning/` | 接下来做什么，哪些任务互相依赖？ | 使用本仓库阶段与任务编号 |
| [架构决策](decisions/32_ADR_DECISIONS.md) | 冲突条款如何裁决，为什么这样选？ | 决策被接受不等于功能完成 |
| `development/` | 如何维护、构建与发布？ | 发行候选产物不等于已有发行物 |
| `reviews/` | 某次审查实际检查、修改和验证了什么？ | [代码审查](reviews/2026-10-08.md)、[文档优化](reviews/2026-10-08-docs.md)与[过度防御审查](reviews/2026-10-09-code-audit-defense.md)分别记录 |
| `research/`、`history/` | 外部研究与旧结论从哪里来？ | 研究结论、历史测试不作为当前通过证据 |
| [V3.1 导入基线](subtitle_tool_rust_planning_v3_1/00_README.md) | 新审查引用的原文是什么？ | 保留原始字节与行号；不在此目录继续维护规格 |
| [2026-10-08 文档审查包](subtitletool_full_audit_20261008/README_START_HERE.md) | 原规划有哪些问题和整改建议？ | 38 项问题针对导入基线；59 项 WBS 是建议，未整体替换当前计划 |

发生冲突时，先核对代码与实际测试，再查适用 ADR。修改对应规格并记录采纳结果；不要仅按文件名日期决定哪条规则生效。审查包的 `A001` 是问题编号，当前 WBS 的 `A001` 是任务编号，引用时分别写 `AUDIT-A001` 和 `WBS-A001`。

## 按主题阅读维护文档

保留 00–41 原编号。35–38 属于核心规格，应与前面的相关章节一起阅读。

| 主题 | 推荐阅读顺序 |
|---|---|
| 产品与边界 | [00 规划总纲](planning/00_README.md) → [01 产品范围](specs/01_PRODUCT_VISION_SCOPE.md) → [02 工作流](specs/02_USER_WORKFLOWS.md) → [03 功能矩阵](specs/03_FEATURE_MATRIX.md) |
| 界面与配置 | [04 信息架构](specs/04_UI_INFORMATION_ARCHITECTURE.md) → [05 页面规格](specs/05_UI_SCREEN_SPEC.md) → [35 状态与交互](specs/35_UI_STATE_INTERACTION.md) → [36 配置模型](specs/36_PRESET_CONFIG_SCHEMA.md) |
| 架构与数据 | [06 Windows 选型](specs/06_TECHNOLOGY_DECISIONS_WINDOWS.md) → [07 系统架构](specs/07_SYSTEM_ARCHITECTURE.md) → [08 格式与 IR](specs/08_FORMATS_AND_IR.md) → [09 画布与多输出](specs/09_CANVAS_RESOLUTION_MULTI_OUTPUT.md) |
| 时间、颜色与渲染 | [10 时间与 FPS](specs/10_TIME_FPS_SYNC.md) → [11 颜色映射](specs/11_HDR_COLOR.md) → [12 渲染器](specs/12_RENDERER_FILTERS_WINDOWS.md) → [13 PGS/BDN](specs/13_PGS_BDN_BLURAY.md) → [14 转换流水线](specs/14_CONVERSION_PIPELINES.md) |
| 调度与辅助能力 | [15 任务引擎](specs/15_BATCH_TASK_ENGINE.md) → [16 媒体探测](specs/16_MEDIA_PROBE_SMART_ASSIST.md) → [17 字体](specs/17_FONT_MANAGEMENT.md) → [18 MKV](specs/18_CONTAINER_HELPER.md) → [19 OCR](specs/19_OCR_OPTIONAL.md) → [20 CLI](specs/20_CLI_AUTOMATION.md) |
| 质量与恢复 | [21 许可证](specs/21_DEPENDENCIES_LICENSES.md) → [22 诊断](specs/22_LOGGING_DIAGNOSTICS.md) → [23 安全](specs/23_SECURITY_ROBUSTNESS.md) → [24 测试语料](specs/24_TEST_QA_CORPUS.md) → [25 验收](specs/25_ACCEPTANCE_TESTS.md) → [37 容量](specs/37_PERFORMANCE_CAPACITY.md) → [38 恢复](specs/38_FAILURE_RECOVERY.md) |
| 实施与发布 | [26 阶段](planning/26_DEVELOPMENT_PHASES.md) → [27 WBS](planning/27_WBS_DEPENDENCIES.md) → [28 构建与发布](development/28_WINDOWS_BUILD_RELEASE.md) → [39 检查表](planning/39_IMPLEMENTATION_CHECKLIST.md) |
| 来源与研究 | [29 开源调研](research/29_OPEN_SOURCE_RESEARCH.md) → [30 研究来源](research/30_RESEARCH_SOURCES.md) → [40 源码研究](research/40_SOURCE_CODE_AUDIT.md) → [第三方来源锁与说明](../third_party/README.md) |
| 历史、决策与术语 | [31 历史审查](history/31_AUDIT_REPORT.md) → [32 ADR](decisions/32_ADR_DECISIONS.md) → [33 术语](reference/33_GLOSSARY.md) → [34 V3 变更](history/34_V3_CHANGELOG.md) → [41 V3.1 变更](history/41_V3_1_CHANGELOG.md) |

## 维护约定

- 每篇规格写明当前实现边界；未来 API、配置示例和界面控件标为目标。
- 行为变更同步更新对应规格、WBS 与验收项。已完成状态需要代码位置和实际验证记录。
- 验收记录列输入或素材哈希、依赖版本、操作、判定依据、结果与未覆盖项。缺依赖的提前返回单列为未覆盖。
- 日期明确区分文档更新与行为验证。仅改文档时不刷新代码测试通过日期。
- [目录约定](development/REPOSITORY_LAYOUT.md)说明维护目录与导入资料的区别；[2026-10-06 状态快照](history/2026-10-06/STATUS.md)和[旧门阶](history/2026-10-06/REVIEW_GATES.md)仅供历史复查。
