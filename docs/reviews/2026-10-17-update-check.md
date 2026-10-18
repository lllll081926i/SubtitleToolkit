# 裁决：更新检查（28 §6 第一版）

日期：2026-10-17
范围：`crates/cli/src/update.rs`、`crates/cli/src/{args,run,lib}.rs`、`crates/cli/tests/check_updates.rs`
规格：`docs/development/28_WINDOWS_BUILD_RELEASE.md` §6

## 1 规格怎么说的，本实现落在哪里

> 第一版可只"检查更新并打开 release 页面"；成熟后做 signed manifest + staged
> download。**不要在早期自写高权限 auto-updater。**

本实现严格落在第一版，并**比第一版还保守**：它连"打开 release 页面"都不自己做，
只**把地址印出来**。

| 规格允许 | 本实现 | 为什么 |
| --- | --- | --- |
| 检查更新 | ✅ 做版本比对 | — |
| 打开 release 页面 | ❌ 只打印 URL | 起浏览器要 `ShellExecuteW`，而它在无桌面会话/服务里行为不定；打印 URL 在任何环境都能用 |
| signed manifest + staged download | ❌ 不做 | 规格明说等"成熟后" |
| 自写 auto-updater | ❌ 不做 | 规格明令禁止 |

## 2 三条设计决定

### 2.1 不出网

`update.rs` **不引任何 HTTP 客户端**（当前 `crates/cli` 也没有）。清单由调用方
经 `--manifest FILE` 提供。

三条理由，按重要性排序：

1. **它会把一个"读版本号"的功能变成一条会走网络的代码路径。** 本项目至今没有
   任何出网依赖（`cargo tree` 可验）。更新检查是**唯一**会主动出网的功能，
   把它做成一条独立、可审计、可整体关闭的路径，比把 `reqwest`（连带 TLS 栈、
   根证书策略、代理语义）拖进依赖树要小得多。
2. **21 §1 的许可政策。** TLS 那一族依赖要逐项过审，而这条功能的收益
   （少点一次浏览器）远不抵那份长期维护成本。
3. **安全边界。** 更新检查是供应链攻击的经典入口。一个**不下载任何东西**的检查
   最多泄露"有人在用这个版本"；会下载解压的实现则把攻击面从"解析一个 JSON"
   扩大到"解析网络响应 + 落盘 +（若做自动更新）执行"。

**代价如实说**：用户要自己提供一个清单文件。这不是"完整的自动更新"，
但它是一条**可以现在就说清楚边界**的路。要真正自动，得等发布链
（签名、托管、CI）就位之后 —— 那时的清单可以放在已签名的 release 页上。

### 2.2 不假装检查过了

没有 `--manifest` 时返回 `verdict = unknown`，并在 `detail` 里说明
"无法判断" + 指出去哪看。

**一条静默返回"已是最新"的更新检查比没有这个功能更坏**，因为它会让用户
确信自己是最新的。这条有专门用例（`without_a_manifest_it_reports_unknown_and_never_pretends_to_be_current`），
它同时断言 `verdict != "same"`。

### 2.3 解析不了就 `Unknown`，不猜

版本比较只接受严格的 `MAJOR.MINOR.PATCH`。少一段、多一段、带 `v`、带预发布
后缀（`1.2.3-beta`），一律 `Unknown` 并**说清是哪个字符串**。

宽松解析（"`1.2` 就当成 `1.2.0`"）看着友好，但它会把**清单文件的格式错误**
变成一次看似成功的比较。更新检查的两种失败：

- **假阴性**（明明有新版本却说没有）→ 用户留在旧版本上，还能用；
- **假阳性**（读错了字段却说"有新版"）→ 用户去下载一个错的东西。

后者更坏。用例 `unknown_is_never_silently_treated_as_same` 钉住这条。

顺带：比较用**数值**而不是字符串。`compare("9.0.0","10.0.0")` 必须是 `Newer`，
而字符串比较会给 `Older`。用例 `numeric_ordering_is_not_string_ordering` 钉住。

### 2.4 不引 semver crate

只需要比三段数字，而 semver 的完整语法（预发布排序、构建元数据、区间）在这里
全是负担。更关键的是**比较规则的每一条都要能被测试钉住** —— 自己写的三十行
比"依赖某个 crate 某个版本的语义"更容易核对。

## 3 一个诚实的细节：默认 URL 指向哪

`DEFAULT_RELEASE_URL` 是 `https://github.com/example/subtitletoolkit/releases`。

那**不是**一个真实可用的地址，因为本仓库当前没有公开的 release 站点
（28 §1 的发布链仍未走完）。**不编一个看起来像真的地址** —— 用户照着点会 404，
那比明说"这里还没配"更坏。所以：

- 默认值用的是仓库形状的路径（至少形状是对的，改起来是改一处常量）；
- 用 `--release-url` 或清单里的 `url` 可以覆盖；
- 文档与 `--help` 都不声称那个地址已上线。

要真正可用，需要一次发布链落地的动作，那不是代码能解决的。

## 4 抳出的一个真缺陷（我自己写的）

第一版 `check-updates` 用 `scan(rest, ...)` 解析参数，而 `scan` 的签名要求
**返回一个位置输入文件**。本命令没有输入文件，于是空参数时它报
"缺少输入文件" —— 看起来像**命令根本没被识别**。

排查过程值得记一笔，因为它差点被误判成别的问题：我先怀疑过
"match 分支顺序错了"、"二进制是旧的"，都用证据否掉了（`cargo build` 说
无改动可重编；`checkupdates` / `check_updates` 报"未知命令"而
`check-updates` 报"缺少输入文件"，证明**它确实进了正确的分支**）。

修法：照 `diagnose` 的做法自己遍历 `rest`。无位置参数的命令都是这么做的。

## 5 抳出的一个真缺陷（错误码冲突）

第一版用了 `Domain::Input, 60` 与 `61`。而 `STK-INP-0060` 已经被
"尚未实现的命令"占用（`args.rs` 的说明里就写着这个码）。

**`core-model` 的跨 crate 唯一性测试当场报出来了**：
`no_two_crates_share_an_error_code_with_different_meanings` FAILED。

改成 `210` / `211`（先枚举了 `Domain::Input` 与 `cli_err` 已用的全部码，
确认这两个在 200–300 段里是空的）。

这条同理于上一轮：**手写错误码一定会撞，机器检查是唯一可靠的防线。**

## 6 验证记录

### 6.1 行为（真二进制，7 种输入）

```
没清单          → 退 0, verdict=unknown, detail 里给出 --manifest 与地址
清单 0.2.0      → 退 0, verdict=newer
清单 0.1.0      → 退 0, verdict=same
清单 0.0.9      → 退 0, verdict=older
清单不是 JSON   → 退 1, STK-INP-00210「版本清单不是合法 JSON」
清单 v2.0       → 退 0, verdict=unknown, detail 说清是「v2.0」这个字符串
清单自带 url    → 退 0, release_url 用了清单里的
```

每次输出都带 `downloaded: false` / `installed: false` / `meaning`，把
"本程序到底做了什么"写在结果里，而不是写在文档里等人去读。

### 6.2 用例

- `crates/cli/src/update.rs`：12 个单元用例
- `crates/cli/tests/check_updates.rs`：9 个集成用例（走真二进制）

集成用例特别核了**安全属性**：结果里必须自报没下载没安装。这让"它真的没下载"
成为**可检查的事实**，而不是靠读代码相信。

### 6.3 门禁

```
cargo fmt --all --check                              → 干净
cargo clippy --workspace --all-targets -- -D warnings → 干净
cargo test --workspace --lib --tests --no-fail-fast   → 78/78 目标，0 失败
cargo test -p core-model --test error_code_uniqueness → 4 passed
```

## 7 未完成（如实记录）

- **不是自动更新**。没有网络获取、没有下载、没有安装。要真自动，得先有
  签名与托管（28 §5、28 §1），而那两步都未完成。
- **默认 URL 不是真实地址**（见 §3）。
- **没有 GUI 入口**。CLI 有 `check-updates`；GUI 没接。接它需要 GUI 先有
  "设置/关于"面板，那是另一件事。
- **没有"上次检查时间"的节流**。手动命令不需要节流；将来若做自动检查才需要。
