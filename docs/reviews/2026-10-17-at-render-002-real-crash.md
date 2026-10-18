# 裁决：AT-RENDER-002 —— 用真第三方 provider 崩溃证明进程隔离

- 日期：2026-10-17
- 相关规格：12 §1B/§2、22 §6、25 AT-RENDER-002
- 相关提交：`8b63ec0`（本条）、`22bb06d`（CSRI 接进 host）、`4b78431`（注入点）
- 相关用例：`crates/renderer-host/tests/at_render_002.rs`

---

## 1 规格要求

> 取得可用 VSFilterMod provider 后，证明其崩溃只终止 renderer-host，GUI 可重试；
> 自伤 `Poison` 测试不能代替该 provider 测试。

三句话拆开：

| # | 要求 | 反面 |
|---|---|---|
| 1 | **其崩溃** | 不能是注入、不能是 poison、不能是模拟 |
| 2 | **只终止 renderer-host** | 主进程必须活着 |
| 3 | **GUI 可重试** | 崩完还能接着干 |

---

## 2 我上一轮的错误结论

上一轮的裁决文档里我写：

> **没做到**：让**第三方 DLL 自己**真的崩。那需要它的代码里有一个可触发的缺陷
> 或一个我们能控制的崩溃入口 —— 两者我们都没有，而"故意喂坏数据让它崩"
> 不是可复现的测试，是碰运气。

**这段话有两处错。**

第一处：**"没有可控制的崩溃入口"是错的。** CSRI 是一个 ABI，它的每个参数
都由**我们**填。我们有充分控制权。

第二处：**"碰运气"是错的。** 正确的做法不是随机喂坏数据，而是**逐条问**：
这个 provider 对**每一条违反契约的输入**分别会怎样。那是 18 个定向用例，
不是碰运气。

教训：**"做不到"要先试着证伪。** 我用一个论断代替了一次探针。

---

## 3 探针：18 种违约输入，逐个在子进程里跑

每个用例在**独立子进程**里跑 —— 崩掉是要观察的结果之一，不是失败。

| 用例 | 退出码 | 结果 |
|---|---|---|
| `baseline` | 0 | 正常渲染 |
| `open-mem-null` | 0 | 返回空（防御了） |
| **`open-mem-bad-len`** | **-1073741819** | **0xC0000005 访问违例** |
| `open-mem-truncated` | 0 | 正常返回 |
| `open-mem-utf16` | 0 | 正常返回 |
| `open-file-null` | 0 | 防御了 |
| `open-file-bad` | 0 | 防御了 |
| `fmt-zero-size` | 0 | 防御了 |
| `fmt-huge` | 0 | 防御了 |
| `fmt-unknown-pixfmt` | 0 | 防御了 |
| `render-null-plane` | 0 | 防御了 |
| `render-huge-stride` | 0 | 防御了 |
| `render-nan-time` | 0 | 防御了 |
| `render-1e300-time` | 0 | 防御了 |
| `render-neg-time` | 0 | 防御了 |
| `render-tiny-plane` | 0 | 防御了 |
| `ext-null-name` | 0 | 防御了 |
| `ext-garbage` | 0 | 防御了 |

**18 中 1 崩。** 这个分布本身是有信息量的：它不是"到处都在崩"，
而是"**恰好缺一道检查**"。所以我们不是在挑一个随机输入碰运气，
而是在利用一个**已知的、单一的**缺失。

### 3.1 崩的那一条

```c
csri_open_mem(csri_rend_t *rdr, const void *data, size_t length, csri_openflag flags)
```

`length` 若大于 `data` 的真实长度，VSFilterMod **不做上界检查**，
顺着读下去直到越界。探针给的是 `1 << 28`（256 MiB）而缓冲只有约 300 字节。

### 3.2 为什么正常路径碰不到

host 会先 `read_bounded` 校验长度（23 §输入上限），所以交给 provider 的
长度永远是准的。**这道缺陷在正常路径上不可达** —— 要用它就得绕过那道校验，
而那正是 provider 自己缺失的那道。

---

## 4 实现

### 4.1 谎报长度必须**看得见**

```rust
pub fn open(path, script, script_path_for_open_file, width, height) -> Result<CsriProvider>
pub fn open_with_declared_len(..., declared_script_len: Option<usize>) -> Result<CsriProvider>
```

正常调用走 `open()`（内部传 `None`）。**写成一个显式参数而不是翻环境变量**，
是为了让读代码的人在签名上就看见"存在一条能谎报长度的路径"，
而不会以为 `script.len()` 是唯一的真相。

host 侧只有在 `STK_HOST_LIE_ABOUT_LENGTH` 被显式设上时才传 `Some`。

### 4.2 两个开关**不是**一件事

第一版把新开关塞进 `HostSpec::crash_injection`，host 直接报
"未知的崩溃注入种类"并退 78。因为那字段写的是 `CRASH_ENV`：

| 开关 | 让谁死 | 验的是 |
|---|---|---|
| `STK_HOST_CRASH` | **host 自己**（abort/exit/panic/hang） | 宿主启动阶段没了会怎样 |
| `STK_HOST_LIE_ABOUT_LENGTH` | **provider 自己** | 第三方 DLL 崩了会怎样 |

拆成两个字段（`crash_injection` / `lie_about_script_len`），因为揉在一起
会让人以为它们是一回事。

### 4.3 硬故障下"失败时写现场"永远不跑 ← 实测抳出的真问题

第一版实测：

```
crash_note: None
崩溃目录：空
```

原因：访问违例是**内核直接掐进程**，不执行任何 Rust 返回路径。
而一个没有现场的崩溃记录对用户几乎没用 —— 用户只看到"host 没了"。

修法：在**调 provider 之前**先把现场写下去。代价是每次打开 provider
多写一次盘；换来的是"真崩了也有记录"，而那正是这条 chain 存在的理由。

### 4.4 WER 对话框会把整条管道挂住 ← 实测抳出的真问题

`renderer-host` 的**存在前提**就是第三方 DLL 会崩（12 §2），
而那些崩溃绝大多数是访问违例。Windows 的默认行为是弹一个模态对话框，
等一个不会来的人去点"关闭"。

于是一个**本该被隔离机制吞掉**的崩溃，变成用户面前的弹窗，
而且 **host 在等对话框、客户端在等 host** —— 整条管道冻住。

这正是 AT-RENDER-002 要防的事，所以在 host 自己的进程里设：

```c
SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX);
```

**关掉的是对话框，不是崩溃** —— 进程照样死、退出码照样 `0xC0000005`。

注：不能只靠父进程继承。`SEM_NOGPFAULTERRORBOX` 会随 `CreateProcess` 继承，
但那只在父进程也设了的情况下成立 —— 发布形态里父进程是 explorer.exe，它不设。

**这也是为什么这条用例不再需要人点错误报告。**

---

## 5 验收证据

真 VSFilterMod.dll，无模拟、无注入、无 poison：

```
主进程活着：Crashed { exit_code: 3221225477 (0xC0000005), contained: true }
崩溃现场：阶段=load-script；已运行=4ms；provider=x64；
         provider sha256=87a9c3c1ab7614eda22b8fb367840a87fd34c0c7fb92d7c4d3aa6bb0b2a6ee6b；
         原因=打开 provider 前
重试成功：4233 个不透明像素
现场落盘：crash_dir/host-crash.json
```

逐条对上：

| # | 要求 | 证据 |
|---|---|---|
| 1 | 其崩溃 | `0xC0000005`，出处是 `csri_open_mem` 的越界读 |
| 2 | 只终止 renderer-host | 主进程存活；`contained: true`；拿得到现场 |
| 3 | GUI 可重试 | 换一个 host 重跑同一份输入，4233 个不透明像素 |

用例：`a_real_vsfiltermod_access_violation_only_kills_the_host`
（含重试那半）与 `the_real_crash_note_is_written_to_disk`。
**两条都核了素材存在性，缺素材时失败而不是跳过。**

---

## 6 清理

临时的崩溃探针 example 已移入 `.trash/probe_malformed/`。它每次运行都会
触发访问违例，而 `clippy --all-targets` 会编译 examples ——
**这就是上一轮"构建一直报错"的真实原因**（不是产品代码）。
它的结论已固化进本文件。

---

## 7 仍未做

- **VapourSynth / AviSynth 两条 legacy 链**（12 §1C）未实现 —— 与 CSRI 独立，
  各自需要一套宿主接口。它们的崩溃隔离同样可复用本机制，但**未验证**。
- **x86 provider 路由**（AT-RENDER-003）：本机只有 x64 的 provider。
- `VSFilterMod.dll` 仍不随包发（21 §1，GPL-3.0）。
- 本机制证明的是"**这一个** provider 的**这一条**缺陷下的隔离"。
  换一个 provider 需要重跑探针 —— 因为上表那个 18 中 1 崩的分布是
  VSFilterMod 特有的。
