# 安全与健壮性

## 文件
- 大文件、极端 cue 数、zip bomb 不相关格式拒绝；
- 所有长度/offset checked arithmetic；
- Windows 长路径使用 PathBuf/UTF-16 API，不把路径强转 UTF-8 做系统调用。

## XML
禁用外部实体/网络加载；限制节点/属性/字符串长度。

## ASS
限制单行/override token 数；parser fuzz；未知 tag 保留但不执行任何脚本。

## PGS
segment length、RLE decode、palette/object id、dimensions 全 checked；避免分配攻击。

## DLL
第三方 provider 进 renderer-host；PE arch 检测；可选 hash allowlist；不在当前目录任意 DLL 搜索，防 DLL hijacking。

## 外部进程
使用绝对路径；参数数组而非 shell string；Job Object；timeout；限制输出文件位置。

## 更新
下载 manifest 必须签名/HTTPS；更新器不能静默执行未知二进制。
