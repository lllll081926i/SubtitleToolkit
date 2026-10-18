# 裁决：36 §1 第三层 Preset（TOML）与 ISSUES E4 的解除

- 日期：2026-10-17
- 相关规格：36 §1/§2/§3/§4/§6、21 §1、20 CLI
- 相关提交：（本条）
- 相关用例：`crates/core-model/src/preset.rs`（13）、`crates/cli/tests/preset_cli.rs`（8）

---

## 1 E4 到底卡在哪 —— 一个行政手续，不是政策冲突

E4 记的是「是否批准新增 `toml` crate（MIT/Apache-2.0）」。
我核了**权威来源**（crates.io API，2026-10-17）：

```
crate: toml
max_version: 1.1.8+spec-1.1.0
license: MIT OR Apache-2.0
repository: https://github.com/toml-rs/toml
```

**`MIT OR Apache-2.0`** —— 正是 21 §1 明确允许的宽松家族
（"默认安装包核心优先 MIT/Apache/BSD/ISC/zlib 等宽松许可"），
且与本项目自身许可相同。

E4 的原文写的就是「**建议批准**」，并列出了退路（先只支持 JSON preset）。
所以阻塞的事实是「依赖表里没有它」，而那是**行政手续**，不是政策冲突。
用户已批准。

**教训**：一条"待决策"记久了会看起来像"被否决"。定期回去核一下
它到底是在等什么 —— 这次它只是在等一次确认。

---

## 2 交付内容

- `core_model::preset`（`crates/core-model/src/preset.rs`）
- CLI `preset show|validate|example`
- 新增 `Domain::Preset`（`STK-PRE-xxxx`），单独立域而不是并进 `Input`：
  preset 的故障（读不开、schema 太新、字段非法）与"任务自身的输入不对"是两件事，
  混在一起用户分不清"我的字幕有问题"还是"我的配置有问题"。

实测（真 CLI）：

```
$ subtitletool preset example > spec.toml
$ subtitletool preset validate spec.toml
ok  preset
{ "outputs": 2, "problems": [] }
corr-id: J-ffde595d-35c059c55166        exit=0
```

### 2.1 依赖门禁的联动（这是好事，不是障碍）

加了依赖之后，SBOM 门禁**当场报出**：

```
SBOM 过期：有 537 个组件，当前依赖树应该是 542 个，重跑 python tools/notices.py
NOTICES FAIL
```

重跑后：`506 个第三方包 + 2 个源码归档 + 27 条 native/二进制哈希锁，0 个问题`。
SBOM 里 `toml 0.9.12+spec-1.1.0` 的许可是 `MIT` / `Apache-2.0`。

**这次联动证明了那道门禁真的在管事**：它不信任"我记得许可证没问题"，
而是逐个核依赖树。

---

## 3 设计上真正要说的三件事

### 3.1 "可搬运"决定了这个模块的大部分设计

**一条 preset 会离开它被写下的那台机器。** 这句话推出一串约束：

| 约束 | 理由 |
|---|---|
| 不存绝对 DLL 路径与哈希（§4） | 把 `D:\某人的机器\x64\libass.dll` 写进去，收件人一定用不了。存 provider **偏好**（`libass`/`csri`/`auto`），实际路径交给 JobSnapshot |
| 未知字段保留（§3） | 旧版读新版写的 preset 再写回，不能把读不懂的字段吃掉 —— 那会让"用旧版打开一次"毁掉新版配置 |
| schema 高于自己时拒绝执行（§6 条 1） | 不猜含义 |
| 每个字段是 `Option` | §6 条 2 要按"默认 → preset → 用户显式值"合成；解析时就填默认值会让调用方**看不出**哪些是 preset 说了话 |

最后一条与 ToolDefaults 接进 CLI 时同一个道理：**解析阶段只记"用户给了什么"**。

### 3.2 与 batch spec 的**相反**策略，而两者都对

| | batch spec | preset |
|---|---|---|
| 拼错字段 | `deny_unknown_fields`，**必须拒** | **保留** |
| 理由 | 猜错会产出**错误的成品** | 陌生字段只说明它来自别的版本；猜错只丢字段 |

这不是不一致，而是**失败代价不同**。两条规则各自是对的那一侧。

### 3.3 三个动作各自回答一个问题

| 动作 | 回答 | 调用者 |
|---|---|---|
| `show` | 里面是什么？ | 想知道这份配置会产出什么的人 |
| `validate` | 能不能用？ | CI / 提交前检查（只要退出码） |
| `example` | 我该照什么写？ | 第一次写 preset 的人 |

合成一个 `show` 会让 CI 被迫解析人的输出 —— 一条又脆又没必要的关系。

注意 `show` **只要文件能读就退 0**，哪怕它有 problems ——
用户就是想看看里面有什么、哪里有毛病。`validate` 有 problems 才退非零。
两者用同一个解析器，但**判断"是否成功"的标准不同**，因为用途不同。

---

## 4 实测抳出两个真问题

### 4.1 `hdr.mode` 不是扁平键

第一版把 `HdrTarget` 写成 `#[serde(flatten)]`，指望它接住 `hdr.mode`。
实测报错：

```
preset 字段不认识或不合法：unknown field `hdr`  in `outputs`
```

**原因**：TOML 里 `hdr.mode = "off"` 的**点号不是键名的一部分**，
它是**嵌套表的语法糖**，等价于：

```toml
[outputs.hdr]
mode = "off"
```

于是 `hdr` 是一个**嵌套结构**，而 `flatten` 收的是表里剩余的**键** ——
它看不到嵌套结构。

另外：serde 的 `deny_unknown_fields` 与 `flatten` **本来就不能共存**
（flatten 会接住所有剩余字段，两者语义互斥）。

修法：按它本来的形状写成嵌套表 `pub hdr: Option<HdrTarget>`。两边都对，
而且未来加 `hdr.transfer` 之类的兄弟字段不需要改结构。

### 4.2 `preset example` 存出来的文件自己读不了

第一版除了样板还打了一份 JSON 信封。实测：

```
$ subtitletool preset example > spec.toml
$ subtitletool preset validate spec.toml
STK-PRE-0002: preset 内容不合法
  - spec.toml：不是合法的 TOML：TOML parse error at line 20, column 5
  20 | ok  preset
     |     ^
key with no value, expected `=`
```

存出来的文件**前半是 TOML、后半是 JSON**。

而 `preset example` 的**全部用途**就是"把样板原样拿走存成文件"
（`subtitletool preset example > my.toml`）。一个"给你样板"的命令
存出来自己读不了。

修法：给 `Envelope` 加一个**显式** `raw_stdout` 标记，
`emit` 见到它就不再写信封。

**为什么不用"在 `emit` 里按命令名特判"**：特判会散在 `emit` 里，
而看到 `emit` 的人不知道哪几个命令是特例 —— 那正是"隐式行为"
最典型的坏处。一个字段名把这件事说清楚了。

---

## 5 仍未做（如实记）

### 5.1 `batch --preset` 的直接接入

36 §6 的合成顺序是"默认 → preset → 用户显式值"。
而 `batch` 的 JobSnapshot 是**已冻结**的一批任务。把 preset 塞进去只有两条路：

- 改写已冻结的字段 → 破坏"可重放"（而那是 JobSnapshot 存在的全部理由）；
- 什么都不做，只报告"我读过了" → **等于没接**。

所以本模块把 preset **解析成一组 `OutputTarget`**，由调用方决定怎么用。
接进 `batch` 需要先解决"冻结与覆盖"的语义问题，而那是一个**设计问题
不是实现问题**。不假装做了。

### 5.2 第五层 Plan

Planner 的 stage DAG 未实现。它与 preset 无关，是另一条线。

### 5.3 其他

- §5 的"未知字段保留"在 `batch` spec 上是**反向**的（见 3.2）——
  两份配置的规则不同是**有意的**，不是疏忽。
- preset 目前不参与任何执行路径。用户能读、能校验、能拿到样板，
  但"用这份 preset 跑一次"还没有入口。这一点在 `show` 的输出里
  没有暗示（它只说"里面是什么"），在规格里写明了。
