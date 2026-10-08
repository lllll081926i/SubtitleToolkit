# 失败处理、恢复与数据安全

本文规定恢复目标。当前已有单文件事务、批处理恢复凭据与部分成功分类；BDN 多文件目录事务、统一宿主 Cancel 协议和断电恢复矩阵仍需独立验收。相关职责见 WBS-J004/J005 与[任务引擎](15_BATCH_TASK_ENGINE.md)。

## 1. 原文件
所有输入以只读打开；“覆盖输出”也先写同目录临时文件，再验证替换。

## 2. Partial Success
多输出任务中：1080 ASS 成功、SUP 失败时，保留已经验证成功的 ASS，任务状态 `PartialSuccess`；明确列出失败 profile。

## 3. renderer crash
- host 退出；
- 主程序捕获 IPC EOF/exit code；
- 自动最多重启一次（仅无用户数据风险时）；
- 第二次失败停止该 output，不循环重启；
- 其他不依赖 renderer 的 outputs 可以继续。

## 4. 磁盘不足
执行前按估算检查；运行中 write error 立刻停止新 stage；清理临时文件；缓存清理不得删除当前 job 正引用 artifact。

## 5. 断电/崩溃
临时文件带 UUID；启动时扫描 stale temp，默认提供“清理”而不是直接删除用户可能想保留的数据。

## 6. 外部程序版本改变
ffprobe/mkvtoolnix/provider hash 进入 JobSnapshot；恢复任务发现版本变更时使相关 cache 失效。

## 7. Cancel
协作式取消；writer 完成一个 atomic unit 后停止；renderer-host 可先发 cancel，超时再 terminate process tree。

当前宿主协议没有 Cancel 请求；不得宣称这一目标已经实现。

## 8. 文件与目录事务边界

单文件先写最终目标同卷的临时文件，关闭并重新读取验证后提交。Replace 先保留旧目标；安装失败则恢复旧目标。恢复失败时报告备份与临时路径，禁止用空成功结果掩盖错误。

BDN 是 XML 与全部 PNG 的集合。目标实现须在最终目录同卷 staging 中完成全部文件，校验 XML 引用与 PNG 尺寸后发布整个集合；文件占用、提交中断与回滚分别报告。不能只原子替换 XML，却留下不完整的图片集合。

跨卷搬运不是原子 rename。若中间产物在别的卷，先复制到目标同卷 staging，再完成验证与提交；旧目标不提前删除。该路径必须有实际失败测试，不能只依据调用顺序宣布断电安全。

残留 staging、备份与用户产物按所有者和任务身份区分。自动恢复只复用摘要、版本、schema 与 stage 状态全部核对通过的产物；不能扫描任意目录后清理无关会话文件。
