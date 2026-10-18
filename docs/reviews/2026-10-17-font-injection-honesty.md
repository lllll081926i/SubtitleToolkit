# 裁决：字体注入能力报告在 CSRI 链上说了假话

- 日期：2026-10-17
- 相关规格：17 §7（内存字体注入）、12 §1B（CSRI 链）、22 §（不静默少做一件事）
- 相关提交：`a742bec`
- 相关用例：`crates/renderer-host/src/backend.rs` 的
  `the_font_injection_capability_agrees_with_what_add_font_reports`；
  `crates/cli/src/rendercache.rs` 的格式步进用例

---

## 1 现象

一份带 1.0 MB 内嵌字体的 ASS 走 CSRI 链渲染。命令成功（退 0），
报告里写：

```json
"font_injection": {
  "enabled": true,
  "requested": 1,
  "embedded": 1,
  "injected": [{ "name": "Arial", "bytes": 1047208, "sha256": "74d696…" }]
}
```

而 provider **根本没见过那字节** —— CSRI 头文件里没有加字体的入口
（12 §1B 早已写明这一点）。

用户据此会以为字形来自他给的那份字体，实际用的是系统字体。
**这正是那条能力检查存在的全部理由**，而它被自己的调用方绕过去了。

---

## 2 根因：注释拦不住人

`Backend::add_font` 的第一版对 CSRI 返回 `Ok(())`，注释写得很清楚：

> `Ok(())` **不表示成功**，调用方必须先看 [`Self::supports_font_injection`]。

而 `main.rs` 里恰好就在同一段代码里写：

```rust
Ok(()) => injected += 1,
```

**"调用方要先看能力"是一条靠不住的契约** —— 它只写在注释里。
注释不参与编译，也不会在改动时报错。

### 修法

让**返回值自己承载真相**：

```rust
pub fn add_font(&mut self, family: &str, bytes: &[u8]) -> Result<bool>
//  true  = provider 真的拿到了这份字节
//  false = 没做
```

于是忘了查能力也不会得到一个假数字。

**仍然不做成 `Err`**：能力缺失不是失败。做成错误会让
"某份 ASS 带内嵌字体"在 CSRI 链上整体渲染不了，而那比"没注入"坏得多
—— 用户还能拿到字幕，只是字形未必是他要的那份。

---

## 3 三处改动

### 3.1 `Backend::add_font` → `Result<bool>`

见上。

### 3.2 host 在**收到字体时**就分类并回一个诚实的数

为什么要在那时算，而不是等 provider 打开之后：

**请求—应答是同步的。** 控制面发一个 `AddFont` 就等一个回复。
在中间插一条它没在等的消息，会让它把那条当成**下一个请求的回复** ——
那是一个很难查的错位。

而那个循环里已经有 `dll_path`，分类（只读 PE 导出表、**不加载**）是现成的：

```rust
let chain_takes_fonts = !matches!(
    renderer_host::backend::classify(&dll_path),
    Ok(render_api::capability::ProviderKind::Csri)
);
let acked = if chain_takes_fonts { bytes } else { 0 };
```

跳过时回 `0`。

### 3.3 CLI 把"回 0"当成能力缺失，而不是传输错误

原来这两件事合在一起：只要 `count != bytes.len()` 就报
`STK-RND-0090` 并**中止整个渲染**。

于是带内嵌字体的 ASS 在 CSRI 链上**根本渲染不了** ——
而真正该报的"沉默"反而没人报。

改判之后：

```rust
if count == 0 {
    loss.user_attention(&format!(
        "这条渲染链不支持内存字体注入：{}（{} 字节）被跳过，\
         字形将来自系统字体而非你给的那份（17 §7）", …));
    skipped_fonts += 1;
    continue;
}
```

并新增一个单独的字段 `font_injection.skipped_by_chain`。

**为什么单独立字段而不是只靠 loss**：`requested` 减 `injected.len()`
虽然能算出来，但要求用户知道该拿哪两个数相减。而这一项的意义是
"你看到的字形不是你给的那份字体" —— 那件事必须一眼看得到。

---

## 4 实测（真 DLL，两条链对照）

同一份 ASS（2 个 face：库里的 Arial 18.7 KB + 内嵌的 1.0 MB）：

| 链 | `requested` | `injected` | `skipped_by_chain` |
|---|---|---|---|
| libass | 2 | 2 份（18748 + 1047208 字节） | — |
| CSRI | 2 | `[]` | **2** |

CSRI 侧的 `meaning` 现在直接说：

> **部分字体没有交给渲染器**：这条渲染链不支持内存字体注入，
> 被跳过的那几份不会影响字形 —— 字形来自系统字体（17 §7）

libass 侧的 `meaning` 不变（"这些字节是本次直接交给 libass 的内存字体"）。

---

## 5 顺带修的两处

### 5.1 渲染缓存要存 `skipped_fonts`

它是报告里的一个字段，而报告**不能随缓存冷暖而变**。
不存的话，同一次执行在冷缓存下报 `skipped_by_chain: 1`、
热缓存下报 `0` —— 而两者给出**完全不同的结论**。

### 5.2 渲染缓存的手写字段步进测试

那条用例一步步走过二进制布局，去定位 patch 的 rgba 长度字段，
然后故意改坏它、断言解码器拒绝。

新增 8 字节后它读到了 `38654705671`（而不是期望的 16）并失败。

**这不是坏事，正是它存在的意义**：它证明"手写步进错位会被报出来"，
而不是静默去改一个无关的字节。加一行步进，并在注释里写明
"新增字段必须同步"。

---

## 6 排查过程中的一段弯路（如实记）

排查时我用**复用的输出目录**反复跑，看到：

```
STK-INP-0173: BDN 目录校验未通过
  - 核对 1 个 PNG，1 个问题（首个：events[0] 0000.png → STK-BDN-0073）
```

`STK-BDN-0073` = XML 声明的尺寸 ≠ 实际 PNG 尺寸。我一度以为是真 bug
（CSRI 与 libass 表现不同），去查了 PNG 的 IHDR 与 XML 的 `<Width>`。
XML 声明 `356x45`，而目录里那个 `0000.png` 是 `1920x1080`。

核实后是**我自己的测试残留**：`validate_dir` 核对的是**整个目录**，
而前一次实验留下的 `0000.png` 还在那儿。换到干净目录后两条链都正常。

**产品行为是对的** —— 正因为 `validate_dir` 核对整个目录，
才抓到了"上一次的产物还躺在输出位置"这个状态。
错的是我的复现方式。

教训：**排查产品问题时，先把测试环境清干净**。
否则会把"自己的残留"当成"产品的缺陷"，然后去改一个没坏的东西。

---

## 7 仍未做

- **`font_injection` 目前只在单命令 `render` 与 `pgs encode` 的报告里。**
  `batch` 路径的每条任务都会经过同一个 `front::render_to_plan`，
  所以行为一致，但没有单独的用例锁住 batch 侧的字段。
- **CSRI 链上"内嵌字体被跳过"没有给用户一条可行的替代路径。**
  现在只说"字形来自系统字体"。可行替代是把字体装进系统
  （用户自己去做），或者改用 libass 链 —— 后者是可执行的建议，
  但**没有写进 `user_action`**。
