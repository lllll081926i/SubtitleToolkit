# 2026-10-08 文档审查建议采纳记录

本页连接新增审查包与当前维护文档。审查对象是[导入 V3.1 基线](../subtitle_tool_rust_planning_v3_1/00_README.md)，不是当前程序；问题原文见[38 项台账](../subtitletool_full_audit_20261008/01_ISSUES.md)，实施建议见[核心契约](../subtitletool_full_audit_20261008/03_IMPLEMENTATION_CONTRACTS.md)。

资料保底提交为 `b1ca4b5`；本次优化从已有代码审查提交 `8db2292` 继续。保留两个导入目录的原始字节与证据行号。当前能力只维护在[STATUS](../STATUS.md)和[功能矩阵](../specs/03_FEATURE_MATRIX.md)，不在本页复制完成率。

## 采用方式

- **规范已补**：维护文档已消除对应歧义；不表示功能实现或真实验收关闭。
- **纳入计划**：保留建议，按当前 WBS/Phase 实施；本次未补齐全部细节或代码。
- **保留待裁决**：依赖真实样本、规则、分发方案或用户环境；不得填成已通过。
- **按现有实现调整**：采用建议的目的，接口或顺序与建议包不同；理由在 ADR 或维护规格中明确。

不整体套用建议包的 59 项 WBS、M0–M5、i128 持久时间类型和 NamedPipe/FileMapping。现有 CLI、i64 有理数模型及有界匿名管道有各自的代码与测试，变更须有具体需求与验收。详见 [ADR-013–016](../decisions/32_ADR_DECISIONS.md#adr-013-按完整渲染条件分组)。

## 全部问题的处理去向

| 原问题 | 本次处理 | 维护位置与仍需完成的工作 |
|---|---|---|
| AUDIT-A001 多规格渲染共享 | 规范已补 | [09](../specs/09_CANVAS_RESOLUTION_MULTI_OUTPUT.md)：完整分组字段与共享矩阵；WBS-R040/J003、实际调用计数仍待验收 |
| AUDIT-A002 HDR 算法 | 按现有实现调整 | [11](../specs/11_HDR_COLOR.md)：当前四模式、公式、量化与 203 基准；真实 HDR 播放链尚未校准 |
| AUDIT-A003 PGS 字节/时序规格 | 纳入计划 | [13](../specs/13_PGS_BDN_BLURAY.md)、WBS-P004/V002/P011；独立解码与缓冲语料仍待补 |
| AUDIT-A004 WBS 缺节点/依赖方向 | 规范已补 | [27](27_WBS_DEPENDENCIES.md)：补过程、编码、颜色、采样与 QA 节点；验证器先于 Advanced Author |
| AUDIT-A005 16:9 例外冲突 | 规范已补 | [09](../specs/09_CANVAS_RESOLUTION_MULTI_OUTPUT.md)：删除正式特效输出的非 16:9 例外；保留原始 PlayRes/LayoutRes |
| AUDIT-A006 ASS 字节保真 | 规范已补 | [08](../specs/08_FORMATS_AND_IR.md)、[25](../specs/25_ACCEPTANCE_TESTS.md)：无变换逐字节与目标字段差异分开；未覆盖编码不算保真 |
| AUDIT-A007 provider 矩阵/回退 | 纳入计划 | [12](../specs/12_RENDERER_FILTERS_WINDOWS.md)：既有 libass 宿主；VSFilterMod/x86/legacy 各自真实验收 |
| AUDIT-A008 Strict 夸大 | 规范已补 | [13](../specs/13_PGS_BDN_BLURAY.md)：结构、时间、模型、列明设备独立报告，不宣称全设备认证 |
| AUDIT-A009 进程不等于沙箱 | 规范已补 | [23](../specs/23_SECURITY_ROBUSTNESS.md)、ADR-016；受限令牌/权限隔离仍未实现 |
| AUDIT-A010 验收缺判定依据 | 规范已补 | [25](../specs/25_ACCEPTANCE_TESTS.md)：步骤、误差、定位与证据模板；外部素材不足的用例仍未覆盖 |
| AUDIT-A011 文档顺序 | 规范已补 | [导航](../README.md)：全部 00–41 按主题读取，原始/维护/历史文档职责明确 |
| AUDIT-A012 程序命名 | 按现有实现调整 | [07](../specs/07_SYSTEM_ARCHITECTURE.md)、[20](../specs/20_CLI_AUTOMATION.md)：当前两个 Cargo bin；发行候选不冒充产物 |
| AUDIT-A013 Windows 支持策略 | 规范已补 | [06](../specs/06_TECHNOLOGY_DECISIONS_WINDOWS.md)：Windows 11 主验收、Windows 10 兼容目标；发行矩阵仍需真实 OS 测试 |
| AUDIT-A014 Fast Path 无损措辞 | 规范已补 | [13](../specs/13_PGS_BDN_BLURAY.md)：无位图重编码与视觉/时间变化分开 |
| AUDIT-A015 preset/多输出冲突 | 规范已补 | [15](../specs/15_BATCH_TASK_ENGINE.md)、[36](../specs/36_PRESET_CONFIG_SCHEMA.md)：JSON 现状与 TOML 目标分开，继承/快照/碰撞顺序明确 |
| AUDIT-A016 多文件/跨卷事务 | 纳入计划 | [38](../specs/38_FAILURE_RECOVERY.md)：BDN 集合与同卷 staging 目标；锁文件、断电等完整矩阵仍待行为测试 |
| AUDIT-A017 ASS→SRT 损失 | 纳入计划 | [14](../specs/14_CONVERSION_PIPELINES.md)、WBS-T040；继续补绘图、重叠、换行策略与素材 |
| AUDIT-A018 编码/原始字节 | 规范已补 | [08](../specs/08_FORMATS_AND_IR.md)、WBS-F002；不确定编码与重编码必须报告 |
| AUDIT-A019 字体许可 | 保留待裁决 | [17](../specs/17_FONT_MANAGEMENT.md)、[21](../specs/21_DEPENDENCIES_LICENSES.md)；存在/覆盖/嵌入/公开分发分别核实 |
| AUDIT-A020 PGS Profile 矩阵 | 保留待裁决 | [13](../specs/13_PGS_BDN_BLURAY.md)、WBS-V002；具体 FPS/画布/设备组合需来源与样本 |
| AUDIT-A021 SUP 色彩推定 | 规范已补 | [11](../specs/11_HDR_COLOR.md)、[13](../specs/13_PGS_BDN_BLURAY.md)：不得从后缀/尺寸推定 HDR，彩色与 Y-only 分开 |
| AUDIT-A022 HFR 采样 | 纳入计划 | [09](../specs/09_CANVAS_RESOLUTION_MULTI_OUTPUT.md)、[12](../specs/12_RENDERER_FILTERS_WINDOWS.md)；自适应降采样阈值与视觉样本仍待冻结 |
| AUDIT-A023 MKV 命令合同 | 纳入计划 | [18](../specs/18_CONTAINER_HELPER.md)、WBS-A002；未实现，轨道/附件/章节保留需真实工具验证 |
| AUDIT-A024 容量阈值 | 纳入计划 | [37](../specs/37_PERFORMANCE_CAPACITY.md)：明确测量字段与缓存引用边界；不编造配额和性能数据 |
| AUDIT-A025 进度/取消 | 纳入计划 | [15](../specs/15_BATCH_TASK_ENGINE.md)、[35](../specs/35_UI_STATE_INTERACTION.md)、[38](../specs/38_FAILURE_RECOVERY.md)；宿主 Cancel 协议仍未实现 |
| AUDIT-A026 可复现外部依赖 | 按现有实现调整 | [第三方来源锁](../../third_party/SOURCE_AUDIT_LOCK.json)：沿用现有锁，逐个追加真实二进制来源与核验；不把所有候选写为已固定 |
| AUDIT-A027 GPL/AGPL 分发 | 保留待裁决 | [21](../specs/21_DEPENDENCIES_LICENSES.md)；外部进程与源码锁不自动完成分发义务 |
| AUDIT-A028 首发范围/阶段 | 按现有实现调整 | [26](26_DEVELOPMENT_PHASES.md)：按现有 CLI 继续，GUI/可选适配器独立，不从 M0 重做 |
| AUDIT-A029 控件行为合同 | 纳入计划 | [35](../specs/35_UI_STATE_INTERACTION.md)与[导入界面审查](../subtitletool_full_audit_20261008/07_UI_AND_INTERACTION_AUDIT.md)；逐控件合同在 GUI 阶段细化 |
| AUDIT-A030 错误码/脱敏 | 纳入计划 | [20](../specs/20_CLI_AUTOMATION.md)、[22](../specs/22_LOGGING_DIAGNOSTICS.md)；已有 CLI 信封，统一 registry/诊断包需继续完善 |
| AUDIT-A031 源码审查复现 | 按现有实现调整 | [来源锁](../../third_party/SOURCE_AUDIT_LOCK.json)、[40](../research/40_SOURCE_CODE_AUDIT.md)；历史源码研究不替代当前二进制测试 |
| AUDIT-A032 规范化范围 | 纳入计划 | [14](../specs/14_CONVERSION_PIPELINES.md)、WBS-T040；明确每个有损选项，避免自动改写作者内容 |
| AUDIT-A033 自用/公开分发 | 保留待裁决 | [21](../specs/21_DEPENDENCIES_LICENSES.md)、[28](../development/28_WINDOWS_BUILD_RELEASE.md)；公开发布许可与正文仍待闭环 |
| AUDIT-A034 语料授权 | 纳入计划 | [24](../specs/24_TEST_QA_CORPUS.md)、WBS-QA001；外部字幕、字体、真盘样本登记来源与使用范围 |
| AUDIT-A035 视觉/无障碍 | 纳入计划 | [35](../specs/35_UI_STATE_INTERACTION.md)、WBS-QA002；键盘、IME、DPI 为目标，品牌 token 尚未冻结 |
| AUDIT-A036 需求追踪 | 规范已补 | 本页、[27](27_WBS_DEPENDENCIES.md)、[25](../specs/25_ACCEPTANCE_TESTS.md)、[39](39_IMPLEMENTATION_CHECKLIST.md)互相链接；完整机器追踪后续实施 |
| AUDIT-A037 VFR/FPS 概念 | 规范已补 | [10](../specs/10_TIME_FPS_SYNC.md)：精确分数、十进制差异与真实帧时间图边界 |
| AUDIT-A038 合并样式引用 | 纳入计划 | [14](../specs/14_CONVERSION_PIPELINES.md)、WBS-T050；内联 `\rStyle`、附件冲突与完整引用映射仍需实现验收 |

## 当前实施优先次序

先验证现有真实 libass/host 与 UTF-8 内存输入，再完成跨 profile 分组和输出目录事务的行为测试。PGS 独立对照与 GUI 合同按各自阶段推进。未裁决许可或设备规则只阻塞依赖该结论的发布项，不阻塞无关 CLI 修复。

本次文档检查与追加原生验证分别记录在[文档优化与依赖接入记录](../reviews/2026-10-08-docs.md)。它不改写导入审查包的“未运行程序”声明。
