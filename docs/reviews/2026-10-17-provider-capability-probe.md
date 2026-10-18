# 裁决：provider 能力探测（12 §1/§4）与两处真缺陷

日期：2026-10-17
范围：`crates/core-model/src/pe.rs`、`crates/render-api/src/capability.rs`、
`crates/render-libass/src/{ffi.rs,Cargo.toml}`、`crates/renderer-host/src/{main.rs,protocol.rs,client.rs}`、
`crates/cli/src/{args.rs,run.rs,front.rs}`、`crates/cli/tests/provider_probe.rs`

## 1 起因：我上一轮把 AT-RENDER-002 的原因说错了

上一轮我把 AT-RENDER-002 记为"需要真 VSFilterMod provider，涉及许可审计"，然后就停了。
本轮重新审视，发现这个结论**既不准确、也不完整**：

- 工作区里**已经有** `third_party/VSFilterMod.dll`，而且在
  `SOURCE_AUDIT_LOCK.json` 里登记了来源（sorayuki/VSFilterMod，PDB 路径与上游相符）
  与 `license_status = GPL-3.0`；
- 21 §6 明写它的**默认设计就是"用户提供/可选外部 renderer provider"** ——
  即"不随安装包分发"是设计，不是阻塞；
- 真正拦住的是**别的东西**，见 §3。

所以本轮不是"继续等素材"，而是去查清到底缺什么。

## 2 查明：能力探测层是**孤儿代码**

`crates/render-api` 定义齐了 12 §1 要的东西：

```rust
pub enum ProviderKind { LibassBuiltin, Csri, LegacyVapourSynth, LegacyAviSynth, LegacyOther }
pub fn probe(filename_hint: &str, exports: &[String]) -> ProbeResult
```

但它**没有任何生产调用方**：

- 它不在任何 crate 的 `[dependencies]` 里；
- `capability::probe` 只在它自己的单元测试里被调用过一次。

生产路径上真正跑的是 `render-libass` 自己的 `LIBASS_REQUIRED_SYMBOLS`
（那一条是好的，但它只回答"像不像 libass"，回答不了"**它是什么**"）。

**缺的那一个输入**是 PE **导出表**：`core-model::pe` 只读 Machine 字段，
没有任何代码读导出名。`capability::probe` 的注释自己写着
"`exports` 由 PE 解析层提供（不在本 crate 里读 DLL，保持可测试）"——
而那个"PE 解析层"从来没被写出来。

## 3 抳出的真缺陷（三个）

### 3.1 `CSRI_REQUIRED_EXPORTS` 的名字是**编的**

原来是：

```rust
pub const CSRI_REQUIRED_EXPORTS: [&str; 4] = [
    "csri_open", "csri_close", "csri_render", "csri_get_renderer",
];
```

其中 **`csri_open` 与 `csri_get_renderer` 根本不是 CSRI API 的一部分**。

实测（本机真实 `VSFilterMod.dll`，20 个导出名）：

```
含 csri 的导出: csri_close, csri_open_file, csri_open_mem, csri_query_ext,
               csri_render, csri_renderer_byname, csri_renderer_default,
               csri_renderer_info
```

它**没有** `csri_open` —— 也就是说，**按旧集合，每一个真的 CSRI provider
都会被判成"不是 CSRI provider"**，而那四个幽灵名字还会出现在错误信息里，
让用户以为是自己拿错了 DLL。

独立参考（Aegisub 自带的 CSRI 示例、x264 的 vsfilter patch）给的调用序列是：

```c
csri_rend *r = csri_renderer_default();       // 或 csri_renderer_byname()
csri_inst *i = csri_open_file(r, path, NULL); // 或 csri_open_mem()
csri_render(i, &frame, t);
```

**修法**：按这条真实调用链取集合。

```rust
pub const CSRI_REQUIRED_EXPORTS: [&str; 3] = ["csri_renderer_default", "csri_render", "csri_close"];
pub const CSRI_OPEN_VARIANTS: [&str; 2] = ["csri_open_file", "csri_open_mem"];
```

两个 open 变体**只需其一**：只支持内存脚本的 provider 也是可用的 provider，
把它判成不可用是**假阴性**。写成两个常量而不是塞进一个列表，
是因为"两个都要"与"二选一"是不同语义。

#### 一个差点又搞错的步骤（值得单独记）

取回上游 `csri.h` 逐名核实之后，我一幽把 **`csri_request_fmt`** 也加进了必需集 ——
它在头文件里确实存在，而旧式调用序列（设格式 → 画帧）要用它。

**用例当场否掉了这个改动**：本机真实 `VSFilterMod.dll` 导出 20 个名字，
其中有 `csri_query_ext` 而**没有** `csri_request_fmt`。原因正好对应
12 §1B 那句"只有确实暴露标准 CSRI / `csri_stream_ext` 的 provider 才走这条链"——
新式路径把"设格式"挪进扩展接口了。

所以它归入 [`CSRI_OPTIONAL_EXPORTS`]。**教训**：

> 能力集合不能从"头文件里有什么"推，要从"**真的 provider 导了什么**"反推。
> 头文件描述接口全集，而实现会选其中一条路径。

可选项现在单独报出来（`csri_optional_extensions` + `provider probe` 的
`csri_optional_exports` 字段）：它们不参与可用性判定，
但告诉用户**这个 provider 走哪条路径** ——
尤其 `csri_query_ext` 是 12 §1B 点名的 `csri_stream_ext` 的入口。

### 3.2 `probe` **先加载再看符号** —— 恰好违反 12 §3

原来：

```rust
pub fn probe(path: &Path) -> Result<(LibHandle, ProbeReport)> {
    let handle = LibHandle::open(path)?;   // ← 这就是 LoadLibraryExW
    for name in LIBASS_REQUIRED_SYMBOLS { handle.symbol(name) ... }
```

两件事都不对：

1. **位数**。对 x86 DLL 调 `LoadLibraryExW`，在 x64 进程里只会拿到一句含糊的
   "不是有效的 Win32 应用程序"；而 12 §3 **直接禁止跨位数加载** ——
   "先加载再问你是谁"在位数错时已经晚了。
2. **错误质量**。`GetProcAddress` 问不出"它到底是谁"，只能回答
   "没有 `ass_library_init`"。而用户需要知道的是
   "这是一个 CSRI provider，不是 libass"——那要看导出表。

**修法**：先读导出表判能力，通过了才 `LibHandle::open`。
读导出表既不加载也不执行代码。

顺带修正了一处**错误质量退化**（我自己引入的）：改完之后"文件不存在"
一度也报 `STK-RND-0072`。改成按 `PeError` 分两种：
`Unreadable` → 71（环境问题，修路径）、`NotPe` → 72（拿错了东西，换一个）。

### 3.3 host 的诊断在**进程边界上丢掉了**

host 是唯一真的看过那个 DLL 的地方。但：

- `reply_err(w, code, title)` 只传 **code + title**，`StkError.details` 丢；
- 协议 `Response::Error { code, title }` **没有 details 字段**；
- CLI 的 `render_host_error` 用 `format!("{}：{error:?}", ...)` 包，
  于是那份诊断变成 `Protocol("…")` 这种 Debug 输出。

三层各丢一点，结果是**用户最后看到的还是那句旧的"不是可用的 libass provider"**，
而 host 其实已经知道答案了。

**修法**：三层各补一处——

- 协议加 `details: Vec<String>`（`#[serde(default)]` 保持向后兼容）；
- host 加 `reply_stk_err`（带 details）并在 provider 打开失败时用它；
- client 四处 `Response::Error` 分支都拼上 details（`join_details`）；
- CLI 的 `render_host_error` 把 `Protocol(s)` 里的正文当一条普通 detail 输出，
  而不是 `{:?}` 包一层。

修好之后用户看到的：

```
code  = STK-RND-0091
title = renderer-host 协议不符：host 拒绝握手：STK-RND-0072 候选 DLL 不是可用的
        libass provider（"…\VSFilterMod.dll" 看导出表是一个 **CSRI provider**
        （不是 libass）。CSRI 通过 renderer-host 的 CSRI 链调用，
        不经过 libass 的 ABI）
```

这才回答了用户的问题。

## 4 交付：`provider probe` 子命令

把能力探测从一个孤儿模块变成用户能用的东西：

```
subtitletool-cli provider probe <dll> [<dll>…]
```

**只读 PE 头与导出表，不加载任何 DLL** —— 所以它对 x86 的 DLL 也安全，
而这正是"在加载之前就知道"的意义。

实测输出（真文件）：

```
probed=2  any_usable=True
  VSFilterMod.dll
    arch=x64  kind=csri  usable=False
    verdict=是 CSRI provider，不是 libass；当前只有 libass 链已实现（12 §1B）
  libass-9.dll
    arch=x64  kind=  usable=True
    verdict=可作为 libass provider
```

三个设计选择：

- **一次能问多个**。用户手上常有几个候选（`renderers/` 里一堆 DLL），
  一次一个地跑会让人来回对照。
- **逐行报错而不是整条失败**。"其中一个读不到"不该让另外几个的结论也拿不到。
- **结果里自报"没有加载任何 DLL"**（`meaning` 字段）。
  与 `check-updates` 同一条纪律：把"这次到底做了什么"写进结果，
  而不是写在文档里等人去读。

## 5 这次交付**不**含什么（说清楚，免得被当成已做）

- **CSRI 渲染链没实现**。探测能认出 CSRI provider，但 `renderer-host` 仍然
  只有 libass 一条链。`usable_as_libass = false` 就是这件事的如实表达。
- **VSFilterMod 没做真实渲染对照**。AT-RENDER-002 要的是"证明它的崩溃
  只终止 renderer-host，GUI 可重试"——那需要 CSRI 链 + 一次真的崩溃注入。
- **`third_party/VSFilterMod.dll` 仍未进安装包**，且在 21 §1 下**不应**进：
  它是 GPL-3.0。探测它是"读文件"，不是"分发它"。
- **`ProviderKind::LibassBuiltin` 从未被产出**。`probe` 里没有任何分支返回它，
  因为 libass 走的是另一条判定路径（`pe_exports_have_libass_required`）。
  这个枚举值现在是个死值 —— 留着是因为规格里有这条链，
  但**没有代码用它是事实**。

## 6 验证记录

### 6.1 真 DLL 实测（两个候选都判对了）

```
VSFilterMod.dll：导出 20 个名字 → 能力判定 Some(Csri)
                 CSRI 符号齐全 = true（按修好的集合）
                 libass 必需要素 = false
libass-9.dll   ：导出 50 个名字 → 能力判定 None（不是 CSRI）
                 libass 必需要素 = true
```

### 6.2 端到端（改进后的错误文案）

见 §3.3。libass 侧无回归：`ok=true, pngs=1, canvas=1920x1080`，产物落盘。

### 6.3 用例

| 位置 | 数量 | 核什么 |
| --- | --- | --- |
| `core-model/src/pe.rs` | +8 | 导出表解析：合成 PE、大小写敏感、无导出表、截断、谎报计数、**真 kernel32**、**真 libass**、**真 VSFilterMod** |
| `render-api/src/capability.rs` | 21 | 含两条新回归：真实 CSRI 导出集被认成 CSRI；两个幽灵名字不是必需项 |
| `cli/tests/provider_probe.rs` | 9 | 走真二进制；含"真 libass 被认成可用"与"真 VSFilterMod 被认成 CSRI" |

### 6.4 门禁

```
cargo fmt --all --check                              → 干净
cargo clippy --workspace --all-targets -- -D warnings → 干净
cargo test --workspace --lib --tests --no-fail-fast   → 81/81 目标，0 失败
```

## 7 对 AT-RENDER-002/003 状态的更新（仍不验收）

- **AT-RENDER-002** 仍然**无证据**。本轮把它的**前置条件**从"缺素材"
  修正为"缺 CSRI 渲染链"。规格要的是"证明它的崩溃只终止 renderer-host"，
  而那需要先有那条链。**探测 ≠ 渲染**，本裁决不把它算作验收。
- **AT-RENDER-003** 仍**无证据**（需要真 x86 libass DLL）；
  但本轮的 `provider probe` 让"x86 DLL 路由"多了一个可用的核对工具：
  它能对一个 x86 DLL 报出位数与能力，**而不去加载它**。

`tools/at_coverage.py` 的两条说明已按这个新认识更新。
