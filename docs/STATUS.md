# 交付状态与审查结论（SubtitleToolkit-RS）

> 生成时间：2026-10-06（同日第三次更新，`[Fonts]` 内嵌字体附件落地之后）。本文只写"能被命令复现的事实"。
> 复现入口：`RELEASE=1 bash tools/check_all.sh --strict`（20 个门阶）；
> 逐条证据：`docs/TRACEABILITY.md`；缺陷与决策记录：`docs/ISSUES.md`；门阶口径：`docs/REVIEW_GATES.md`。

## 1. 一句话状态

**四条格式链 + CLI（含 14 §ASS→SUP 与 14 §ASS→BDN 两条真渲染交付链）+ 分层校验 + author 规划层 + 批量任务执行（`batch`）+ 真实 libass 渲染已通过全部可跑的门阶；
GUI（Iced）、容器助手、发布链路仍未开始；字体服务到 17 §2-§4 的报告 + 注册字体 + 整份交付 + 渲染身份 + **字体注入** + **`[Fonts]` 内嵌字体的读侧**（`font scan`/`--inject-fonts` 已实现；17 §7 的两条通路都由门阶在真 CLI 输出上核对。仍未做的是 §5 subset 与 §6 的输出侧三档，DirectWrite 权威没接，见 ISSUES AG1/AG2/AG8/AK5/AK9）。**
当前不存在任何"已验证兼容硬件蓝光"的声明：按 Strict 校验一定会被判失败（这是设计，不是遗漏）。

## 2. 已实现（20 个 crate，808 个测试）

| crate | 干什么 | 关键契约 |
|---|---|---|
| `core-model` | 精确有理数时间、画布四层几何、错误模型、LossReport | 时间/几何不用浮点（ADR-007）；BDN 时间码=帧号 |
| `format-detect` | 编码/BOM/换行/格式判定 | 判定必须带置信度与理由 |
| `codec-ass` | ASS/SSA 容忍式解析 + override 分词 | parse→render 字节稳定 |
| `codec-srt` | SRT/VTT 解析与写出 | 任意字节不 panic；往返稳定 |
| `codec-pgs` | 段级读写、display set 分组、RLE、调色板、Basic Encoder、**median-cut 量化** | ODS 分片/长度不变量；全透明→erase；量化误差必须被度量 |
| `codec-bdn` | BDN XML 流式读写、PNG IHDR 一致性 | 声明尺寸==PNG 实际尺寸；单一时间源 |
| `transform-time` | fps 换算、偏移、两点校准 | 量化点唯一 |
| `transform-hdr` | HDR10/HLG 元数据行改写 | 只改色彩相关行 |
| `transform-resolution` | V3.1 四层几何 Rewrite/Preserve | Preserve 逐字节不变；两把 libass 尺度分开处理 |
| `image-core` | RGBA 位图 + bbox 语义 + 真压缩 PNG | 裁剪不改画布位置；PNG 经 Pillow 双向验证 |
| `validator` | 13 §6 分层校验（1/2/3/4/6 层）+ ASS 侧校验 | Strict 需要外部层，未覆盖就报 error |
| `pgs-author` | 变化检测/重复消除/erase 规划 + 编码自检 | 指纹含位置；空白转 erase；`author_from_plan` 让"已规划好的结果"能直接进编码（14 §ASS→SUP 的共享前半链靠它） |
| `task-engine` | 输出事务、命名模板、覆盖策略、Job/缓存 key/恢复、输入发现、分阶段并发执行器 | PartialSuccess；复用按 `(input, profile)` + 哈希 + 分量；崩溃当场进预算 |
| `render-api` | provider 能力类型、采样时间线 | 不按文件名猜 ABI |
| `render-libass` | ASS_Image→RGBA 合成 + 运行时加载 + `ass_render_frame` 真实封装 + **`ass_add_font` 内存字体注入**（17 §7；`ass_clear_fonts` 刻意不碰） | 顺序/stride/量化记账；**color 的 alpha 字节是透明度**；缺 DLL 可报告 |
| `win-job` | Job Object 子进程树约束（`media-probe` 与 renderer-host 共用一份实现） | 附着失败必须带能翻译的 Win32 错误码，不许静默退化（AE3） |
| `renderer-host` | 12 §2-§3：host 子进程 + 分帧协议 + PE 位数判定 + 崩溃/超时取证 | 异位数 DLL 在 spawn 之前就被拒；host 崩溃只终止 host 且可重试；帧长度与摘要必须自洽 |
| `media-probe` | ffprobe JSON 字段抽取、色彩/HDR 判定、建议引擎、身份哈希（ffprobe 二进制内容的 `sha256-<hex>`，外部锚点测试锁住） | 字段名以**真工具抓下来的 JSON** 为准（AF1）；二进制不可用时给可报告的依赖错误 |
| `font-service` | 17 §2-§4：字体库建库（读 `name`/`OS2`/`cmap`/`fvar`/`maxp` 表，TTC 逐成员）+ **注册表两张键**（17 §2 的"字体注册"）、ASS 真实用量抽取、候选评级与缺字报告、**渲染用的字体环境身份**（17 §7：实际用到的每个 face 的字节哈希 + resolver policy）、**字体库磁盘缓存**（`scan_cached` + `CacheMode`/`CacheState`，复核点在使用点而不是命中前，AI5） | 解析器是 `skrifa`/`read-fonts`（换掉了被 RustSec 判为 unmaintained 的 ttf-parser，AG15；`deny.toml` 的 ignore 仍为空）；不靠文件名猜字体；默认目录含用户注册字体目录；"名字对上"与"画得出字"是两件事，覆盖不足必须改判 `missing_glyph`（AG4）；判定前核对 SHA-256，字体内容变了就拒给旧结论（AT-FONT-002）；**未**使用 DirectWrite 权威（AG1）、**未**做 subset（AG2）；没被用到的 face 的索引仍可能滞后一次（AI7） |
| `cli`（`probe`） | 视频探测输出 `video_streams` + `suggestions` + ffprobe 内容哈希 | 候选值必须带证据；`recommends_speed_conversion` 恒为 false（10 §8、16 §3） |
| `cli`（`font scan`） | 真字体库 × 真 ASS：`data.matches[]` 逐条评级 + 缺字 + `scope` 自证边界 + `render_identity`（与渲染缓存 key 同一个函数算出来的）；`--out` 落盘报告；`--no-registry` 关掉注册表来源；`--copy-fonts DIR` 走 17 §6 第二/五档（复制整份字体 + `manifest.json`）；报告的 `database.cache`/`render_identity.cache` 说清这次是 `scanned`/`hit`/`stale-refreshed`/`off`，全局 `--no-font-cache` 给一条不认缓存的路；全局 `--inject-fonts` 把解析到的 face 字节交给 libass（注入前核对哈希，不符即拒渲染），报告里 `data.font_injection` 逐条列出名字/字节数/SHA-256 并分清"来自本机库/来自脚本内嵌"；脚本自带的 `[Fonts]` 由 `font_service::attachments` 解析，既走注入也进渲染身份 | 默认 report only，除 `--out` 的报告外不写产物；给了 `--copy-fonts` 才写文件，且复制件必须逐字节相同、仍能被解析器读回来（17 §8）；指错的 `--fonts-dir` 是错误而不是"库里没有"；相对路径的交付目录在解析层就退 2；`font subset` 仍占位 |
| `cli` | `subtitletool-cli`（probe/convert/time/hdr/pgs adjust/pgs validate/**pgs encode**/**render**/**batch**/**font scan**/doctor） | 单行 JSON 信封；退出码 0/1/2；未实现命令 `STK-INP-0060`；ASS→SUP 全链一路到原子写出；ASS→BDN 交出 XML+PNG 目录并按盘上 IHDR 复核；批量 = 输入发现 × 多 profile × 报告 × 恢复，目录型产物（bdn）每输入一层目录、复用判据覆盖整组文件；同一输入的 SUP+BDN 共用前半链、只渲染一次（报告给 `executions/renders/keys` 三个数） |

## 3. 门阶证据（2026-10-06 实跑）

```
STK_REQUIRE_LIBASS=1 STK_REQUIRE_FFPROBE=1 STK_REQUIRE_FONTS=1 RELEASE=1 bash tools/check_all.sh --strict
gate-selftest 0s  format 1s  clippy 1s  tests 162s  png-cross-check 1s
cli-smoke 42s  strict-badge 1s  fuzz-smoke 1s  dependency-audit 1s
advisory-audit 4s  subs-cross-check 1s  security-limits 0s  render-real 438s
renderer-host 15s  probe-real 24s  font-scan 111s（真字体库 + 注册表 + 交付 + 缓存对照）
third-party-notices 1s  concurrency-soak 17s  doc-claims 1s（三段）  release-build 31s
——20 个门阶全 PASS，fail=0 skip=0 → RESULT: PASS（退出码 0）。**每阶秒数是这一梯的快照**：同一份代码在轮次之间会漂 1~10 s（编译缓存、字体缓存、进程调度），逐阶权威在 `target/gate/summary.txt`，别把这些数字当契约。`probe-real` 含容器层外部裁判（真 ffprobe 读我们写的 .sup，ISSUES G4）；`tests` 含帧边界规则的双向基线（ISSUES AK16）与本批新增的三条 check-id 双向用例（AK17 的第一批）
```

- `tests`：`cargo nextest run --workspace` = **808 tests run: 808 passed（0 skipped，0 leaky）**，nextest 自身 150.266 s、门阶这一阶含编译 162s，含新增的容器裁判与帧边界双向基线（接入字体环境身份之后的同一阶是 497 s —— 字体库缓存把每个 CLI 进程 45 s 的重复建库去掉了，AI5）。
- `pgs encode` 端到端：真 libass 渲染一个 3 秒事件 → 77 个采样点 → 2 个显示集 7210 字节的 .sup，再用同一个二进制的 `pgs validate` 回读，`no_blocking_findings=true`。
- `render`（14 §ASS→BDN）端到端：两个 Dialogue → 恰好 2 个 BDN 事件，InTC/OutTC 落在 25fps 帧网格（`00:00:01:00`…`00:00:04:00`）；
  **AT-BDN-001 由测试自己解析 PNG IHDR 判定**（不用 `codec-bdn` 的 validator 当唯一裁判）；`--min-axis 9999` 把事件清空时如实报告"BDN 是空的"，
  且盘上没有孤儿 PNG（ISSUES AD2/AD3）。
- 批量 bdn（14 §Multi-output）：目录型产物每输入一层目录（两个输入的 `0000.png` 不会互相覆盖）；复用判据是整目录指纹——
  删掉一个 PNG 后必须重做而不是 `from-cache`，改 `min_axis` 也必须重做（ISSUES AD4/AD5）。
- `render-real`：**第一次让 native 代码进门阶**。`ass-9.dll`（libass 0.17.5）真渲染出像素，再把像素 author 成 PGS 段流与 BDN 目录并读回；缺 DLL 时 SKIP，`--strict` 下 SKIP 即 FAIL。
- `render-real` 的注入四段（17 §7，本轮新增，数字取自本梯 `target/gate/{inject,noinject,embed,plain}-check.log`）：
  开 `--inject-fonts` → `enabled=True requested=4 from_library=4 injected=4`；不开 → `requested=0` **且台账为空**（不是"没查"）；
  带 `[Fonts]` 的脚本 → `embedded=1`，且内嵌那批字节的 SHA-256 出现在台账里；去掉 `[Fonts]` 的同一份脚本身份从 `fed2b17b417b…` 变成 `8c6b40f46f69…`（AK6 的门阶版）。
  核对交给 `tools/check_injection_report.py` 做语义判断而不是 grep —— 原先那条 grep 匹配的是 JSON 键顺序，加一个字段就假红（AK7）；
  样本由 `tools/make_embedded_fixture.py` 现场生成（把一份系统字体按 libass 的真语法 `fontname:` + 折行 base64 嵌进 `[Fonts]`，字体二进制不入库，同 AK1 的理由）。
- 输出事务的失败会**点名留下来的那个诊断文件**（D13）：`task-engine` 的 verify 失败路径现在把 `目标文件未被替换；这次的内容留在临时文件里供诊断：<tmp 路径>` 与一步 `user_action` 写进错误，由 `verify_failure_keeps_old_file_intact` 断言（同一条测试还钉着『final 没被替换』与『tmp 确实留着』）。
- 探测子进程的管道必须一边跑一边排空（D10）：假 probe 吐 200 KB stderr，`chatty_stderr_does_not_deadlock_the_wait` 以 25 s 时间上界判定；结果从线程外用 `recv_timeout` 收，所以“退回老写法”会报 FAIL 而不是把门阶挂死。**反向验证过**：临时把顺序改回“先 wait 后读”，该测试在 25.01 s 处以 D10 那句消息失败；恢复修复后 0.07 s 通过。
- 输入编码（J10 的这一半）：`load_text` 现在按 BOM 解 UTF-16LE/BE，无 BOM 时只在结构能证明时才接受，解出来后 BOM 放回正文开头交给 codec 记账（AT-ASS-001 的不变量没破）；`Loaded.encoding` 与 `convert` 报告里的 `data.input_encoding` 把"这次按什么读的"说出来（`convert_reads_a_utf16le_ass_file_and_says_so` 端到端钉住）。ANSI/CP936 仍明确拒绝：无声明的编码猜错会变成"看起来正常的乱码"，那要的是显式 `--encoding`，属于决策。
- F4（13 §3.1 的 ODS 哈希不变量）：`pgs adjust` 的报告现在交出 `per_object`（逐对象 payload 的 SHA-256）与 `aggregate_sha256`（折成一个可存档的值），不再只给一个永远为真的 `equal`；逐对象摘要由外部锚点测试钉住（期望值在实现之外算出来），换平移量后 aggregate 必须相同而 PTS 必须真的不同。0 个 ODS 时不折摘要，免得空集合伪装成通过。
- `third-party-notices` 的哈希锁段（W3）：清单不再只覆盖 crates.io 那部分。`tools/notices.py` 读 `SOURCE_AUDIT_LOCK.json`，为 libass 的 dll/lib/两个头文件与 ffprobe/ffmpeg 各生成一行，**并同时重算本机那个文件的 SHA-256**（6 条全 `match`）；`absent-here` 与 `match` 分开写，`--check` 还要求清单文件本身不过期。负向验证：改锁定哈希一位 → rc=1；抹掉清单里的哈希 → rc=1。
- `renderer-host`：12 §2-§3 的失败语义真跑（AT-RENDER-001..003）。位数不符在 spawn 之前被拒；`Poison{abort}` 之后主进程拿到退出码与 libass 的 stderr，并用**新 host** 重试成功；`Poison{hang}` 走 700ms 超时并能放手。
  这一条同时纠正了 Z6 的误诊：Job Object 的结构体布局一直是错的（`ERROR_BAD_LENGTH(24)`），修好后 nextest 下的 LEAK 消失、`doctor` 报 `tree_contained=true`（ISSUES AE3）。
- `subs-cross-check`：pysubs2 1.8.1（MIT）读我们渲染的 ASS，我们读它写的 ASS/SRT；比较语义而不是字节（ISSUES U1-U4）。
- `font-scan`：**第一次把"本机字体库"这件事进门阶**。`cargo test -p font-service`（16 项，其中 6 项直接读 `C:\Windows\Fonts` 的表）
  → 真 CLI 扫 `full22.ass` 并落盘报告 → 断言信封 `ok:true`、报告写盘、`faces` 非零、`scope` 交代未用 DirectWrite 权威、报告词汇表里有 `missing_glyph`。
  实测：154 文件 → 196 face、192 note；注册表 164 条登记全部解析到文件（与 `reg query` 行数一致），本机没有"注册在别处"的字体，这一分支由构造输入测（AH2）；`--copy-fonts` 的整份交付与 `manifest` 也在这条阶里跑（逐字节 `cmp` + `--dry-run` 不落盘）；`Arial` 名字对上（`arialbd.ttf` 那条 bold）却画不出"你好"，于是被判 `missing_glyph`（AG4 的由来）；
  `Microsoft YaHei UI` 对上了但 🎬 不在 cmap 里。缺字体目录时 SKIP，`--strict` 下 SKIP 即 FAIL。
- `font-scan` 的缓存对照段（AI5/§33）：**报 `build_ms` 必须带上是谁扫出来的那一份库**。本梯：整座库（`database.faces` = **196**）在 `font-report.json` 里是 cache=scanned **45 709 ms**，同一条令第二次在 `font-report-hit.json` 里是 cache=hit **42 ms** —— 这才是磁盘缓存那一层的收益（约 1000 倍，最近四梯 45 024 → 62、44 722 → 42、44 963 → 42、45 709 → 42，量级稳定）。而 `cache-use.json`/`cache-off.json` 那两个 **311 ms / 301 ms** 是**另一个作用域**（`database.faces` = 4，字体注入对照用的窄库），拿它跟 42 ms 比会得出「缓存只省 7 倍、45 s 是假的」——那是错的，本轮差点就这么写（AK13）。三条路的 `render_identity` 摘要仍逐字节相同（`f5a6dc3e1ca7df45…`）；`--no-font-cache` 那条既不读也不写缓存，是逃生口与对照，不是优化项。
- `tests` 里的错误码身份把关（H8/AK12，`crates/core-model/tests/error_code_uniqueness.rs`）：跨 crate 不许用同一个 `(domain, code)` 说两件不同的事（要么换号，要么标题逐字相同）。对照器认两种形态 —— 直接点 `Domain::X, N, "标题"`（含按 crate 汇总的命名常量 `codes::OVERFLOW`）与**转发 helper** `fn cli_err(code: u16, msg: &str)` + 调用点 `cli_err(150, "…")`；只扫前一种会漏掉 CLI 的 110–185、image-core、codec-bdn、pgs-author、font-service 附件码整批（199 → 305 个编号、443 处使用、15 个 helper），而第 5 处撞码 `Input/151` 正是被后一条路藏住的。查出 9 处真撞码并全部换号；`STK_DUMP_CODES=1` 是编号占用的唯一权威清单（换号前先看它，别靠 grep）。把关器自己配了负基线：`the_checker_still_detects_a_deliberate_collision`（故意撞码必须报出来 —— 解析器坏了的表现恰好是"零撞码=通过"）、`comments_and_string_contents_are_not_error_codes`（注释与 `format!` 里的示例不算码）、`a_trampoline_helper_feeds_its_call_sites_into_the_table`（一级与二级转发都要进表）。- 容器层有了**外部裁判**（G4）：`pgs encode` 的产物不再只由我们自己的 reader 验收。`ffprobe_reads_our_pgs_container_with_the_same_clocks`（media-probe 的 real_probe，跑在门阶 `probe-real` 里，因此天然受 `STK_REQUIRE_FFPROBE=1` 管辖 —— 没有 ffprobe 就是失败，而不是「没跑也算过」，Z4 那条规矩）：用我们自己的编码器造两个显示集写成 .sup，让真 ffprobe 的 `sup` demuxer 读，要求段数逐段相等、每段 PTS 相等、90000 ticks 被读成 `pts_time=1.000…`；再加一条反向对照 —— 文件截断一半后不许仍报出同一串，否则这条比较就成了「任何输出都算过」。真 CLI 产物同口径：49 558 字节的 .sup，我们 `pgs validate` 数出 `data.segments=40`，ffprobe 报 40 个 packet，`ffmpeg -f sup -i … -c copy -f null -` 退出码 0。量程结论：`.sup` 容器这个字段就是 4 字节（写文件的硬上限 2^32），33bit 说的是 PES/流层（core-model `PTS_MAX_TICKS`），两处代码注释已按此改写并指向 G4。
- 帧边界这条口径修在了**裁判**身上（AA5/AK16）：`PGS.TML.05` 的网格由 `validator/src/pgs.rs` 的 `FpsTicks` 提供，而它写的是 `Time::from_rational(fps).to_pts_ticks()` —— 把 25 帧/秒当 25 秒换算，得 2 250 000 tick，于是检查实际在要求「PTS 是 25 **秒**的倍数」。我按 AA5 的说法去实现吸附，实现完 CLI 报「4 个显示集起点里 0 个被吸」而 warning 还在 —— 两边不可能同时对，才去查 validator，量出来写出的 PTS（100800/208800/320400/432000）`% 3600` 全为 0。改成 `90000 / fps`，并且**只在整数 tick 时判定**（23.976 的一帧 3753.75 tick，没有帧边界可判就跳过，不罚人）。补上缺的两条负基线：网格上的流 0 条 TML.05、非整数 tick 的网格完全不判 —— 原来那条唯一用例（半速缩放）在 bug 下也是红的，所以它为错误的理由通过。author 侧的吸附是真能力：`Plan::snap_starts_to_grid(fps)`（floor 起点 + 严格递增守卫，相邻事件不许塌进同一格），`pgs encode --snap-pts` 默认关闭（开了会改动输出字节，必须显式请求；批量 profile 不暴露）；5 条 plan 级测试 + CLI 端到端 `pgs_encode_on_a_frame_grid_raises_no_boundary_warning_and_snapping_is_inert`（钉住：0 条 TML.05、对已上格的流逐字节不变、且「0 个被移动」要写进 loss 不许静默）。真跑同一条命令现在 `display_sets=4`、`no_blocking_findings=true`、findings 里不再有 TML.05。
- `security-limits`：AT-SEC-001 用计数分配器实测"谎报 4 GiB / 8000x8000 画布 / 不终止游程 / 随机字节"都不超额分配。
- `png-cross-check`：强制 `STK_REQUIRE_PILLOW=1`，Pillow 12.3.0 读我们的 PNG、我们读 Pillow 的 7 种 PNG，逐像素相等。
- `fuzz-smoke`：确定性变异器打 4 个解析入口 + 2 个真实 CLI 入口；`convert` 200 个变异输入 → 95 失败 / 105 成功，且失败必须没有输出文件。
- `dependency-audit`：`cargo deny check` 四项 ok（licenses/bans/sources/advisories）。
- `doc-claims`：两段。①文档里点名的标识符全部真实存在（297 处，数量以 `target/gate/docs.log` 首行为准）；
  ②新增的证据链核对（`tools/audit_evidence.py`）：把 `#[test]` 全集、本次门阶真的跑过的测试、文档点名为证据的测试三者对上——
  本轮 **808 个 #[test] 全部执行、文档点名为证据的 268 个测试全在已执行集合里、0 断链**（AG17）；第三段（`tools/issue_status_audit.py`）现在查四类：缺口没被点名、已解决还被列在『没做』、**引用指不到问题行**（本轮新加，AK14：挖出 4 处注释指向已被重排掉的编号，现在 163 处引用全部指得到行），以及行解析本身 —— 以前状态带限定语的 15 行被严格正则整行跳过（AG1/AH6/F10 三条缺口因此从 §4 消失过），放宽后 263｜已解决 233｜开着 30（缺口表 30 行）。

## 4. 明确没有的东西（不要当成已完成）

| 缺什么 | 现状 | 依据 |
|---|---|---|
| A4 | 未解决 P1 | 无 CMake / Ninja / NASM |
| A5 | 待解决 P1 | 无包管理器（winget/choco/scoop 均缺） |
| A6 | 未解决 P1 | **libass crate 在本机不可用** |
| AE4 | 未解决 P1 | 大帧传输还没走 shared memory |
| AE5 | 未解决 P2 | crash code 有了，minidump 没有 |
| AE6 | 未解决 P2 | `renderer-host-x86.exe` 还不存在 |
| AF5 | 待解决 P1 | AT-PROBE-001 的"真语料"仍然只覆盖合成素材 |
| AG1 | 未解决 P2 | 17 §2 说"以 DirectWrite 为权威"，我们给的是"读表"视角 |
| AG2 | 未解决 P1 | `font subset` 没有实现，而 20 §Commands 里有这条 |
| AG8 | 未解决 P2 | 17 §6 还差三档（§7 的注入已做，默认关闭）（原文把"字体字节进缓存 key"也算在未完成里，那一半已在 §3… |
| AH6 | 未解决 P2 | 注册表读的是两张键，不是 DirectWrite 的全部真相 |
| AI7 | 未解决 P3 | 缓存命中时，**没被用到**的 face 的元数据仍是上一次的索引 |
| AI10 | 待解决 P3 | CLI 层那条"撒谎的缓存必须自愈"的测试要 127 秒，是全树最慢的单个测试 |
| AK5 | 未解决 P2 | 注入这条链的两个口子：`[Fonts]` 解析已合上，剩下的只是默认关闭 |
| AK9 | 待解决 P2 | 内嵌字体**不保证**覆盖系统里同名的字体 —— 规格没要求，要不要额外保证是产品决定 |
| AK17 | 未解决 P2 | 校验规则的双向用例覆盖是个**没量过的洞**：56 条检查点里只有 2 条有两个方向的用例 |
| C2 | 待解决 P1 | 00 §5 / 29 |
| C4 | 待解决 P1 | 06 §1 / 12 §1B |
| C6 | 未解决 P2 | 28 §1 |
| C7 | 待解决 P1 | 13 §4 |
| C8 | 待解决 P1 | 00 §5 / 29 |
| C9 | 待解决 P1 | 29 表 / 12 §1B |
| C16 | 待解决 P1 | PGS 调色板的取值范围：limited 还是 full |
| C17 | 待解决 P1 | 图末标记 `00 00 FF FF` |
| C18 | 待解决 P2 | 参考实现把 WDS 写成整画布窗口 |
| C19 | 待解决 P1 | PCS 第 5 字节（video_format 高 4 位 + frame_rate 低 4 位）的取值表 |
| F6 | 未解决 P1 | 12 §1B/C：CSRI provider 与 legacy VSFilterMod/VapourSynth/Av… |
| F10 | 未解决 P1 | 40 §5.2：字体 subset 必须过 GSUB/GPOS/ligature/Arabic/Indic/comb… |
| J10 | 未解决 P1 | UTF-16 已交付；剩下的是**无声明的 ANSI/CP936**，我们不猜 |
| N7 | 待解决 P1 | object reuse 到底做不做 |
<!-- 本表由 tools/issue_status_audit.py --gap-table 生成，共 30 行，勿手改 -->

这张表不新增判断：它就是 ISSUES 里状态不是"已解决"的每一行照搬过来，让"还欠着什么"在交付状态里同样看得见。生成与核对用的是**同一个解析器**，所以两边不一致时门阶 `doc-claims` 第三段直接报错，而不是各写各的。
## 5. 等你决策（`docs/ISSUES.md` §E）

**等你的决定（不给就不做，不猜）**：

- **E3** 真盘 SUP / 真视频语料的来源 —— 它一次卡住四条：AF5（探测的"真语料"仍是合成素材）、C16（调色板 limited/full）、
  C17（图末标记）、C19（PCS 第 5 字节取值表）、N7（object reuse 做不做）。
- **E4** 是否批准为 `font subset` 引新依赖（AG2 命令、F10 的 GSUB/GPOS/… 等价测试都排在这后面）。
- **E5 / E7** 枚举与输入面：`lossless`/`corpus_required` 是否合并成 `equivalence`；是否要求读 16 位/隔行 PNG。（E6 已作废。）
- **AK5** 字体注入是否默认打开（现在默认关；开了会改渲染输入面）。**AK9** 内嵌字体是否要互斥同名系统字体（规格没要求）。
  **J10** 无声明的 ANSI/CP936 要不要猜（现在不猜，报 111/112 并给两条出路）。

**不需要决定、可以直接继续的**（下一次接手从这里进）：

1. **AK17 剩下的 26 条**：给从未在测试里点名的校验规则补双向用例，形状照 `1c528ed`/`f0f2a26` 两批做
   （断言该报的报、干净流闭嘴、严重度不写反、一次构造不顺带触发别的检查），每批后跑
   `python tools/rule_coverage.py` 复量 —— 现在的分布是 19 只有该报 / 2 只有不该报 / 9 双向 / 26 未点名。
2. **AE4** 大帧走共享内存（P1，纯实现，无外部输入）；AE5 minidump、AE6 x86 host 二进制、AI7/AI10 缓存两处 P3。
3. **从未动过的交付面**：GUI（04/05/35/36）、`mkv` 容器辅助（18）、安装包 + Authenticode + SBOM（28 §3/§5）。

一梯约 15 分钟（`render-real` 单独占 7~8 分钟），规矩别破：**一次只跑一梯**，跑梯期间不改源码也不起第二条重流水线。

## 6. 本轮审查方法论（踩过的坑都在这儿）

1. **自家 writer + 自家 reader 的往返不算证据**：CRC-32 步进写错时我们自己的测试全绿，Pillow 直接拒收（M2）。
2. **命名会撒谎**：`allows_strict_badge()` 只看"没有 error/fatal"，不看有没有按 Strict 跑过，于是 Basic 报告也宣称徽章可用（N1/N2）。
3. **失败路径也要交付信息**：`pgs validate` 之前把已算好的发现丢掉再退非零，用户拿到退出码却不知道改哪（N3）。
4. **验收工具本身必须被验收**：PowerShell 门阶把自己的结果弄丢却报 PASS（O1）；bash 门阶被我手改坏语法（S2）。
5. **没跑过的层不能算过**：Strict/设备兼容相关的一切结论必须有外部实现或真机语料参与，否则就在报告里写成 error。
6. **新参数要么进 cache key，要么改变某次系统调用**：`--overwrite` 只解析不传递（AC1）、`min_axis` 没进缓存键（AD5）都是同一件事——参数成了文本，用户改了却没生效，而测试全绿。
7. **产物是"一组文件"时，判据必须覆盖整组**：batch 复用只看 XML 哈希，PNG 被删也会被判 from-cache（AD4）。目录型产物要么整体指纹，要么别声称能恢复。
8. **"用一个贵步骤去省一个贵步骤"要先量成本构成**：字体库缓存第一版在命中前重读整座库复核哈希，实测比不缓存还慢一倍（release：复核 2.6–4.9 s vs 全量解析 2.7 s）——因为读文件才是大头。守卫必须放在"这个值真正被用掉的那一步"，不然它保护的是没人在读的东西（AI5）。
9. **一个语义只该有一个调用点**：共享渲染原先同时存在引擎侧的分组函数与 CLI 侧 `front::RenderMemo`，后者才是真路径；本轮把前者删掉（AD9）。留着两份的代价不是重复代码，是后来者不知道该信哪一份——而"有测试、有语义"并不构成留下的理由，判据是有没有调用点。
10. **两边都由我写的约定会"自洽地错"**：ASS 的 alpha 字节是透明度——合成层和用它做夹具的整链路测试都按直通 alpha 写，两边一致、全绿，真渲染画出的是空画布（Z1）。同类问题现在靠"用对方的函数反查自己的打包"来防。
11. **工具的话术与路径都不能猜**：vcpkg 的成功判据我先是 grep 错词，后又编了一个本机不存在的状态文件路径（Y2/Y3）；证据文件（`summary.txt`/`tests.log`）如果不属于本次运行，报出来的数字就是别人的（Z5/Z7）。
12. **一条判定规则要有两个方向的用例**（AK17 的盘点，起因 AK16）：只有『该报』的用例，规则算偏严也看不出来；只有『不该报』的，规则失效同样看不出来。本轮把这条量成了工具（`tools/rule_coverage.py`）：56 条检查点里双向齐的只有 2 条 —— 这个数字本身是登记着的缺口（AK17），不是已完成。

