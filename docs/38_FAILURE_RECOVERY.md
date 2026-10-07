# 失败处理、恢复与数据安全

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
