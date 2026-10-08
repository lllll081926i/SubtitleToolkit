# 批处理、多输出与任务引擎

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

## 5. 命名模板
`{stem}.{profile}.{width}x{height}.{fps}.{hdr}.{ext}`；非法 Windows 字符自动 sanitize。

## 6. 覆盖策略
Skip / Rename / Replace-after-verify。绝不直接 truncate 原文件。

## 7. 恢复
`.jobstate.json` 记录 stage 和 artifact hashes；程序崩溃后只复用 hash/metadata 验证通过的 cache。

## 8. 报告
批量完成生成 JSON/HTML（后期可选）摘要：成功、失败、warning、outputs、耗时、renderer/version。
