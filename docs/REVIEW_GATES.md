# 审查门阶清单（28 §7 CI 等价 + 24 §QA + 39 §清单 + 25 §AT）

这个文件的存在理由：26/39 要求"按阶段验收"，但本机没有 CI runner。
于是把验收做成**一条可执行命令 + 一份可核对的清单**：`bash tools/check_all.sh --strict`。
清单里每一项都写清楚：谁检查它、检查什么、当前状态、以及"没做过的事不能被叫过"的边界。

## 1. 门阶本身

| 门阶 | 检查内容 | 依据 | 当前状态 |
|---|---|---|---|
| `gate-selftest` | `bash -n tools/check_all.sh`：门阶脚本自己先要语法可执行 | 28 §7 | 通过 |
| `format` | `cargo fmt --all --check` | 28 §7 | 通过 |
| `clippy` | `cargo clippy --workspace --all-targets -- -D warnings` | 28 §7、23 | 通过（0） |
| `tests` | 全部单元 + 集成测试（本机用 `cargo nextest`，输出落 `target/gate/tests.log`） | 28 §7、24 §QA | 通过（**754 tests run: 754 passed，0 skipped，0 leaky**；曾经那例 LEAK 的真因是 Job 结构体布局错，见 Z6/AE3。`tools/audit_evidence.py` 会核对这 745 条与文档点名的证据是否对得上，AG17） |
| `png-cross-check` | **外部实现参与**：Pillow 读我们写的 PNG、我们读 Pillow 写的 | 24 §QA、25 AT-BDN-001 | 通过（`STK_REQUIRE_PILLOW=1` 强制不许自跳；`Pillow 12.3.0`，`VERIFY OK 4` + 7 个反向参考文件逐像素相等） |
| `cli-smoke` | 真二进制契约：信封字段、单行 JSON、退出码 0/1/2、`--dry-run` 不写文件、未实现命令报 `STK-INP-0060`；外加 **batch 多输出 + 第二次运行必须复用**、**doctor 未实现清单不许含已实现命令**、**缺 libass 时 `pgs encode` 与 `render` 都必须退 1 且不留半成品**、**非 16:9 画布是用法错误退 2** | 20、22、25、15 §1/§7、09 §1 | 通过 |
| `strict-badge` | 按 `--level strict` 校验必须被 `PGS.DEC.01`/`PGS.DEV.01` 拦下，且 findings 仍然完整交出 | 13 §6、39 | 通过 |
| `fuzz-smoke` | 确定性变异器（9 种真实损坏方式）× 4 个解析入口 + 2 个真实 CLI 入口 | 28 §7 fuzz smoke、23 §健壮性、38 §1 | 通过（`convert` 变异结果 `failed=95 succeeded=105`；失败分支必须没有输出文件） |
| `dependency-audit` | `cargo deny check licenses bans sources`（离线可做） | 21 §1/§11、28 §1 | 通过：**bans ok, licenses ok, sources ok** |
| `advisory-audit` | `cargo deny check advisories`（联网拉 RustSec DB；走仓库内临时镜像配置，不动用户全局） | 28 §1 | 通过：**advisories ok**（直连 github.com 会随机超时，见 ISSUES T5） |
| `subs-cross-check` | pysubs2 读我们写的 ASS/SRT、我们读它写的（比较**语义**：时间、layer、style、name、边距、effect、text、PlayRes、SBS） | 24 §QA、25 AT-ASS-001 | 通过（`STK_REQUIRE_PYSUBS2=1`；3 个参考文件） |
| `concurrency-soak` | task-engine 的并发/崩溃测试重复跑 12 轮（`cargo nextest --runs 12`） | 15 §3、38 §3、24 §QA | 通过（43 项 × 12 轮全绿；这条是因为同一断言在 cargo test 下绿、nextest 下红才加的，见 ISSUES AB2） |
| `probe-real` | **真 ffprobe + 真视频**：`tools/make_probe_fixtures.sh` 生成的 1080p25 / 4K23.976+PQ / 4:3 三个素材，走完整链并核对色彩与候选 | 16 §2-§5、25 AT-PROBE-001 | 通过（5 项 e2e + CLI 输出的 `is_hdr`/`hdr-preset`/`recommends_speed_conversion:false` 三条 grep；缺二进制时 SKIP，`--strict` 即 FAIL） |
| `security-limits` | 恶意 length/RLE 不崩溃**且不超额分配**：测试装计数分配器实测字节数 | 25 AT-SEC-001、23 §健壮性 | 通过（5 个用例；最坏随机输入分配 <40 MiB） |
| `render-real` | **native 依赖真跑**：`ass-9.dll` 渲染出像素 → author 成可读回的 PGS 段流 → 再交付成 BDN 目录（`cli` 的 `render` 端到端 + `batch` 的 bdn 目录型产物 + 共享渲染的 `renders < executions`） | 12 §1A、13 §4/§5、14 §ASS→BDN、14 §ASS→SUP（**不等于**下一条 `renderer-host`：像素对不对与"第三方 DLL 崩了能不能关住"是两件事，见 TRACEABILITY §27） | 通过（libass 0.17.5；`render_real` 2 项 + `pgs-author` pipeline 5 项 + `cli` render/bdn/share 测试；缺 DLL 时 SKIP，`--strict` 下 SKIP 即 FAIL） |；最后一段是真 CLI 的注入回路（17 §7）：`render --inject-fonts` 的报告交给 `tools/check_injection_report.py` 做**语义**核对（`ok` 为真、缺字段即报错、`requested = embedded + from_library`、开注入时台账条数 = `requested` 且每条 `sha256` 是 64 位小写十六进制互不重复、`render_identity.digest` 是 SHA-256；没开注入时台账**必须为空**）。原先这里用 grep 匹配 `"font_injection":{"enabled":true`，而 serde_json 按字母序排键，加一个字段就把 `enabled` 挤走、门阶假红（AK7），故换掉。再往下一段证内嵌通路：`tools/make_embedded_fixture.py` 现场把一份系统字体 base64 成带 `[Fonts]` 的脚本（字体二进制不入库，同 AK1），要求那批字节的哈希出现在台账里、`embedded >= 1`，且**去掉 `[Fonts]` 的同一份脚本算出不同的 `render_identity`**（AK6 的门阶版）。"没开"与"开了但没字体"、"开了但只从字体库取字节"这三种结论因此都能分开读 |
| `renderer-host` | **12 §2-§3 的失败语义真跑**：构建 `subtitletool-render-host.exe` → 位数不符在 spawn 前被拒 → provider 崩溃只终止 host 并带回退出码与 stderr → 挂住走超时 → `doctor` 如实报告能力与孙进程约束状态 | 12 §2/§3、23 §外部进程、28 §2、AT-RENDER-001..003 | 通过（host 侧 5 项 + lib 侧 10 项；`tree_contained=true`，job_error=0 —— 修正 AE3 之前这里一直是 24） |
| `font-scan` | **真字体库 + 真 ASS + 真注册表 + 真交付**：`cargo test -p font-service`（含 `real_fonts` 7 项，直接读 `C:\Windows\Fonts` 的表与两张 Fonts 键）→ 真 CLI 扫 `full22.ass` 并落盘报告 → 检查信封认领命令、报告真的写盘、`faces` 非零、`scope` 交代未用 DirectWrite 权威、`missing_glyph` 档在报告词汇表里、注册表解析到的条数不为零 → 再用一份临时字体目录跑 `--copy-fonts`（manifest 的 `mode`/`subset_sha256: null` + 复制件逐字节 `cmp`）与它的 `--dry-run`（不落盘）；报告还必须带 `render_identity`（渲染缓存 key 的字体环境那一维，§32）；
最后一段锁字体库磁盘缓存（AI5/§33）：第一跑必须是 `scanned`，同一件事第二跑必须 `hit` 且 `build_ms` 真的下降、`render_identity.digest` 逐字节不变，`--no-font-cache` 那条既不读也不写缓存文件且身份仍是同一个 | 17 §2-§4、25 AT-FONT-001/002、20 §Commands | 通过（本机实测 154 文件 → 196 face、192 note；缺字体目录时 SKIP，`--strict` 即 FAIL。加这一阶的理由见 ISSUES AG3/AG4：合成 fixture 测不出"Arial 画不出中文"这类真事实，而 `Missing Glyph` 这一档正是被真跑才暴露出被吞掉的） |
| `third-party-notices` | `tools/notices.py --check`：从 Cargo.lock + registry 缓存生成 `THIRD_PARTY_NOTICES.md`，并按 21 §1 白名单校验每个 SPDX 表达式；另一段是 **native/exe 的哈希锁**：清单里每条产物都由 `SOURCE_AUDIT_LOCK.json` 的锁定值与**本机重算值**并排给出，状态分 `match` / `absent-here`（这台机器没有那个文件，哈希仍是锁定的事实、但不谎称实测过）/ `MISMATCH`；实测不符或清单过期（哈希不在文件里）都判 FAIL（W3） | 21 §11、28 §2 | 通过（43 个第三方包，0 许可问题；6 条 native/exe 产物本机全 match；字体解析换成 fontations 一系的 `skrifa`/`read-fonts`/`font-types`，都是 MIT OR Apache-2.0；`advisory-audit` 同时把 ttf-parser 的 unmaintained 判定清掉了，见 ISSUES AG15） |
| `doc-claims` | 三段。①`tools/doc_claims_check.py`：文档反引号点名的标识符必须在代码树里存在；**核对范围只含 TRACEABILITY 与 REVIEW_GATES 这两份『交付声明』文档** —— `ISSUES.md` 故意不在名单里，那篇叙述文大量引用外部 API（IDWriteFactory、CreateProcess、FFmpeg 的 avio_rb32 等），要求它们在 crates/ 里存在只会制造假红；范围写在这儿，免得下一个人以为 ISSUES 的说法也被机器管着。②`tools/audit_evidence.py`：被 TRACEABILITY/ISSUES **点名为证据**的测试，必须在**本次**门阶的执行记录里真的跑过（存在≠执行过，见 ISSUES AG17）。它曾抓到一次改名漂移。③`tools/issue_status_audit.py`：文档内部一致性，现查四个方向 —— 开着的缺口必须在 STATUS/TRACEABILITY 被点名；已解决的编号不得再被『没做』清单引用（AJ1/AJ3/AJ4 的来源）；**引用必须指得到问题行**（本轮新加：代码注释让读者去查的编号早已在某次重排中被并掉，前三类检查看不见这个方向，因为它们只看 ISSUES ↔ STATUS/TRACEABILITY —— 挖出 4 处断链，记在 AK14）；**行解析本身不许静默跳行**（状态格带限定语的行整行不被解析，277 行表里跳过了 15 行状态行，AG1/AH6/F10 三条开着的缺口因此从 §4 消失过；放宽后问题总数从 246 涨到 261）。第四方向配了 `--ref-selftest`：喂一条假引用与一条真引用，必须前者被抓、后者不误伤 —— 解析器坏掉时的表现恰好是「零断链=通过」（与 AK12 同族）。STATUS §4 那张「开着的缺口」表由 `--gap-table` 生成，不再手抄（手抄版曾把已解决的 AH5 留在缺口里）。 | 39、22、24 §QA | 通过 |
| `release-build` | `cargo build --workspace --release` 产出 `subtitletool-cli.exe` **与** `subtitletool-render-host.exe`（28 §2 的两个产物） | 28 §2 | 通过（`RELEASE=1` 时纳入） |

2026-10-06 实跑（`font-service`/`font scan` 与门阶 `font-scan` 落地后）：
`STK_REQUIRE_LIBASS=1 STK_REQUIRE_FFPROBE=1 STK_REQUIRE_FONTS=1 RELEASE=1 bash tools/check_all.sh --strict`
→ **20/20 PASS，fail=0 skip=0，`RESULT: PASS`（退出码 0）**，
`cargo nextest run --workspace` = **754 tests run: 754 passed, 0 skipped**（0 leaky —— Z6 报的那例 LEAK 真因是 Job Object 结构体布局错，AE3 修正后消失）；clippy `-D warnings` = 0；fmt 干净。
门阶从 15 → 16（`render-real`，第一次让 native 代码进门阶——在此之前"全绿"其实从没碰过渲染器）→ 17（`concurrency-soak`，并发与崩溃路径重复 12 轮，抓调度相关的假绿）→ 18（`renderer-host`，位数/崩溃/超时这三种失败必须在进程边界外解决）→ 19（`probe-real`，真 ffprobe + 真视频：schema 类缺陷只有真工具能抓出来，AF1 就是例子）→ 20（`font-scan`，真字体库 + 真 ASS：`Missing Glyph` 这一档就是被这一阶逼出来的，AG4）。
`render-real` 现在还往下多走两步：BDN 目录的 CLI 端到端（`cli --test render`）+ 批量里的 bdn 目录型产物（`cli --test batch bdn`），
因为"渲染对了"不等于"交付对了"。
门阶抓到过：一次"编辑后只跑了测试没跑 lint"（ISSUES Q5）、一次改名漂移、
`summary.txt` 把十几轮历史追加在一起（ISSUES Z5）、`tests.log` 在 nextest 分支根本不写而留着上次的残留（Z7）、
一次**门阶自己的前提过期**：`cli-smoke` 一直拿 `pgs encode` 当"占位命令不许假成功"的例子，
把它接上真实实现后这条检查立刻 FAIL（ISSUES AA1）——占位例子换成 `mkv`/`font`，同时补了正向检查：
（这一族问题后来又撞了三次：AD7 的 `render`、AG2 的 `font`——`font scan` 落地之后 `font` 已经不是占位命令，
门阶注释与测试里的例子现在写的是 `font subset`；教训是"清单必须问解析器要"，见 `not_built_commands()`。）
缺 libass 时 `pgs encode` 与 `render` 必须退 1、报 `STK-INP-0141`/`STK-INP-0110`、且不留下半成品（本轮第二次撞同一族问题，见 ISSUES AD7）。
逐条结果落在 `target/gate/summary.txt`（每次运行清空重写），完整日志 `target/gate-probe.log`。

`--strict` 的语义：**SKIP 也算失败**。这是刻意的——第一版门阶用 PowerShell 写，因为作用域问题把
自己的结果丢了，摘要空白却报 `RESULT: PASS`（ISSUES O1）。从那次起规则是：
门阶必须先把"自己记录了什么"证明出来，才轮到它去证明产品。

## 2. 文档要求 → 门阶/测试映射（能审的部分）

| 文档要求 | 落点 | 证据 |
|---|---|---|
| AT-CANVAS-001/002（四层几何、Preserve 字节不变） | transform-resolution | `preserve_is_byte_identical_on_a_dirty_script` + 42 个相关测试 |
| AT-TIME-001（单一时间源）、10 §7（禁止两处舍入） | core-model/time、transform-time、codec-bdn | `bdn_timecode_is_a_frame_index_not_wall_clock`、`timecodes_are_derived` 守卫 |
| AT-BDN-001（XML 尺寸==PNG 尺寸） | image-core + codec-bdn | `tests/bdn_png.rs`、`tests/pillow_cross.rs` |
| AT-OUT-001（失败不留半成品覆盖原文件） | task-engine 输出事务 | `task-engine` 事务测试（先 tmp → fsync → 重开校验 → rename） |
| 13 §4（Basic Encoder 必须项） | codec-pgs、pgs-author | `pgs-author/tests/author.rs`（erase 表达、PCS 画布坐标、DTS<PTS、显示集计数） |
| 13 §5（image quantization：合并可以，静默不行） | codec-pgs/quantize | `median_cut_stays_inside_budget_and_is_deterministic`、`alpha_ramp_survives_as_a_ramp_not_a_binary_choice`、`real_libass_output_flows_into_a_valid_pgs_stream`（失真进 LossReport） |
| 12 §1A（真实渲染器语义不能靠记忆） | render-libass | `render-real` 门阶 + `color_unpack_is_rgba_with_inverted_alpha`（ASS 的 alpha 字节是透明度） |
| 13 §6（分层校验 + 不许假通过） | validator | `strict_badge_requires_strict_level_and_no_error_or_fatal`、DEC/DEV 升 error |
| 09 §3（bbox 裁剪不改位置） | image-core、pgs-author | `crop_then_place_back_is_pixel_identical`、`authored_stream_is_valid_and_keeps_canvas_positions` |
| 22 §错误模型 | cli | `cli-smoke` 门阶 + `tests/cli.rs` |

## 3. 还没到过"可以叫审完"的门槛（必须逐条承认）

- `THIRD_PARTY_NOTICES.md` 由 `tools/notices.py` 生成（version/license/repo/.crate SHA-256 + 非 crates.io 组件），
  **native/exe 的哈希也进清单并逐条本机实测**（W3：libass 的 dll/lib/两个头文件 + ffprobe + ffmpeg，6 条全 `match`；产物本体仍不入库，21 §7）。
  还没做的是 SBOM（CycloneDX 之类）与 mkvtoolnix —— 后者要等 `mkv` 助手落地，接的那天按同样方式锁哈希。
- ~~native 依赖没落地：vcpkg 失败、`installed/` 里没有 `ass/` 头文件、`render-libass` 与 `renderer-host-x64/x86` 与像素 diff 语料全部空缺、`pgs encode`/`render`/`batch`/`font` 一律返回 `STK-INP-0060`~~
  **这一段整片过期，本轮按现状改写**：libass 0.17.5 已经由 vcpkg 装好并被门阶 `render-real`/`renderer-host` 真跑（哈希锁见上一条与 W3），
  `render-libass`、`renderer-host-x64`、`pgs encode`、`render`（14 §ASS→BDN）、`batch`、`font scan` 都已实现并进门阶。
  仍然成立的两点：占位命令只有 `mkv` 与 `font subset`（它们照旧显式返回 `STK-INP-0060`，不许假成功），
  以及**像素 diff 语料**与 `renderer-host-x86`（32 位 provider）依旧空缺 —— 前者等 12 §9 的设备语料（AF5、E3），后者等 32 位构建产物（AE6）。
- 13 §5 Advanced Author（object reuse / palette animation / epoch / windows layout /
  ahead buffering / coded object buffer / acquisition point）未实现；validator 第 5、7 层
  未覆盖，因此 Strict 徽章按定义不可能自封（这正是门阶要证明的）。
- Iced `app`（04/05/35）、`container-adapter`（18）、媒体探测真语料（16）未开始；
  `font-service`（17）已到「§2-§4 报告 + 整份交付 + 渲染身份 + §7 两条通路（`--inject-fonts` 注入与 `[Fonts]` 读侧解析）全部交付并进门阶」；
  仍未做的是 §5 的 subset 等价 corpus 与 §6 的**输出侧**三档（copy subset / 把字体 embed 进产物 ASS / MKV 附件）（ISSUES AG2/AG8/AK5/AK9）。
- 发布链路（28 §2-§6：installer、portable、Authenticode、SBOM、诊断包脱敏）未开始。
- fuzz/语料：24 §QA 要求的 corpus 与设备矩阵在本机没有依据，只能声明"未覆盖"。

## 4. 复核入口（人肉审查时用这些命令，不是看本文档自述）

```bash
bash tools/check_all.sh --strict            # 全门阶（缺依赖即失败）
RELEASE=1 bash tools/check_all.sh --strict  # 加 release 构建
cargo test --workspace                      # 只看测试
cargo clippy --workspace --all-targets -- -D warnings
grep -rn "STK-INP-0060" crates/cli/src      # 未实现命令的出口
```

日志：`target/gate/`（tests/pillow/strict/deny/release 各自一份）与 `target/gate-run.log`。
