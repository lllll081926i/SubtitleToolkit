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

renderer-host 与 Job Object 提供崩溃隔离、超时收尾与资源管理，不构成权限沙箱。插件仍可能访问当前用户可访问的文件与网络。当前未实现受限令牌或完整性级别隔离，产品不得宣称恶意 DLL 安全执行。

当前 libass 采用绝对路径、受控 DLL 搜索与 provider 身份核对。ABI 导出匹配只证明存在候选接口；真实加载与画面测试分别报告。新增 provider 必须定义依赖 DLL 搜索策略，不能靠放宽到任意 PATH 目录掩盖缺失依赖。

## 外部进程
使用绝对路径；参数数组而非 shell string；Job Object；timeout；限制输出文件位置。

## 更新
下载 manifest 必须签名/HTTPS；更新器不能静默执行未知二进制。
