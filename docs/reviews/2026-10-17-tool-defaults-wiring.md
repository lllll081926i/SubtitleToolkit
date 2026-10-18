# 裁决：ToolDefaults 接入层

日期：2026-10-17
范围：`crates/cli/src/defaults.rs`（新增）、`crates/cli/src/{args,run,render,mkv,subset,batch}.rs`
规格：`docs/specs/36_PRESET_CONFIG_SCHEMA.md` §1
前置：[ToolDefaults 存储层](2026-10-17-tool-defaults.md)

## 1 为什么接入要单独做一轮

上一轮交付了 ToolDefaults 的**存储与求值**，但 CLI 命令**还没有**在
"用户没给参数时"去查它。那状态下这一层只是"能存能看"，**不是"真的生效"** ——
而这正是那一轮末尾明确标出的关键缺口。

分开做的理由（当时写下的，事后证明成立）：接入层每一处都要单独验证
"不写参数时确实变了、写了参数时确实没变"，而存储层是独立可验的。
把它们混在一次改动里，"存储对了但接入漏了一个命令"这类错误难以定位。

## 2 求值为什么要收在一处

新模块 `crates/cli/src/defaults.rs`，顺序是：

```
用户给的值  →  ToolDefaults 里的值  →  编译期默认值
```

三个来源的地位不同：

1. **用户显式给的**永远优先 —— 命令行是当下的意图，设置文件是过去的偏好。
2. **ToolDefaults** 是跨会话的偏好，但**不覆盖**本次命令行的显式选择。
3. **编译期默认值**是兜底，且**只有一份**（`PgsEncodeDefaults` /
   `RenderDefaults` 这些结构里），不在别处再写第二份。

收在一处的两个理由：

- 各命令各自查会有"用户值 vs 默认值"的优先级被重新解释一遍的问题，迟早不一致；
- "这次跑的到底是哪些取值"（20 §Repro 的前提）就无法统一回答。
  收在一处之后，求值变成**可单独测试的纯函数**（16 个单元用例）。

## 3 关键的实现选择：解析阶段只记"用户给了什么"

`args.rs` 里 `pgs encode` / `render` 的 `fps`/`canvas`/`level`/`default_font`/
`min_axis` 从**具体值**改成了 `Option<T>`。

理由：原先在解析阶段就填成 `ValidationLevel::Standard`，到执行阶段已经
**看不出**用户到底给没给 —— 而那正是"用用户的值"与"用默认值"的分界。

所以：`Some` = 用户给了，`None` = 去查 ToolDefaults。

## 4 不合法就报错，不静默退回

设置里的值可能拼错（`standardd`）。处理与命令行参数错**一样**：报错退出
（`STK-INP-0219`），而不是静默用默认值。

**静默退回会让用户以为自己的设置生效了，而他其实在用别的值。**

差异只在"改法"上：设置错的改法指向 `tool-defaults set`：

```
STK-INP-0219: 工具默认值里的校验级别无法解析
  - pgs encode.level = "standardd"
  只能是 probe|structure|basic|standard|strict；改它就跑：
  tool-defaults set "pgs encode" level standard
```

注意那句"改它就跑"是**照抄能用的命令**，而不只是列出合法取值 ——
用户已经知道取值集，他需要知道改的是哪一项、用什么命令。

## 5 报告里要多一项 `resolved_from`

`pgs encode` 与 `render` 的输出新增 `resolved_from`，逐项报出
`命令行` / `tool-defaults` / `内置默认`。

理由：用户问的是"我设的生效了吗"。只有取值**不回答**那个问题 ——
同一份报告在"设了 `level=standard` 而默认恰好也是 `standard`"时，
与"从没设过"长得一模一样。这个字段把两者分开。

同时把读设置时的警告也带进报告（`tool_defaults_problems`），
否则"我设了却不生效"会变成一件没有线索的事。

## 6 抳出的三个问题（都由既有门禁当场报出）

### 6.1 跨 crate 错误码冲突（两处）

`core-model` 的 `no_two_crates_share_an_error_code_with_different_meanings`
是上一轮建立的防线，这次连报两次：

**(a) `Input/31`**：我给 `render` 的画布校验自造了 `Domain::Input/31`，
而 `core-model/src/canvas.rs` 已用它表达"特效字幕逻辑画布必须是完整 16:9"，
且两处标题不一样。

修法**不是换号，而是不重写判定** —— 改用
`Geometry::for_canvas(canvas).validate()`（09 §1 的唯一事实源，`codec-bdn`
也走同一条）。手写 `is_16_9()` 会造出第二份规则，而两份规则迟早会对同一个
画布给出不同答案 —— 那时报告里就会出现"同一输入两种结论"。

**(b) `Input/220`**：那是 `font-service` 的"字体索引不存在"。换到 `221`。

### 6.2 同名常量让测试无法判断

同一个测试还报：

```
crate core-model 里常量 CURRENT_SCHEMA 同时等于 1 和 1，编号无法确定
```

因为 `settings.rs` 与 `tool_defaults.rs` 各有 `CURRENT_SCHEMA = 1`，
测试靠"同名常量 = 同一个编号"来核对，同名但不同语义会让它无法判断。

**不是弱化测试**，而是把名字改清楚：`TOOL_DEFAULTS_CURRENT_SCHEMA`。
名字区分开既让测试能继续把关，也让读代码的人一眼看出是哪份文件。

## 7 一个刻意的行为变化（且更好）

画布的 16:9 校验从**参数解析阶段**搬到了**执行阶段**：画布可能来自
ToolDefaults，要到求值之后才知道。

于是 `render` 的非 16:9 画布从

- 旧：退 2 + stderr 文本
- 新：退 1 + 带错误码（`STK-INP-0021`）的 JSON 信封

变成**更好**：信封带错误码（脚本能判因）与可机读的 details，stderr 文本不能。
`crates/cli/tests/render.rs` 的相应用例已同步（并改了名，从
`..._as_a_usage_error` 改成 `..._with_a_coded_error`）。

### 一个顺序修正

`render` 的画布校验现在在**载 DLL 之前**做。原先它埋在 `codec-bdn` 深处，
于是"缺 DLL"会抢在"画布不对"前面报 —— 而画布错是更便宜、更根本的一类。
用例也据此改为不依赖本机是否装了 libass。

## 8 覆盖了哪些命令

| 命令 | 接的键 |
| --- | --- |
| `pgs encode` | `fps`、`canvas`、`level`、`default_font` |
| `render` | `fps`、`canvas`、`default_font`、`min_axis` |
| `mkv` | `tools_dir` |
| `font subset` | `subset_dir`、`fallback_full` |

**`batch` 明确不接**：spec 里的值是**已解析的确定值**，spec 自己就是一份
完整的参数快照（36 §1 的 JobSnapshot 层）。让它去查用户的 ToolDefaults 会让
**同一份 spec 在不同机器上跑出不同结果**，而"可重放"正是 spec 的用途。
`command_for` 因此一律传 `Some(...)`，并把这个理由写在代码里。

**开关不进 ToolDefaults**（`exact_colors`、`embed_fonts`、`snap_pts`）：
开关的语义是"这次要不要做这件事"，而不是"过去我偏好什么"。

## 9 验证记录

### 9.1 端到端（真 libass，真二进制）

```
1. 没设 ToolDefaults
   → canvas 1920x1080，resolved_from 四项全是"内置默认"

2. 设 canvas=1280x720 后再跑同一条命令
   → canvas 真的变成 1280x720（产物 3633 字节 vs 5117 字节）
   → resolved_from.canvas = "tool-defaults"

3. 再显式给 --canvas 1920x1080
   → canvas 回到 1920x1080，resolved_from.canvas = "命令行"（压过设置）

4. 设 level=standardd（拼错）
   → 退 1，STK-INP-0219，不是静默退回

5. font subset 不給 --subset-dir，而设置里给了
   → 真的写到设置指的那个目录（产出 ARIAL_0.74d696e6.subset.TTF
     + subset-manifest.json）
```

### 9.2 用例

- `crates/cli/src/defaults.rs`：16 个单元用例
  （优先级三种组合、不跨工具泄漏、五类非法值、格式宽容度、
   `min_axis=0` 是取值而非缺失）
- `crates/cli/src/args.rs`：旧的"解析阶段就有默认值"用例改成
  "解析阶段只记用户给了什么"（`pgs_encode_records_only_what_the_user_gave`）
- `crates/cli/tests/render.rs`：非 16:9 画布的用例同步到新行为

### 9.3 门禁

```
cargo fmt --all --check                              → 干净
cargo clippy --workspace --all-targets -- -D warnings → 干净
cargo test --workspace --lib --tests --no-fail-fast   → 79/79 目标，0 失败
cargo test -p core-model --test error_code_uniqueness → 4 passed
```

## 10 未完成（如实记录）

- **Preset（36 §1 第三层）** 仍卡在 TOML 依赖审批（ISSUES E4）—— 外部决策。
- **GUI 无入口**。
- **`fallback_full` 只接了一半**：求值与生效都做了，但命令行没有"显式关"
  的写法（`--fallback-full` 只能置真）。所以设置里的 `true` 无法被本次命令行
  推翻。这一点写在 `subset.rs` 的注释里，不让它成为暗坑。
  要真正对称得加 `--no-fallback-full`，那是另一个决定（开关的命名约定），
  本次不做。
