# 日志、错误码与诊断

## 1. 日志
`tracing` structured logging；默认 `%LOCALAPPDATA%/SubtitleToolkit/logs`；滚动 7 天/容量限制。

## 2. Correlation ID
每个 Job/Output/renderer session 都有 UUID；用户看到的错误卡可复制 ID。

## 3. 错误域
INPUT / PARSE / TRANSFORM / RENDER / FILTER / IMAGE / PGS / BDN / CONTAINER / OUTPUT / VALIDATE / DEPENDENCY。

## 4. 错误格式
`STK-PGS-0104` + short title + user action + technical details expandable。

## 5. 诊断包
zip 内容：app/version、OS、settings-redacted、job plan、logs、dependency versions/hashes、crash dump（若有）、**默认不包含原字幕/视频**；用户显式选择才附样本。

## 6. renderer-host
单独 stdout/stderr capture；超时、exit code、SEH crash/minidump 分离记录。
