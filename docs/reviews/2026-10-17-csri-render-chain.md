# 裁决：CSRI 渲染链（12 §1B）

日期：2026-10-17
范围：新增 `crates/render-csri`；`crates/core-model/src/dllload.rs`
规格：`docs/specs/12_RENDERER_FILTERS_WINDOWS.md` §1B、§1、§3
前置：[provider 能力探测](2026-10-17-provider-capability-probe.md)

## 1 交付了什么

一个**真的能渲染**的 CSRI 链。实测（本机真实 `third_party/VSFilterMod.dll`）：

```
provider 身份：name="vsfiltermod_textsub" longname="2.39"
               specific="VSFilterMod/TextSub (guliverkli2)"
640x480 @2.0s：1882 个白像素（字幕）+ 303907 个黑像素（底）
0-4.9s 有字幕，5.0s 起 0 个  ← 时刻真的传进去了
libass-9.dll 被正确拒绝：STK-RND-0096「不是 CSRI provider」
```

模块划分：

| 文件 | 职责 |
| --- | --- |
| `ffi.rs` | CSRI C ABI。结构体偏移用 `offset_of!` **编译期**核对 |
| `frame.rs` | `BGRA`/`BGR_`/`RGB` → RGBA8。**唯一**转换点 |
| `provider.rs` | 加载用户提供的 provider 并跑起来 |
| `codes.rs` | 错误码集中定义（见 §4.3） |
| `tests/real_provider.rs` | 拿真 DLL 走完整链路 |

## 2 许可边界（设计前提，不是脚注）

21 §6 明说 VSFilterMod 的默认设计是"**用户提供/可选外部 renderer provider，
通过 renderer-host/CSRI 调用**"；21 §1 又明说"**进程边界不自动解决分发
许可义务**"。两条合起来：

- **写 loader 是允许的** —— 本 crate 不含任何 GPL 代码，与现有 libass 路径同构
  （运行时 `LoadLibraryW`，DLL 由用户提供）。
- **把 GPL 的 DLL 随包发是另一件事** —— 本次不做，也不该由代码做。
- **"进程隔离"不等于"许可干净"** —— 这条已写进 `lib.rs` 模块头部，
  免得被误读成免责。

## 3 与 libass 那条链的关系

**共享**加载器：`core_model::dllload::LibHandle`（本轮抽出来的）。
共享它是有意的 —— 加载策略里每条都踩过坑（不搜当前目录、必须完整路径、
正斜杠要规范化），而**两份实现对同一个 DLL 会给出不同答案**：
一边放行一边拒绝，用户看到的是一条自相矛盾的报错。

**不共享**装订代码：CSRI 与 libass 的 ABI 完全不同，
抽一个"通用 provider"会把两套不兼容的语义塞进一个抽象里。

## 4 抳出的四个真问题

每一个都会导致**静默错结果**（不崩、不报、图出得来但不对）。

### 4.1 `PixFmt::BgrUnderscore` 的数值写错了

我写成 `0x101`。头文件里那一组是：

```c
CSRI_F_RGB_ = 0x100,
CSRI_F__RGB,      // 0x101
CSRI_F_BGR_,      // 0x102  ← Windows "RGB32"
CSRI_F__BGR,      // 0x103
```

**从锚点逐项递增，中间几个没有显式写值。** 我把"相邻"当成了"同一个数"。

后果：把 `0x101` 交给 provider，它会照那个数字解释内存 ——
图出得来，只是通道或步长不对。

**抳出它的方式**：写了一个逐个试"真 provider 接受哪种格式"的探针，
结果是 0x102 → 0（接受）、0x101 → -1（拒绝）。单靠构造数据的用例看不见这个，
因为它们按我写错的（错的）枚举造数据，自然对得上。

> 教训与 `CSRI_REQUIRED_EXPORTS` 那次同源：**枚举数值不能从拼出来的
> 文本里抄**，要读头文件的完整原文，或者拿真 provider 试。

### 4.2 stride 的合法性检查比错了对象

第一版写的是 `|stride * height|` 与行宽比较，而正确的比较是
**`|stride|` 与行宽**。`height > 1` 时两者不等价：

- `stride=4`、`height=2`、行宽 8 → `4*2+4 = 12 >= 8`，于是**放行**；
- 然后按 8 字节读一个只有 4 字节步长的缓冲 → **越界读**。

是我自己的用例抳出来的（那条用例断言"stride 装不下一行必须被拒"，
第一版返回了 `Ok`）。修完补了一条配对用例：**恰好等于行宽的 stride 必须被接受** ——
只断言"太小的被拒"会漏掉"把合法的也拒了"。

### 4.3 两处错误码一号两义

跨 crate 唯一性测试当场报出：

- `Render/80` 同时表示"只支持文件开脚本却没给路径"与"路径不是合法 UTF-8"；
- `Render/83` 同时表示"只有 stream 扩展"与"两条路都没有"。

那两种情形**用户要做的事不同**，共用一个码会让"看错误码判因"失效。
新增 `codes.rs` 集中定义，新码 92–97。

顺带还报出一次跨 crate 冲突：我给 `dllload` 用了 `Render/84`，
而那是 `render-libass` 的"libass 给出负 stride"。改成 97。

### 4.4 CSRI provider 不是线程安全的

**并行跑用例时同一份输入会得到不同像素数**（有时 1882、有时 0），
串行跑永远 1882。

原因：`csri_renderer_default()` 返回进程级单例，多个实例同时
`csri_render` 会互相干扰。这不是我们的代码错，也不是能在库内用锁修好的
—— provider 自己没承诺线程安全。

处理：用例加进程内锁串行化；**而真正接入 `renderer-host` 时，
一个宿主进程同时只跑一个 provider 实例** —— `renderer-host` 本来就是
"一个宿主进程一次渲染"的模型，所以那里不构成限制。

把这一条写下来的理由：不写的话，下一个看到"并行跑偶尔失败"的人会去查
我们的像素转换代码 —— **而那里是对的**。

## 5 另一个差点被"绿"骗过的地方

真 provider 用例的第一版**全部静默跳过**，而 `test result: ok`。

原因：我用"从当前 exe 往上退 4 层"找仓库根，而本仓的 `CARGO_TARGET_DIR`
在 Cargo 目标目录，从 `<target>/debug/deps` 往上退
永远退不到仓库。找不到文件 → `return` → 用例"通过"。

**抳出它的方式**：跑 `--nocapture`，看到满屏的"跳过"。
改成用 `CARGO_MANIFEST_DIR`（仓库既有做法），并把"缺素材"从跳过
改成**失败** —— 静默跳过会让"没测"看起来像"测过且对"。

顺带修正另一处同类问题：第一版用"非透明像素数"判断"字幕画出来了"，
而 `BGR_` 的第 4 字节是填充、我们统一写 255，于是**整帧都是 alpha=255**，
与字幕画没画无关（实测两帧都是 76800/76800）。改成核**颜色**
（白像素 = 字幕、黑像素 = 底）。

## 6 本次做到了哪一步（后续更新）

### 6.1 CSRI 链**已接进 `renderer-host`**（同日）

新增 `crates/renderer-host/src/backend.rs`：按导出表分派到两条链之一。
实测（真 VSFilterMod.dll，经完整 CLI 管道）：

```
命令：subtitletool-cli render x.ass --libass-dll VSFilterMod.dll
结果：ok=true, events=1, samples=50, rendered_frames=1, pngs=1
     两条链产出的 XML 都是 516 字节
     （PNG 字节数不同：两个不同渲染器栅格化必然不同，那不是错误）
```

接的过程中抳出并修掉五个真问题：

1. **CSRI 给的是“带不透明背景的合成位图”，与 libass 的语义不同。**
   实测：CSRI 对同一输入产 3 帧（10493/10730/10493），libass 只产 1 帧。
   下游 `image_core::Frame::bbox` 是**按 alpha 判定可见内容**的，
   于是不透明背景被看成“整幅画布都有内容”。`front.rs` 末尾补的空白采样
   （`Frame::empty`，全透明）与 CSRI 的“全不透明黑”不相等，
   产出长度为零的事件 → `STK-BDN-0059 事件结束不晚于开始`。
   修法：在 backend 侧把背景剥成透明（判据“与角点同色且不透明”，
   敲掉多少记账）。**判据不完美，所以它只做一件事且可观察**。
2. **客户端握手校验写死了 `provider == "libass"`** —— 把诚实的 `csri` 拒了。
   改成“认识的名字就放行”，真正要拒的是“我不认识的 provider”。
3. **`render_libass::Script<'a>` 借用 `Provider`**，与“可变借用 backend”冲突。
   `Symbols` 加 `Copy`，`Script` 拷一份函数表而不再借用。
4. **三处错误码跨 crate 冲突**（唯一性测试报出）：改为 98/99/100。
5. **握手的 `version` 成了空串** —— 两条链都要真打开才知道版本，
   而握手发生在脚本到达之前。**没有去伪造**，而是把契约改清楚。

### 6.2 AT-RENDER-002：做到了哪一步，没做到哪一步

注入点已移到**真的要调 provider 的那一刻**（`render-abort` / `render-hang`），
因为原来那几个都在 `serve()` 最早期跑，那时连 provider 都没打开 ——
它们验的是“host 启动阶段死了会怎样”，不是规格要的那件事。

实测（真 CSRI provider）：

```
主进程活着：Crashed { exit_code: 3221226505 (0xC0000409) }
崩溃现场：阶段=render；序号=1；已运行=37ms；provider=x64；
         provider sha256=87a9c3c1ab76…；原因=出帧中途 abort
重试成功：4233 个不透明像素
挂住时超时：Timeout { waited_ms: 2000 }
现场落盘：crash_dir/host-crash.json
```

**仍不验收。** 规格要的是让**那个第三方 DLL 自己**崩。
那需要它的代码里有一个可触发的缺陷或我们能控制的崩溃入口（两者都没有），
而“喂坏数据碰运气”不是可复现的测试。
本轮的注入复现的是**同一个时序**，而隔离机制要对付的正是那个时序 ——
**但这不是规格要求的那条证据**。

### 6.3 其他仍未做

- `VSFilterMod.dll` 不进任何发行物（21 §1，GPL-3.0）。
- VapourSynth / AviSynth 两条 legacy 链未做（12 §1C）。
- **CSRI 链不支持内存字体注入**（头文件没那个入口）。
  `backend` 暴露 `supports_font_injection()`，host 不支持时打明确警告 ——
  静默不注入会让用户以为字形来自他给的那份字体。
- `render-libass` 里还留着自己那份 `LibHandle`。

## 7 验证记录

```
cargo test -p render-csri
    lib 30 passed / real_provider 7 passed

cargo fmt --all --check                              → 干净
cargo clippy --workspace --all-targets -- -D warnings → 干净
cargo test --workspace --lib --tests --no-fail-fast   → 83/83 目标，0 失败
cargo test -p core-model --test error_code_uniqueness → 4 passed
```

真 provider 用例覆盖：探测识别、开脚本、设格式、渲染出像素、
**时刻敏感性**、同刻两次逐字节相同、非 CSRI 被拒、不存在的文件被拒、
非 UTF-8 路径被拒。
