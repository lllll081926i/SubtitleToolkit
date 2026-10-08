# 批处理、多输出与任务引擎

文档修订：2026-10-08。当前 CLI 从 JSON spec 调用单命令执行链；已有 Job/profile、取消标记、恢复凭据和逐输出报告。GUI、TOML preset、完整渲染分组缓存属于目标。参数与当前示例见 [CLI 规格](20_CLI_AUTOMATION.md)。

## 1. Job
`Job = InputSet + Pipeline + Vec<OutputProfile> + OutputPolicy + RuntimePolicy`。

## 2. 输入发现
- file list；
- folder；
- recursive；
- include/exclude glob；
- paired files（IDX+SUB）；
- optional same-name video association。

## 3. 并发
- Parse/transform：CPU pool；
- Render：按 renderer provider 设 semaphore；
- Image quant：CPU pool；
- PGS author：CPU；
- Disk IO：bounded；
- 外部 process：单独 limiter。

## 4. 多输出共享
缓存 key 必须包含：
`input hash + transform hash + canvas + fps + renderer identity + font environment hash + render options`。

`render options` 必须覆盖 StorageSize/PAR、颜色算法与采样策略。帧共享先满足 [RenderGroupKey](09_CANVAS_RESOLUTION_MULTI_OUTPUT.md#6-rendergroupkey-与时间采样)，再讨论缓存；相同 timestamp 不允许跨组共享像素。

## 5. 命名模板
`{stem}.{profile}.{width}x{height}.{fps}.{hdr}.{ext}`；非法 Windows 字符自动 sanitize。

## 6. 覆盖策略
Skip / Rename / Replace-after-verify。绝不直接 truncate 原文件。

## 7. 恢复
`.jobstate.json` 记录 stage 和 artifact hashes；程序崩溃后只复用 hash/metadata 验证通过的 cache。

当前 `JobState` 已有产物、cache key 与工具版本核对。目标的统一 JobSnapshot 还应冻结输入摘要、归一化参数、字体/provider 身份与 schema 版本。恢复验证通过只允许复用对应 stage；不能因某个输出存在就将整个任务改为成功。dry-run 不写恢复凭据。

## 8. 报告
批量完成生成 JSON/HTML（后期可选）摘要：成功、失败、warning、outputs、耗时、renderer/version。

## 9. 目标状态机与执行边界

目标执行状态为 `Queued → Probing → Ready → Running → Verifying → Committed`。取消请求先进入 `Cancelling`，待原生进程与 writer 收尾后才报告 `Cancelled`。最终任务结果单独汇总为 Success、PartialSuccess、Failed 或 Cancelled；已成功提交的 profile 不因另一个 profile 失败而回滚。

开始前冻结参数并展开全部输出路径。预检输入/输出重合、profile 重名、Windows 大小写等价路径、保留名、尾随点/空格与覆盖策略。名字规范化后再次检查冲突，不能仅对原模板字符串去重。目录事务与锁定文件恢复见[失败恢复](38_FAILURE_RECOVERY.md)。

同一任务中的 Draft 编辑不影响冻结参数。取消后的残留 staging 与已提交产物分别报告；不得在进程仍写出时报告完成取消。当前宿主尚无 Cancel 消息，目标取消协议不能当成已接通能力。
