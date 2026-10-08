# UI 状态机、交互与排错规范

本文为未实现 GUI 的目标规格。须联合阅读[页面控件](05_UI_SCREEN_SPEC.md)、[配置模型](36_PRESET_CONFIG_SCHEMA.md)与[任务状态](15_BATCH_TASK_ENGINE.md)。有页面或按钮设计不等于对应核心 stage 已交付。

## 1. 页面状态统一模型
每个工具页统一五态：`Empty -> Probing -> Ready -> Running -> Result/Error`。

- Empty：只显示输入区和示例说明。
- Probing：文件已加入，异步探测；参数控件可先显示 skeleton，不能冻结窗口。
- Ready：输入合法，显示手动参数和建议。
- Running：参数快照锁定；允许 Cancel，不允许悄悄改变正在运行的 Job。
- Result/Error：显示产物、验证等级、日志入口。

页面五态描述当前输入草稿；后台任务独立保存 Queued、Running、Cancelling、Cancelled、PartialSuccess、Failed 等状态。切换页面不取消任务，部分成功不显示为全部成功。开始按钮由与 CLI 同源的参数和能力校验驱动，未实现的功能不能提交执行。

## 2. 参数来源标记
每个参数值保存 `ValueOrigin`：
- UserExplicit；
- Preset；
- InputMetadata；
- VideoProbe；
- AutoRecommendation；
- Default。

优先级：UserExplicit > Preset > InputMetadata/VideoProbe > Recommendation > Default。
UI 用小图标表示来源；视频 probe 不能覆盖 UserExplicit。

## 3. Dirty 与 Snapshot
用户编辑的是 Draft；点击“加入队列”生成 immutable JobSnapshot。运行中的任务永不读取 Draft。

## 4. 错误展示
- 输入错误：控件 inline error；
- 依赖错误：页面顶部 dependency card；
- 运行错误：任务抽屉 error card；
- 详细 stack/command 放“技术详情”，普通区域只给可执行解决办法。

## 5. 拖放路由
Iced 0.14 `FileDropped(PathBuf)` 逐文件触发。
- subtitle extension/magic -> 当前工具输入；
- video -> 若当前页面允许，填 Target Video；
- mkv -> 打开 Container Helper 或弹路由选择；
- folder -> 批量页面扫描；
- mixed drop -> 先分类，再一次性显示确认摘要。

## 6. 批量编辑
在 Inputs/Profile table 支持：
- multi-select；
- apply-to-selected；
- duplicate output profile；
- enable/disable profile；
- column filter；
- 搜索文件名。

## 7. 可解释性
任何自动动作必须可以回答：
“为什么建议这个？”、“会改什么？”、“是否有损？”、“如何撤销？”

## 8. UI 调试能力
开发构建提供 `Ctrl+Shift+F12` Dev Panel：
- current page state；
- last Messages；
- Task list；
- cache keys；
- renderer sessions；
- external process commands（敏感路径可脱敏）；
- layout bounds / DPI。
利用 Iced time-travel/debug tools，复现 UI 状态问题。

## 9. 交互验收条件

纯键盘可到达输入、输出、开始、取消与错误详情；焦点可见并提供可访问性名称。IME 组合输入期间不提交半成品参数。1366×768 与 100%/150%/200% DPI 下，面板可折叠或滚动，开始与取消始终可到达。输入发现与递归扫描在后台执行并可取消。

每个控件登记配置字段、ValueOrigin、启用条件、验证错误、核心 stage 和验收项。导入包的[界面审查建议](../subtitletool_full_audit_20261008/07_UI_AND_INTERACTION_AUDIT.md)可作为细化依据，其中 VTT、MKV、x86 自动路由等控件必须按实际 capability 决定是否提供。
