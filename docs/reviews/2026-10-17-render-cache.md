# 裁决记录：37 §2 磁盘渲染缓存（2026-10-17）

## 需求

37 §2 要求**两级** Render Cache。内存那级（批量内同 key 共享）此前已落地，
磁盘那级一直缺。规格还要求「设置容量上限和自动清理」、
「容量达到上限时先淘汰未引用产物；正在使用的产物不得删除」、
以及 §4 的「配额、峰值内存与输出大小必须有记录后再冻结默认值」。

## 交付

- `crates/cli/src/rendercache.rs`：`DiskCache` + 显式二进制编码器 + 配额执行；
- 入口挂在 `front::resolve_cached`——**两条渲染路径（SUP 的 `pgs_encode`
  与 BDN 的 `render::build`）唯一都经过的地方**。挂别处会漏批量或让两条路语义分叉；
- CLI：`--no-render-cache`、`cache stat`、`cache clear`；
- 环境变量：`STK_RENDERCACHE`（0/off 关闭）、`STK_RENDERCACHE_DIR`、
  `STK_RENDERCACHE_MAX_BYTES`；
- 目录走 `DataDir`：便携包内 `data/rendercache/`，否则 `%LOCALAPPDATA%\subtitletool\rendercache`。

## 决定与理由

### D-RC-01：key 复用 `RenderMemo` 的口径，不另立一套

复用是否成立取决于「输入内容 + 画布 + 帧率 + 默认字体 + renderer 二进制哈希」。
这个判断在 `front` 里已经钉死在内存共享的 key 上。磁盘这级如果再定义一遍，
两份定义迟早分叉，后果是**假复用**（拿 A 的像素当 B 的输出）——静默错误，比慢严重。
所以由调用方传同一个 key。

后半链参数（`exact_colors`、`level`、`min_axis`）**不进 key**：它们不改变前半链产物。
这正是跨运行复用成立的原因——换颜色档位不必重跑 native 渲染。

### D-RC-02：写进磁盘的东西必须能自证身份

文件名按 key 生成，但文件名不构成证据：磁盘可能被换过、被截过、被同名者占过。
每条记录自带 `key` 字段，读回时逐字节比对，不符即未命中并删除。
这是 23 §「不信任外部输入」在本地的落法——缓存文件是上一次进程写给这一次进程的，
中间隔着断电与无数次人为操作，它**就是**外部输入。

### D-RC-03：读失败退化为未命中，绝不报错

缓存坏了不该让用户任务失败，只该让这次慢一点。唯一例外是 `cache clear`
（那是用户显式要求的动作，失败要说出来）。

### D-RC-04：编码器不用 serde

`image-core` 的 `Patch` 没派生 serde，而给它加 `Serialize` 会把「位图层」与
「序列化格式」耦死：字节序与裁剪语义都会因此变成对外契约。
改用显式的、带魔数与长度校验的小编码器（约 90 行）。代价换来的是格式变更时
旧缓存会被**识别并丢弃**，而不是被误读成合法数据。

### D-RC-05：配额淘汰到 90%，不只是降到上限

留 10% 余量避免「刚清完又满」的抖动：每次刚好清到上限，下一个写入立刻又触发全目录扫描。
「正在使用的产物不得删除」这条在本地实现里没有对应物——每条记录在 `obtain`
里被读进内存后就不再需要磁盘那份，进程之间也不共享句柄。

### D-RC-06：缓存命中写进 `loss` 与报告

命中意味着**没跑 native 渲染**，凡是从渲染结果反推的结论都是上次那份。
不写出来，同一命令两次运行的耗时差异就没有解释。
报告里 `render_cache.hit` 与 `counters` 是显式字段，不是内部细节。

### D-RC-07：把缓存编进 SBOM 与便携包

37 §2 要求配额与容量「有记录」。新增 `Domain::Cache`（`STK-RCA-####`）而不是
并进 `Output`：缓存的故障语义与输出不同——输出失败意味着用户拿不到东西，
缓存失败只意味着慢一点。混在一个域里，读报告的人分不清「产物坏了」和「缓存坏了」。

## 实测证据（真 libass）

`crates/cli/tests/render_cache.rs`（5 用例，无 DLL 时明确跳过）：

| 用例 | 断言 |
|---|---|
| `a_second_run_hits_the_disk_cache_and_reproduces_the_same_bytes` | 第 1 次 `hit=false`、第 2 次 `hit=true`，XML 与**每个 PNG** 逐字节相同 |
| `disabling_the_cache_gives_byte_identical_output` | `--no-render-cache` 与命中给出同一份产物 |
| `a_changed_input_is_re_rendered_instead_of_serving_stale_pixels` | 改文案 `hit=false` 且位图确实变；改回原样重新命中 |
| `cache_stat_and_clear_report_the_actual_disk_state` | 报告的条数/字节数与磁盘一致；clear 后归零 |
| `the_environment_can_turn_the_layer_off_and_the_report_says_so` | `STK_RENDERCACHE=0` 时 `enabled=false` 且不写盘 |

`crates/cli/src/rendercache.rs` 单元测试（6 用例）：往返逐字段相等
（含 patch 四个坐标各自回对位置）、换 key 被拒、截断/坏魔数/尾部多余字节被拒、
patch 的 RGBA 与宽高不符被拒、配额真的生效、外来记录被计为 corrupt。

手工验证（本机真渲染）：上限 20 KiB 时 5 份 17,786 字节记录留 1 份；
上限 60 KiB 时留 3 份 / 53,062 字节。

## 开发中修掉的两个真缺陷

1. **digest 误用 `c.bytes()`**：解码端拿带长度前缀的读法去读固定 32 字节的 digest，
   把它的头 8 字节当成一个巨大长度字段 → 整个文件被判损坏。
   表现是「写成功、读回永远 miss + corrupt」。往返测试会直接抓住这类错误。
2. **`Patch` 字段序与声明序不一致**：encode 写 `x,y,w,h`，而 `Patch` 声明是
   `width,height,x,y`。写读自洽所以不报错，但读代码的人会以为它对应另一个结构。
   已按声明序统一。

## 未做（诚实边界）

- 内存那级不是容量受限的 LRU：它是批量内同 key 的 slot 共享（引用计数式），
  没有淘汰策略。37 §2 写的是「最近 frames/indexed images」，那需要帧级缓存，
  当前实现只缓存「前半链的事件补丁」，粒度更粗但正是重算代价高的那一层。
- 不测「更快」：需要基准与噪声控制，`hit` 这个显式标志才是可核对的事实。
- §6 的 criterion + end-to-end benchmark 与 CI warning 阈值仍未落地。

## 门禁

`cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、
`cargo test --workspace --lib --tests --no-fail-fast`（67 目标 0 失败）、
`cargo test --workspace --doc -j 1`、`cargo build --workspace --release`、
`python tools/notices.py --check`（493 包 / 0 问题）全绿。
