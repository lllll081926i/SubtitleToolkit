# 安全与健壮性

## 文件
- 大文件、极端 cue 数、zip bomb 不相关格式拒绝；
- 所有长度/offset checked arithmetic；
- Windows 长路径使用 PathBuf/UTF-16 API，不把路径强转 UTF-8 做系统调用。

**大文件拒绝（2026-10-17 已实现）**：`pipeline::read_bounded`。

| 上限 | 值 | 依据 |
|---|---|---|
| 字幕文本（`load_text`） | 1 GiB | 3 小时 5000 行 ASS 不到 5 MiB；带内嵌字体的几百 MiB；比真字幕高两个数量级 |
| `probe` / SUP / BDN / 批量输入 | 4 GiB | 本机真 SUP 是 98.97 MB，4K 长片版到 GB 级不奇怪 |

关键点是**先 `metadata()` 看长度再读**：`fs::read` 会先把整份读进内存，
之后再拿大小判断就已经晚了 —— 机器内存已经吃完，而用户可能只得到一句无关的 OOM。

错误码 `STK-INP-0113` 单独立一个，而不并进 0110（无法读取）：
“文件太大”与“文件读不了”的用户动作完全不同（换输入 vs 修权限）。
详情里同时给出**实际字节数与上限**，并指出本工具处理字幕而不是视频。
`probe` 也要查：它是用户“拿错文件”时最可能先跑的命令。

已知缺口：`codec-*` 内部的 cue/段数上限不成体系（不像 PGS 段那样逐处 checked），
没有一条“极端 cue 数”的端到端用例。

## XML
禁用外部实体/网络加载；限制节点/属性/字符串长度。

## ASS
限制单行/override token 数；parser fuzz；未知 tag 保留但不执行任何脚本。

## PGS
segment length、RLE decode、palette/object id、dimensions 全 checked；避免分配攻击。

## DLL
第三方 provider 进 renderer-host；PE arch 检测；可选 hash allowlist；不在当前目录任意 DLL 搜索，防 DLL hijacking。

**实现状态（2026-10-17）**：

- **PE arch 检测：已实现**（`core_model::pe`）。单份实现在 core-model 而不是两份：
  主程序（加载前自检）与 renderer-host（真的加载前再查）都要这个判断，
  两份实现对**同一个 DLL 会给出不同答案**，那就是一边放行一边拒绝。
  只读 PE 头（不加载、不执行），先取 4 KiB，头靠后时只补读 `e_lfanew` 处的 6 字节。
- CLI 侧三种失败**保持可分**（这是本条要求的可判定形式）：
  不存在 `STK-INP-0141` / 不是有效 PE `STK-INP-0142` / 位数不符 `STK-INP-0143`。
  位数那条的详情里写明"本程序是 x64，该库是 x86"，并给出能照做的补法。
  **为什么必须在这里判而不是交给 `LoadLibrary`**：后者只会返回一句含糊的
  “不是有效的 Win32 应用程序”，用户看到的信息就只剩这一句。
- `JobSnapshot` 里的 `arch` 从**二进制 PE 头**读，不再写死：写死会让
  “当时用了哪个二进制”那句变成谎话，而那是 JobSnapshot 的全部用途（20 §Repro）。
- **未实现**：可选 hash allowlist（当前靠绝对路径 + SHA-256 进报告，没做白名单强制）；
  DLL 搜索路径加固已有（只接受 `--libass-dll` / `STK_LIBASS_DLL` 给的绝对路径，
  不做当前目录搜索），但没有写成一条专门的测试。

renderer-host 与 Job Object 提供崩溃隔离、超时收尾与资源管理，不构成权限沙箱。插件仍可能访问当前用户可访问的文件与网络。当前未实现受限令牌或完整性级别隔离，产品不得宣称恶意 DLL 安全执行。

当前 libass 采用绝对路径、受控 DLL 搜索与 provider 身份核对。ABI 导出匹配只证明存在候选接口；真实加载与画面测试分别报告。新增 provider 必须定义依赖 DLL 搜索策略，不能靠放宽到任意 PATH 目录掩盖缺失依赖。

## 外部进程
使用绝对路径；参数数组而非 shell string；Job Object；timeout；限制输出文件位置。

## 更新
下载 manifest 必须签名/HTTPS；更新器不能静默执行未知二进制。
