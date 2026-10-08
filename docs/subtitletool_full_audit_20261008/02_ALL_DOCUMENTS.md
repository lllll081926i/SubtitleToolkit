# 42份文档逐一审查结果

文件完整性：检查了每份 Markdown 存在、前导编号、标题、主要要求、上下游文档关联、对应缺陷。以下状态均是 **规范待补**，并非“源码已坏”。

| 文件 | 原文行数 | 本轮补强要求 | 对应问题 |
|---|---:|---|---|
| `00_README.md` | 90 | 补充 V3.1 42份的正式索引、跨章节覆盖优先级、阅读顺序与审计日期；不要把旧V3文字称最新版。 | A011,A026,A036 |
| `01_PRODUCT_VISION_SCOPE.md` | 83 | 明确首发里程碑范围、Windows11主支持/Win10兼容、Strict认证措辞和产品非目标。 | A008,A013,A028,A033 |
| `02_USER_WORKFLOWS.md` | 77 | 对每一用户流补输入格式/手动参数/失败分支/产物与损失说明、目标视频辅助规则。 | A015,A020,A032 |
| `03_FEATURE_MATRIX.md` | 43 | 将功能行增加稳定REQ ID、release里程碑、功能完成定义、依赖主键、测试ID。 | A028,A036 |
| `04_UI_INFORMATION_ARCHITECTURE.md` | 63 | 补响应式主窗口布局、1366×768缩放、键盘/焦点/屏读无障碍与拖放状态。 | A029,A035 |
| `05_UI_SCREEN_SPEC.md` | 110 | 每个控件补默认值、启用/禁用条件、验证、事件、任务行为与错误信息。 | A002,A029,A035 |
| `06_TECHNOLOGY_DECISIONS_WINDOWS.md` | 44 | 锁 Iced 0.14 release tag 而非master；Win10生命周期更新；正式描述依赖二进制来源与InnoSetup许可。 | A013,A026,A027 |
| `07_SYSTEM_ARCHITECTURE.md` | 83 | 将 multi-output DAG按 RenderGroupKey 分组；加入任务接口、job plan hash、process sandbox与同卷输出事务。 | A001,A009,A015,A016 |
| `08_FORMATS_AND_IR.md` | 51 | 定义ASS ByteRangePatch/编码策略、PGS segment typed IR、未知字段透传和精确时间类型。 | A006,A017,A018 |
| `09_CANVAS_RESOLUTION_MULTI_OUTPUT.md` | 97 | 删特效输出的非16:9例外；区分统一时间轴与每配置渲染组；多输出颜色/renderer共享范围。 | A001,A005,A022 |
| `10_TIME_FPS_SYNC.md` | 37 | 公式边界条件、i128中间值、时间原点、PTS/DTS互斥/相关处理、VFR及误差上限。 | A003,A022,A037 |
| `11_HDR_COLOR.md` | 37 | 补两套亮度算法正式公式、色域假设、203nit限制、HDR/SDR SUP source policy、色彩测试向量。 | A002,A021 |
| `12_RENDERER_FILTERS_WINDOWS.md` | 112 | 建立provider二进制兼容清单、ABI probe具体步骤、权限边界与渲染命令字节协议。 | A007,A009,A026 |
| `13_PGS_BDN_BLURAY.md` | 120 | 补PGS PDS/ODS/PCS字段表、时间/颜色/FPS合法组合、decoder buffer模型、合规证据边界。 | A003,A008,A014,A020 |
| `14_CONVERSION_PIPELINES.md` | 31 | ASS→SRT绘图/转义/多事件策略，BDN PNG格式独立策略，merge内联样式引用策略。 | A017,A018,A038 |
| `15_BATCH_TASK_ENGINE.md` | 36 | 将任务/缓存/取消/重试/崩溃恢复/名称冲突拆成可执行算法及状态机。 | A015,A024,A025 |
| `16_MEDIA_PROBE_SMART_ASSIST.md` | 27 | 视频探测内容附优先级、HDR metadata不可证明ASS来源、探测失败回退为纯手动。 | A021,A037 |
| `17_FONT_MANAGEMENT.md` | 83 | 增加字体license/fsType/可再分发检查，subset失败等价回退与完整font环境hash。 | A019,A034 |
| `18_CONTAINER_HELPER.md` | 20 | 补真实mkvmerge/mkvextract参数合同、轨道/附件ID、默认/强制/章节保留规则。 | A023,A027 |
| `19_OCR_OPTIONAL.md` | 15 | OCR首发禁用/后期Gate具体说明；声明文字人工校对问题与不做编辑器替代。 | A028,A034 |
| `20_CLI_AUTOMATION.md` | 24 | 补实际CLI flags/exit codes/stdout/stderr合同、JSONL schema和重放示例。 | A012,A030 |
| `21_DEPENDENCIES_LICENSES.md` | 50 | 逐项锁定tag+hash+SPDX、external/bundled策略、字体许可及FFmpeg/MKV GPL依赖义务。 | A019,A026,A027,A033 |
| `22_LOGGING_DIAGNOSTICS.md` | 19 | 建错误码registry、用户可执行处理建议、diagnostics隐私清单/敏感信息脱敏。 | A025,A030 |
| `23_SECURITY_ROBUSTNESS.md` | 24 | renderer-host进程隔离不能视为sandbox；拓展恶意DLL/字体/MKV攻击面安全策略。 | A009,A019 |
| `24_TEST_QA_CORPUS.md` | 75 | 定义golden fixture来源与许可、每组测试量化oracle/容差、自动化覆盖统计。 | A010,A034 |
| `25_ACCEPTANCE_TESTS.md` | 28 | 改单行用例为可执行test spec（fixture/steps/assertions/expected/环境/owner）。 | A010,A036 |
| `26_DEVELOPMENT_PHASES.md` | 39 | 建立M0~M4发版门槛和阶段交付物；单独标识模块实验性/strict后置。 | A004,A028 |
| `27_WBS_DEPENDENCIES.md` | 69 | 补遗漏节点，修 Strict 验证器与 Advanced Author 的依赖方向，添加需求到WBS到验收映射，CI检查 DAG无环。 | A004,A036 |
| `28_WINDOWS_BUILD_RELEASE.md` | 38 | 添加Windows支持基线、安装器来源/licence、portable权限、MSVC x86 helper/CI release验证。 | A013,A026,A027 |
| `29_OPEN_SOURCE_RESEARCH.md` | 219 | 将候选库从简介改到审计表：commit、许可证、API字段、测试状态/是否可复用。 | A026,A031 |
| `30_RESEARCH_SOURCES.md` | 51 | 所有网上链接对齐release/tag/commit，记录访问日/核对字段/是否定案；不要引用漂移master。 | A026,A031 |
| `31_AUDIT_REPORT.md` | 69 | 保留历史变更，但增加open issues索引并标注并非2026-10-08全审结果。 | A011,A036 |
| `32_ADR_DECISIONS.md` | 37 | ADR每条补 alternatives/consequences/dependencies/date/status和驳回条件；Windows10策略ADR。 | A013,A028,A033 |
| `33_GLOSSARY.md` | 18 | 严格区分输入ASS PlayRes、完整输出canvas、SRC FPS关联视频、Strict vs checked等术语。 | A005,A008,A037 |
| `34_V3_CHANGELOG.md` | 14 | 将V3变更日志与V3.1/审计补丁的版本流水对应，避免“最终”误导。 | A011,A036 |
| `35_UI_STATE_INTERACTION.md` | 63 | 页面状态补并行任务、进度/部分成功、值覆盖、撤销与失焦校验；键盘可访问性。 | A015,A025,A029 |
| `36_PRESET_CONFIG_SCHEMA.md` | 41 | TOML示例经真实解析器校验；字段值枚举、迁移/冲突/renderer capability约束。 | A015,A020 |
| `37_PERFORMANCE_CAPACITY.md` | 32 | 缓存键补全部render影响项；给出RAM/IO/磁盘预算、限流阈值和进度上报约定。 | A001,A024 |
| `38_FAILURE_RECOVERY.md` | 26 | 明确目录式BDN发布事务、跨卷、旧文件被占、杀进程/取消、resume版本升级。 | A016,A025 |
| `39_IMPLEMENTATION_CHECKLIST.md` | 73 | 检查表每项链接到WBS/test/owner，标记blocked项与具体过门标准。 | A010,A036 |
| `40_SOURCE_CODE_AUDIT.md` | 192 | 源码审计补commit SHA/raw文件hash/可复现实验；明确没有运行编译和动态测试。 | A026,A031 |
| `41_V3_1_CHANGELOG.md` | 12 | 变更日志标每条受影响文件、错误级别及验证状态；审计结论仍待开发实测。 | A031,A036 |

## 目录与交叉引用审计

- 42个连续编号：00–41；**没有文件编号缺号和重复编号**。
- 文档顺序在叙述上大体合理，但35–41属于后追加的重要产品规范，不应只作为最后附录；建议新增“按主题阅读顺序”，保留现有文件名以避免已建立的引用路径失效。
- 当前大多数文档没有明确双向链接，也没有“需求 → UI → 算法 → WBS → 测试”正式追踪矩阵。
- 不能用总纲中“源码审计后的最终版”等称谓代替可执行证据；每份文件应有状态/版本/上游规范日期。