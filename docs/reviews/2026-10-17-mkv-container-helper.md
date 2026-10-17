# MKV 容器助手实现裁决（18_CONTAINER_HELPER / AT-MKV-001）

日期：2026-10-17
范围：`WBS-A002`（MKVToolNix 适配）、`AT-MKV-001`、`AUDIT-A023`（元数据保持）
代码提交：本次提交（新增 `crates/container-adapter`、`crates/cli/src/mkv.rs`、`crates/cli/tests/mkv.rs`）

## 1. 交付内容

- 新增 crate `container-adapter`：只调用外部 MKVToolNix，**不含任何 EBML/Matroska 实现**（18 §1）。
- 新增三个子命令：`mkv identify` / `mkv extract` / `mkv remux`（18 §2）。
- `doctor` 的 `not_built` 只剩 `font subset`，`mkv` 已从占位清单移出（清单由解析器推导，见 `args::not_built_commands`）。

## 2. 用真实依赖执行的验收（AT-MKV-001）

素材：MKVToolNix v102.0 自行封装的一份 4 轨 MKV（H.264 视频 + AAC 音频 + SRT 字幕 + 真实商业 PGS
字幕，后者来自本机授权的第三方 SUP 语料），另附一份真实系统 TTF。产物大小 60 MB。

结果：**23/23 断言通过**。执行脚本 `target/tmp/at_mkv_001.py`；集成用例
`crates/cli/tests/mkv.rs`（5 个用例，需要 `STK_MKVTOOLNIX_DIR` 或已安装 MKVToolNix，找不到时明确打印"跳过"）。

关键断言与独立判据：

| 断言 | 判据来源 |
| --- | --- |
| 提取 → 处理 → 新 MKV 完成 | 三个子命令 `ok=true`，产物 61 572 661 字节 |
| 原文件不改 | 源文件 SHA-256 前后一致（`3889fb9d…3b771`） |
| 字体附件存在且可抽回 | 独立 `mkvextract` 从产物抽回，与源字体 SHA-256 一致 |
| 元数据保持 | 独立 `mkvmerge --identify -J` 逐轨核对 |
| 产物合法 | 独立 `mkvmerge --identify` 识别为 Matroska，轨 0/1 保留、轨 2 改名改语言、轨 3 已丢 |
| 不原地修改 | `--out` 等于输入时返回 `STK-INP-0257`，源文件哈希不变 |

判据**全部由 MKVToolNix 自己给出**，不用本项目 reader/writer 自证往返（24 §执行记录合同的同类要求）。

## 3. 过程中修正的四个真实现缺陷

这些都不是设计阶段能想到的，全部由"用真实二进制跑"暴露。

### 3.1 `chapters` 在真实 JSON 里是对象列表，不是计数

`mkvmerge --identify -J` 的顶层 `chapters` 是 `[{...}]`，空文件是 `[]`；除此之外顶层还有
`errors`/`warnings`/`global_tags`/`track_tags`/`identification_format_version`。
我最初按 `u32` 建模，真实文件直接解析失败（`invalid type: sequence, expected u32 at line 14`）。

修正：`chapters: Vec<serde_json::Value>` + `chapter_count()`；并新增回归用例
`real_mkvmerge_v102_identify_output_parses`，其样本逐字取自真实二进制输出。

同时把 mkvmerge 自己的 `errors`/`warnings` 透出到报告（它说"某轨解码器缺失"时识别仍成功，
但下游必然撞上；吞掉上游告警等于把它变成我们的意外）。

### 3.2 选项名与取值必须是两个 argv 元素

`Command::args` 不做空格分词。把选项拼成 `"--subtitle-tracks 2,3"` 一个串，
mkvmerge 收到的是一个未知选项，**exit=2，且 stderr 为空**（它把 banner 写在 stdout）。

这一条极难查：同样的 argv 在 shell 里手打就成功，因为 shell 帮你分了词。
现场靠"失败时 stdout 只有 134 字节（banner 一行）"定位。

修正：`push_track_selection` 分别 push 选项名与取值；CLI 侧的
`--track-name`/`--language`/`--default-track`/`--forced-track` 同样改为两元素。
回归用例断言 **argv 里任何元素都不得含空格**。

附带修正 `RunError::NonZero` 的诊断：原来只取 stderr，而 mkvmerge 的主要错误在 stdout，
于是报出来是"exit=2"+一片空白。现在两份输出都进 details。

### 3.3 列表选项是 `--subtitle-tracks`，"全丢"开关是 `--no-subtitles`

拿 kind 拼字符串会送出 `--subtitles-tracks`（多了 s）。mkvmerge 把它当成**待打开的文件名**，
错误信息完全指不到真正的毛病。另一个方向：空集合曾写成 `"!*"` / `"*"`，
而 mkvmerge v102.0 把 `*` 当 IETF BCP 47 语言标签，直接报"不是有效的 BCP 47 标签"。

修正：`(list_flag, none_flag)` 两张表都显式写出；空集合走 `--no-subtitles`。
回归用例同时断言"不出现 `*`"与"不出现 `--subtitles-tracks`"。

### 3.4 "按要求改元数据"被误判成"元数据漂移"

`remux` 写完会重新 identify 并逐轨核对（AUDIT-A023）。第一版把**所有**语言/名称变化都算漂移，
于是"按用户要求把轨道改名为 `Processed ASS`"的操作被自己判失败：

```
轨道 2 language Some("jpn") -> Some("chi");
轨道 2 language_ietf Some("ja") -> Some("zh-Hans");
轨道 2 track_name Some("Japanese Audio") -> Some("Processed ASS")
```

修正：新增 `TrackEdits`，从 mkvmerge 的 `--track-name`/`--language`/`--default-track`/`--forced-track`
选项里解出"被主动改动的轨道 id"，这些项不参与漂移比对。解析故意做得窄——只认我们真会发的四个选项，
因为宽泛解析会把真漂移静默放行，那比误报严重得多。

`requested_track_edits_are_not_reported_as_drift` 双向验证：未请求时 3 项都报，请求后 0 项报。

## 4. 决策记录（不确定处的取舍）

### D-MKV-01 附件去重

**现象**：源文件里已有 `deck.ttf`，再 `--attach-font` 同一份，mkvmerge 忠实地附第二份，
产物里出现两个同名同大小附件。

**决策**：按「文件名 + 字节数」去重，已存在的同名同大小附件跳过，并在报告里列出被跳过的项
（`attached_fonts_skipped`）与策略说明（`attach_policy`）。不报告就等于静默少做一件事。

**为什么不比内容哈希**：identify 只给 `file_name`/`size`，比哈希要求在 identify 阶段把每个附件
抽出来读一遍，而本适配器的工作原则是"调用 MKVToolNix，不自己拆文件"（18 §1）。
真要判同内容，调用方应先自己比哈希再决定传不传 `--attach-font`。

### D-MKV-02 `--out` 是全局开关，不做子命令专属

`--out` 在 CLI 解析的全局阶段被消费（与 `convert`/`render` 等一致）。`mkv remux` 从 `inv.out` 取，
未给则报 `STK-INP-0260`。子命令里写 `--out` 会被全局阶段吃掉，因此解析层对该子命令拒绝这个写法并给出
正确示例，避免"看着像给了其实没给"。

### D-MKV-03 错误码段位

`Input/200` 已被 `codec-srt`（"偏移量超出可处理范围"）占用，且 `cli` 内部 `deliver.rs` 与 `mkv.rs`
本来就要在 190–200 上相撞。本命令统一改用 **250–260** 段，并在文件头注明段位归属。
`core-model/tests/error_code_uniqueness.rs` 是"这个域里哪些数已经用了"的唯一权威答案。

### D-MKV-04 失败回滚删的是自己刚写的半成品

`remux` 失败时 `std::fs::remove_file(staging)`。删的对象是本进程刚创建、名字带
`.stk-tmp.mkv` 后缀的临时产物，属于输出事务回滚（与 `task-engine` 同一条纪律），
不是清理用户资产。源文件在任何路径上都不被写。

### D-MKV-05 确定性产物

`--no-date` + `--disable-track-statistics-tags`：前者防止 "Date" 段让两次运行产生不同字节，
后者防止老版本 mkvmerge 添加源文件里没有的统计标签。这条性质在写入计划时固定下来，
不是运行时开关。

## 5. 未覆盖项（不宣称通过）

- 第 7 层设备语料：产物在真实播放器/硬件上的表现未验证（与 PGS 同一条缺口）。
- 视频/音频轨的**内容**一致性只由校验和间接覆盖（mkvmerge 逐轨校验其输出），未做逐包 diff。
- 章节编辑、转码、全量 mkvmerge 参数：按 18 §3 明确不做，报告里写出能力边界。
- `font subset` 未实现，`mkv` 与它的组合（子集字体回插）因此不可用；`--attach-font` 目前只接受完整字体文件。
