# 待办：CSRI 渲染链（12 §1B）—— 设计与边界

日期：2026-10-17
状态：**未实现**。本文只是把"要做什么、卡在哪、为什么这样切"写清楚，
避免下次接手时重新推一遍。
前置：[provider 能力探测](2026-10-17-provider-capability-probe.md)（已完成）。

## 1 为什么它现在是 AT-RENDER-002 的唯一前置

上一轮我把 AT-RENDER-002 的原因改成"缺 CSRI 渲染链"，本轮把这句话核实到底：

| 项 | 状态 |
| --- | --- |
| 真实 provider 素材 | **有**（`third_party/VSFilterMod.dll`，已在 `SOURCE_AUDIT_LOCK.json` 登记） |
| 来源锁 | **有**（PDB 路径、PE 时间戳与上游 sorayuki/VSFilterMod 相符） |
| 能力探测 | **有**（`provider probe` 实测认出它是 CSRI provider） |
| **CSRI 渲染链** | **没有** |
| 崩溃隔离证据 | **没有**（要等链建好才能测） |

所以剩下的是一件事：把 CSRI 链写出来。

## 2 许可边界（这是设计前提，不是障碍）

21 §6 的原文：

> GPL-3.0；**默认设计为用户提供/可选外部 renderer provider，通过
> renderer-host/CSRI 调用**。是否随安装包分发要在最终项目许可证确定后由
> 法务/许可证审计决定。

21 §1 补了一句很要紧的：

> GPL 组件不复制进核心；仓库内研究源码和候选二进制单独列明来源、许可证与哈希。
> **进程边界不自动解决分发许可义务。**

把两条放在一起看，结论是清楚的：

- **写一个 CSRI loader 是允许的**，而且正是规格设计的形态 ——
  它与现有的 libass 路径同构：运行时 `LoadLibraryExW`，用户自己提供 DLL。
- **把 GPL 的 DLL 打进安装包是另一件事**，需要法务/许可审计，
  本次**不做**，也不该由代码做。
- **"进程隔离"不等于"许可干净"**。所以即便走了 renderer-host，
  分发决策依然独立 —— 这一点必须写进用户可见的文档，
  不能让"我们在另一个进程里跑它"被误读成许可上的免责。

一句话：**loader 可以写，DLL 不能随包发。** 用户要自己放一份。

## 3 要写的东西（按依赖顺序）

### 3.1 `render-csri` crate：CSRI C ABI

从上游 `csri.h`（本轮已逐名核实）取，**不手抄结构体偏移**：

```c
struct csri_fmt   { enum csri_pixfmt pixfmt; int width, height; };
struct csri_frame { enum csri_pixfmt pixfmt; unsigned char *planes[3]; ptrdiff_t strides[3]; };
typedef void csri_rend;  typedef void csri_inst;
```

必需函数（依 [`CSRI_REQUIRED_EXPORTS`](../../crates/render-api/src/capability.rs)）：

```c
csri_rend csri_renderer_default(void);
csri_inst csri_open_file(csri_rend, const char *f, struct csri_openflag *);
csri_inst csri_open_mem(csri_rend, const void *d, size_t n, struct csri_openflag *);
void      csri_render(csri_inst, struct csri_frame *, double time);
void      csri_close(csri_inst);
```

可选（`CSRI_OPTIONAL_EXPORTS`）：`csri_query_ext`（**`csri_stream_ext` 的入口**）、
`csri_request_fmt`、`csri_renderer_info`/`_next`/`_byname`/`_byext`。

**硬约束（沿用本仓已经吃过的教训）**：
- 结构体偏移**必须用 `offset_of!` 做编译期断言**。注释不起作用 ——
  17 §2 的 GDI 那次就是 `WCHAR` 写成 `u8` 导致偏移差 4 字节、
  而纯构造数据的测试完全看不见。
- 每个符号取不到时的行为要分开：**必需缺 = 不是 provider**（可继续找下一个），
  **可选缺 = 降级**（例如没有 `csri_query_ext` 就只能走 `csri_request_fmt` 旧路径）。

### 3.2 像素格式

CSRI 的 `csri_pixfmt` 里我们要的是 **`CSRI_F_BGRA`**（12 §1B 点名）
或退回 `CSRI_F_BGR_`。而 libass 那条链给的是 **RGBA8**。

两条链路都要汇到 `image-core` 的同一个 `Patch`，所以需要一个
**明确的转换点**，而不是"两边都叫 32 位色所以直接拷"。BGR_ 还要补 alpha=255。

### 3.3 接进 renderer-host

现在 `renderer-host/src/main.rs` **硬编码** `render_libass::Provider`。
要改成按 12 §1 的结果分派：

```
provider probe → kind = Csri   → CSRI 链
              → 像 libass     → 现有链
              → 都不匹配      → 明确拒绝（不要"试试看"）
```

**不允许"先试 libass，失败再试 CSRI"**：那会让一次失败的加载在进程里
留下半初始化状态，而这正是 renderer-host 要用进程隔离避免的东西。

### 3.4 AT-RENDER-002 的隔离证据

链建好之后才谈得上它。规格要的是：

> 证明其崩溃只终止 renderer-host，GUI 可重试。

所以要**一次真的注入崩溃**（不是 `Poison` 自伤 —— 规格明说
"自伤 `Poison` 测试不能代替该 provider 测试"），并核对：
host 死 → 主进程存活 → 错误卡带上崩溃现场 → 重试能成。

## 4 本次**不**做的事（写清楚，免得被当成已做）

- 不写 loader（本文只是设计）。
- 不把 `VSFilterMod.dll` 打进任何发行物（21 §1）。
- 不为 CSRI 链做"性能优化"——先让它对，再谈快。
- 不做 VapourSynth / AviSynth 两条 legacy 链（12 §1C）。它们与 CSRI 独立，
  且同样各自需要一套宿主接口。

## 5 用户可见的说法（写进 tools/README 与错误文案）

必须让用户**一眼看出三件事**：

1. CSRI provider **要你自己提供** —— 我们不随包发（21 §6，GPL-3.0）。
2. 我们会**先探测再加载**：`provider probe <dll>` 可以单独问它是什么。
3. 它在**另一个进程**里跑 —— 但那是**故障隔离**，不是许可免责。

第 3 条尤其要紧：把"进程隔离"误讲成"许可上没问题"，是对用户的误导。
