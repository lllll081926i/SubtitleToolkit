# 裁决：x86 renderer host 与跨宿主缓存复用

日期：2026-10-17
范围：`crates/cli/src/rendercache.rs`、`crates/cli/src/front.rs`、`tools/`
规格：`docs/specs/25_ACCEPTANCE_TESTS.md` 的 AT-RENDER-003、`docs/development/28` §2

## 1 起因：AT-RENDER-003 从"不可达"变成"可达"

AT-RENDER-003 的原文是：

> 仅在 x86 host 产物加入支持矩阵后，验证 x86 DLL 路由；**当前缺少该产物，不验收自动路由**。

这句话把两件事绑在一起：**(1)** 产出 x86 host 并加入支持矩阵；**(2)** 验证 x86 DLL 路由。
只要 (1) 没做成，(2) 就明确"不验收"。此前 `tools/at_coverage.py` 把它记为
"无可执行证据"，理由是"需要 x86 renderer host 产物"。

本次发现 (1) **是可达的**，而此前的判断基于一个未经检验的假设。

### 1.1 那个假设错在哪

假设是"x86 host 需要 x86 的 libass DLL 才能构建"。实际上
`crates/render-libass/src/ffi.rs` 从第一行就写明：

> **不在编译期链接 libass**：用 `LoadLibraryExW` + `GetProcAddress` 在运行时取符号。

所以：**x86 host 的构建与是否存在 x86 libass DLL 完全无关。** 实测：

```
rustup target add i686-pc-windows-msvc
cargo build -p renderer-host --target i686-pc-windows-msvc --release
→ Finished `release` profile [optimized] target(s) in 22.70s
```

产物 310.5 KiB，PE 机器类型 `0x014C`（I386）—— 是真的 32 位，不是改名。

### 1.2 加入支持矩阵的实证

不只是"能编译"，而是**能跑、且与 x64 等价**。用真 libass 各渲一次，产物逐字节相同：

```
[x64] 退出码=0  产物=True
[x86] 退出码=0  产物=True
[x64] FF6AB07D3676F386  516
[x86] FF6AB07D3676F386  516
```

所以 28 §2 的 `renderer-host-x86.exe` 现在**有产物了**，不再是"待构建"。

## 2 抳出的真缺陷：渲染缓存跨宿主复用

在做 1.2 的对比时撞上一个**静默错误**。

### 2.1 现象

```
[x64] 先渲一次          → 退出码=0  cache.hit=false
[x86] 换 host 再渲同一个 → 退出码=0  cache.hit=**true**
```

第二行是错的。那只 x86 host **本来会拒绝**这个 x64 DLL —— 我把缓存关掉重跑同一件事，
它就正确地报了错：

```
$ ... --no-render-cache
STK-RND-0091: provider 与 host 位数不符（provider x64，host x86）
  - STK-RND-0201：Arch { provider: "x64", host: "x86" }
退出码: 1
```

也就是说：**host 压根没跑，它的位数校验也就没机会报错。**
用户以为自己在用 x86 host，拿到的却是另一只宿主（上一次）的像素。

### 2.2 根因

`rendercache::key_for` 的 key 由这几项组成：

```
输入内容哈希 + 帧率 + 画布 + 默认字体 + provider DLL 哈希 + 注入字体哈希
```

**没有 host 身份。** 而 `STK_RENDERER_HOST` 恰恰是可以换的，且两只宿主对同一个
DLL 的接受度不同。所以"换 host"这个动作在缓存层是不可见的。

这类错误比"慢"严重得多：它不报错、不警告、产出的文件看起来完全正常，
只是**内容可能来自另一个渲染器**。而且它会稳定复现（第一次换 host 时命中旧缓存），
用户几乎不可能自己发现。

### 2.3 修法

key 里加入 **host 可执行文件自身的 SHA-256**，并把 key 版本号从 `v1` 升到 `v2`。

```rust
pub fn key_for(
    job: &RenderJob<'_>,
    dll_hash: &str,
    host_hash: &str,       // 新增
    inject_hashes: &[String],
) -> String
```

两个细节：

1. **升版本号是必须的**，不是顺手。v1 写下的记录如果继续被 v2 算法读取，
   就还是"少的那个 key"，等于没修。升版本号让旧记录自然变成未命中并被丢弃。
   有专门用例（`the_key_version_is_bumped_so_old_records_do_not_collide`）
   手算一份 v1 算法，断言与新算法不同。
2. **host 哈希读不到时传空串**，而空串是一个**独立的身份取值**
   （用例 `a_missing_host_hash_is_its_own_identity`）。不这么做的话，
   "读不到 host" 会与"某个 host 的哈希恰为空"碰成同一个 key。

## 3 为什么 host 哈希要读**文件内容**而不是路径或时间戳

路径会变（用户可能把不变的内容换个地方放），时间戳会变（重新构建但产物一样），
而**内容**才是"哪一只宿主画了这张图"的真实答案。这与现有 key 里 DLL 哈希的
口径一致，也复用同一个 `pipeline::sha256_hex`。

代价是每次渲染要读一次 host 可执行文件（310 KiB，一次性）。这个开销对
"要不要重跑 native 渲染"这个量级的决策而言可以忽略。

## 4 负基线：不能把缓存修成永不命中

只验"跨宿主不复用"是不够的 —— **把缓存整个关掉**也能让那条用例变绿。
所以另跑一条负基线：同一个 host 连渲两次。

```
第 1 次: 退出码=0 cache.hit=false  产物字节=516
第 2 次: 退出码=0 cache.hit=true   产物字节=516
```

命中仍然成立。**修的是键，不是把功能关掉。**

## 5 验证记录

### 5.1 端到端（真 libass，真 x86 host）

修复前（同一个复现脚本）：

```
[first-x64] 退出码=0 cache.hit=false
[then-x86]  退出码=0 cache.hit=true        ← 错：host 没跑
```

修复后：

```
[first-x64] 退出码=0 cache.hit=false
[then-x86]  退出码=1 cache.hit=?  STK-RND-0091   ← 对：host 真跑并拒绝了
```

### 5.2 单元用例（`crates/cli/src/rendercache.rs`）

| 用例 | 钉住什么 |
| --- | --- |
| `keys_differ_when_only_the_host_binary_differs` | 换 host 必换 key（DLL 哈希相同也得分） |
| `a_missing_host_hash_is_its_own_identity` | 空哈希不与他人碰撞 |
| `keys_still_differ_when_only_the_provider_differs` | 没把原来的 DLL 维度弄丢 |
| `the_key_version_is_bumped_so_old_records_do_not_collide` | 旧记录不会被误当命中 |

### 5.3 门禁

```
cargo fmt --all --check                            → 干净
cargo clippy --workspace --all-targets -- -D warnings → 干净
cargo test --workspace --lib --tests --no-fail-fast   → 77/77 目标，0 失败
```

## 6 未完成（如实记录）

- **AT-RENDER-003 只做到一半。** 原文要求两件事，本次完成的是：
  (1) x86 host 产物 ✅、(2) **x86 DLL 路由** ❌ —— 后者需要一只真的 **x86 libass DLL**，
  本机只有 x64 的那份（`third_party/libass-9.dll`，机器类型 `0x8664`）。
  我验到的是"x86 host 正确**拒绝** x64 DLL"（`STK-RND-0091`），
  这**不是**"x86 DLL 路由工作"的证据 —— 它是反面：证明校验在位。
  所以 `at_coverage.py` 里 AT-RENDER-003 **仍记为无证据**，只是原因更精确了。
- **x86 host 未进安装包。** 安装器的 `legacy` 任务项仍会告警"未随包提供"
  （`tools/subtitletool.iss`）。要真正随包就要加进 stage，而它的分发前提是
  x86 libass DLL 的许可审计（21 §6）—— 那一步没做完之前不进发行物。
- **`renderer-host-x86.exe` 的文件名约定**：安装器脚本里找的是
  `subtitletool-render-host-x86.exe`，而 cargo 产物名是
  `subtitletool-render-host.exe`（按 target 分目录）。随包时需要改名 —— 未做。
- 本机未跑 `i686` 的 clippy / 测试：只有 `renderer-host` 一个 crate 在 x86 下构建过，
  其余 crate 未验证 x86 可编译。**不声称"x86 支持已就绪"**，
  只声称"x86 renderer host 能构建、能跑、与 x64 输出逐字节一致"。
