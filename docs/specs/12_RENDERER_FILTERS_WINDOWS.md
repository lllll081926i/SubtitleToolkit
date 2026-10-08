# Windows ASS 渲染器、特效滤镜与 DLL 兼容

## 1. 三条相互独立的 renderer 能力链

### A. libass（默认、内置）
- 作为默认 renderer 与 compatibility baseline；ISC。
- Windows 构建优先使用 DirectWrite font provider。
- 不承诺所有 VSFilterMod 私有特效像素等价。

当前 Rust 绑定已实现；运行仍需要外部 native DLL。2026-10-08 已纳入用户提供的 `third_party/libass-9.dll` 与其 UCRT64 依赖，并通过真实 host/像素测试，见[运行库说明](../../third_party/README.md#libass-开发期运行库)。这不表示产品安装包已完成许可审计。

#### libass 初始化硬要求（源码审计新增）
在第一次 render 前至少：
1. `ass_library_init()`；
2. `ass_renderer_init()`；
3. `ass_set_storage_size(actual_video_storage_w, actual_video_storage_h)`；
4. `ass_set_frame_size(output_canvas_w, output_canvas_h)`；
5. 按需要设置 pixel aspect；
6. 注册字体 / `ass_set_fonts(... DIRECTWRITE/AUTODETECT ...)`；
7. 再 `ass_render_frame()`。

`storage_size` 不是可有可无；libass 0.17.5 API 明确说明缺失时只能猜测，3D transform/source aspect/blur scale 可能错误。若 ASS 含合法 LayoutRes，LayoutRes 对相关计算具有优先级。

#### Windows 路径与内存输入
libass API 对 Windows filename encoding 有特别说明。主程序不应把任意 Windows `PathBuf` 直接窄化后传给 `ass_read_file()`：
- Rust 使用 Win32/WTF-16 安全路径 API 读取文件；
- 将字幕 bytes 传给 `ass_read_memory()`；
- 第四参数是 codepage，当前显式传 `UTF-8`，不能传 track 名；
- 外部/附件字体读取 bytes 后用 `ass_add_font()` 注入；
- `[Fonts]` 可通过 `ass_set_extract_fonts()` 处理；
- renderer-host 的 IPC 传文件内容或受控 staging path，而不是未经校验的 ANSI path。

#### ASS_Image composite
libass 返回 linked list，必须按链表顺序 composite；每个 bitmap 是 alpha mask + RGBA color。缓存/去重比较应以最终 composited RGBA 为准。

### B. CSRI Provider（特效兼容接口）
CSRI 是一个独立 provider ABI，不能在文档中与“任意 VSFilterMod DLL”写成同义词。
- renderer-host 必须先枚举/探测 provider 的 CSRI 导出与 capability；
- 只有确实暴露标准 CSRI / `csri_stream_ext` 的 provider 才走这条链；
- 2026 xy-VSFilter 的 release 说明表明其 external CSRI backend 使用标准 `csri_stream_ext` + `CSRI_F_BGRA`，并指出 Masaiki 的 VSFilterMod r5.3.0 可作为兼容 provider；这证明“特定 VSFilterMod build 可以是 CSRI provider”，不是证明所有历史 VSFilterMod.dll 都可以。
- 当前工作区的 `third_party/VSFilterMod.dll` 静态导出表含 `csri_*` 符号，但来源提交未核实。导出名存在不等于 ABI 或输出行为兼容；该文件尚未由本项目加载或通过真实画面对照。

### C. VSFilterMod / VapourSynth / AviSynth+ Legacy Adapter
用于没有 CSRI provider 但有 TextSubMod/TextSub/VapourSynth interface 的旧式特效滤镜。
- sorayuki VSFilterMod README 明确提供 `vsfm.TextSubMod(...)` VapourSynth 接口；
- pinterf/xy-VSFilter 提供 AviSynth `TextSub` / `MaskSub` 等接口；
- 此路径独立于 CSRI，不能在 capability probe 中混淆。

## 2. renderer-host 进程隔离

第三方渲染 DLL 不得加载到 CLI/GUI 主进程。当前已实现的隔离范围是 libass：CLI 的 `render` 与 `pgs encode` 经 renderer-host 执行；host 启动时核对 PE 位数，握手报告 provider 哈希和架构，使用 Job Object、请求超时与有上限的 stderr 尾部。

当前控制面和图像负载都走父子匿名管道。帧按 16 MiB 分块，每条消息上限 64 MiB；协议版本为 2。该实现支持 4K RGBA 帧，但会复制像素并增加管道开销。共享内存/file mapping 是后续性能选项，不是当前实现。

主进程接收队列最多缓存一条消息。接收线程至多另持有一条正在读取或等待入队的消息；队列满时等待调用方消费。管道 EOF、截断或协议读取错误只上报一次，随后退出接收线程，避免退出的 host 触发错误消息循环和无界分配。

接收端在分配负载之前拒绝未知消息标签。JSON 消息必须是合法 UTF-8；非法字节直接报错，不替换字符后继续处理路径或请求。

当前未实现：CSRI/VSFilterMod provider、x86 host、heartbeat、任务取消、minidump 和 provider pool。位数不匹配时拒绝启动，不宣称自动路由到 x86。新增 provider 前必须补 capability probe、真实 provider 测试和许可记录。

## 3. x86/x64
主程序 x64。读取 PE Machine：
- x64 provider → renderer-host-x64；
- x86 provider → renderer-host-x86；
- 其他架构拒绝并给诊断。

不得跨位数 `LoadLibrary`。

## 4. Provider discovery
扫描顺序：
1. 用户显式指定目录；
2. 应用 `renderers/` 可选目录；
3. 用户选择的 Aegisub/CSRI 目录（只读发现，不擅自注册系统 DLL）；
4. legacy runtime 用户指定路径。

每个 provider 存：SHA-256、mtime、file version、arch、license/source metadata（若可识别）、capability probe result。

## 5. VapourSynth legacy
- 当前 API 按 VSScript4 正式接口获取，不硬编码 VSAPI function-table offset；
- 动态获取版本并做最小/最大支持范围；
- 不依赖 registry 猜 Python/VS；支持 portable runtime；
- provider load failure 独立错误码；
- runtime/plugin stdout/stderr 收进 renderer log。

## 6. AviSynth+ legacy
只为旧 TextSub/TextSubMod/MaskSub 兼容；不作为普通用户的默认安装依赖。
- 使用官方 C/C++ header/ABI；
- 不手抄 `AVS_Value`；
- 显式检查接口版本；
- 文件路径优先 staging 到 Unicode-safe 短路径仅作为 legacy workaround，原始主程序路径仍保持 Unicode。

## 7. 特效帧采样
绝不能像旧 AssRocket“一事件只渲染起始帧”。需要：
- `\\t`、`\\move`、karaoke、fade、animated clip、VSFilterMod private tags；
- 静态事件可跨样本复用；
- 动态事件至少按目标视频帧采样；
- High Precision SUP 可以加入 event exact boundaries；
- HFR adaptive downsample 必须有视觉阈值、最短保持帧数、PG decoder 约束。

## 8. 变化检测
libass `ass_render_frame(... detect_change)` 只表示“位置可能变化/内容可能变化”，不作为唯一去重依据。
- 先读取 detect_change 作为 fast hint；
- 对输出事件边界做 composited RGBA hash；
- hash 命中后可选 bbox/byte compare 防碰撞；
- BDN/SUP sink 独立维护 previous-visible-frame state。

## 9. Pixel differential corpus
相同 ASS、字体、完整16:9画布、timestamp：
- libass vs CSRI provider；
- libass vs legacy VSFilterMod；
- provider version A vs B。

记录：RGBA diff、bbox、pixel count、SSIM/PSNR（仅诊断，不作为唯一正确性判断）、截图、字体 resolver 结果。

## 10. 默认推荐
普通 ASS / 大多数现代特效：libass。
明确依赖 VSFilterMod 私有行为的字幕：用户选 CSRI compatible provider；没有则提示安装/选择 legacy adapter。软件不得静默把不可支持的特效换 renderer 后仍声称等价。
