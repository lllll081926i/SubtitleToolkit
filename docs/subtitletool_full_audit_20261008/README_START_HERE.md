# SubtitleToolkit-RS — V3.1 全开发文档审计包（2026-10-08）

**这是审计报告和建议修订规范，并非更新后的可执行程序或完整修订版V3.2原文档。** 附带原始V3.1基线42份文件（未覆写），以保证问题证据行号可以复查。

建议按下列顺序阅读：

- `00_SUMMARY.md`：最终结论与整改边界。
- `01_ISSUES.md`：38项按P0/P1/P2排序的问题、原文证据、正确做法、验收方法。
- `02_ALL_DOCUMENTS.md`：42份文件各自的审查结论。
- `03_IMPLEMENTATION_CONTRACTS.md`：新建议的核心数据结构、算法、renderer/PGS/HDR/批量UI合同。
- `04_WBS_REBUILD.md`：59个经过无环校验的新工作包/依赖/里程碑。
- `05_VERIFIED_SOURCES.md`：联网核验来源和能否支持原主张的边界。
- `06_REMEDIATION_GATES.md`：整改验收关卡。
- `07_UI_AND_INTERACTION_AUDIT.md`：逐页面控件、参数、交互和异常/键盘/DPI验收。
- `08_READER_ORDER_AND_TRACE.md`：42份文件的推荐逻辑阅读顺序、需求-代码-测试追踪表。

可机器读取：`issues.json`、`WBS_REBUILT.json`、`BASELINE_SHA256.json`。原始文件位于ZIP的`subtitle_tool_rust_planning_v3_1/`，本审计位于`subtitletool_full_audit_20261008/`。

压缩包解压在同一目录后执行 `python subtitletool_full_audit_20261008/validate_audit.py`，会验证全部源文件SHA256、行号、38项issue引用、59个任务ID和DAG环。

**没有做**：没有在Windows上编译Rust GUI、运行复杂ASS渲染、加载实际VSFilterMod DLL、模拟蓝光硬件buffer、拿到官方封闭蓝光规范进行逐条认证、做版权/许可证法律意见。不能把静态文档审查误认为最终软件测试通过。
