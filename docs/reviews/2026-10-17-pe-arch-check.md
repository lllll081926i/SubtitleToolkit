# 裁决记录：23 §DLL 的 PE arch 检测（2026-10-17）

## 需求

23 §DLL 原文要求「第三方 provider 进 renderer-host；**PE arch 检测**；可选 hash allowlist；
不在当前目录任意 DLL 搜索，防 DLL hijacking」，12 §3 另有「不得跨位数 `LoadLibrary`」。

## 交付前的真实状态（这是本次改动的全部理由）

- `renderer-host` 里**已经有一份**能用的 `pe.rs`（读 PE 头判机器类型），
  host 侧在加载前确实做了位数比对 —— 那一半是好的。
- 但 **CLI 侧完全没有这道检查**：`libass_dll()` 只判"是不是文件"就放行。
  于是主程序会把一个位数不符的 DLL 交给 host，由 host 报位数错误 ——
  用户在"渲染命令"里看到一条来自 host 的错误，而问题其实在命令行的参数上。
- 更糟的是 `crates/cli/src/batch.rs` 里的 `arch` 是**写死的** `"x86_64"`。
  这个值会进 `JobSnapshot`，而 `JobSnapshot` 的全部用途是回答
  「当时到底用了哪个二进制」（20 §Repro）。往里写一个不读文件的常量，
  就是让那句话变成谎话 —— 换 32 位库时它会照样写 `x86_64`。

## 交付

1. **PE 读取移到 `core-model::pe`**（`git mv`，不是复制）。
   主程序与 host 都要这个判断，**两份实现对同一个 DLL 会给出不同答案** ——
   那意味着一边放行一边拒绝，用户看到一条自相矛盾的报错。
   `renderer-host` 改成 `pub use core_model::pe;`，既有调用点一字未改。
2. CLI 的 `libass_dll()` 加位数校验，并把失败分成三个可分的码。
3. `batch.rs` 的 `arch` 改成从 PE 头读；读不到时如实写 `unknown`，不编值。

## 决定与理由

### D-PE-01：一份实现，放在两边都依赖的层

这不是"顺手复用"，而是唯一正确的选择：判断"这份二进制能不能被本进程加载"
必须只有一个答案。richer 的做法（host 里一份、CLI 里一份、各带自己的机器类型表）
会在下一次新增架构时分成两种行为。

### D-PE-02：三种失败必须可分

| 情形 | 码 | 用户要做的事 |
|---|---|---|
| 路径不存在 | `STK-INP-0141` | 改路径 |
| 不是有效 PE | `STK-INP-0142` | 换一个真的 DLL（拿错了/文件坏了） |
| 位数不符 | `STK-INP-0143` | 换成同位的那一份 |

后两种的用户动作完全不同。混成一条错误码，就等于把"这个文件坏了"与
"这个文件没错但版本不对"一起丢给用户猜。有一条测试专门断言三个码互不相同 ——
这条断言看着平凡，但它正是"可分"的定义。

### D-PE-03：`arch` 不写死，读不到就写 `unknown`

`unknown` 与 `x64` 在报告里的区别是"没测到"与"测到了且是 x64"。
编一个值会让读报告的人以为测过了。这与 21 §11 / SBOM 里
`absent-here` 和 `match` 分开写是同一条纪律。

### D-PE-04：不认识的机器类型**不算**不匹配

`PeArch::Other(_)` 在 host 侧的比较里保持放行语义。理由是"不认识"不等于"不能加载"：
把将来新增的架构判成"不行"会让它被无理拒绝，而那时用户看到的理由还是错的
（说它位数不符，实际是我们不认识它）。

### D-PE-05：只读头，不引 PE 解析库

需要的只有"是不是 PE、机器类型、32/64 位"，全在文件头部，判据有公开规范
（Microsoft PE/COFF）。引一个能改节表、解析资源树的库会多一条要审计来源与许可的
依赖（21 §11），而本仓库既不改也不链这些二进制的内部结构。

实现上先读 4 KiB（绝大多数 PE 头就在这里），头靠后时只补读 `e_lfanew` 处的 6 字节：
不为了一个 6 字节的字段把整个几百 MB 的二进制读进内存。

## 实测证据

单元测试（`core-model/src/pe.rs`，4 用例，随模块一起搬过来）：

| 用例 | 断言 |
|---|---|
| `machine_values_map_to_the_documented_arches` | 0x8664→X64、0x014c→X86、0xaa64→Arm64、**0x0200(Itanium)→Other 而不是 X64** |
| `non_pe_and_truncated_input_are_named_not_panic` | 非 PE / 8 字节 / 截断都返回 `NotPe`，不 panic |
| `reading_a_real_file_reports_the_arch_or_an_explained_error` | 真 `System32\kernel32.dll` 读成 X64；不存在的路径给 `Unreadable` |

集成测试（`crates/cli/tests/dll_arch.rs`，5 用例，走完整 CLI）：

| 用例 | 断言 |
|---|---|
| `a_dll_of_the_wrong_bitness_is_refused_before_loading` | `STK-INP-0143`，详情含"本程序是 x64，该库是 x86"，且有可照做的 action |
| `a_non_pe_file_is_reported_as_not_a_pe_not_as_a_bitness_mismatch` | 4 种非 PE 输入（纯文本/空/截断/坏签名）都是 `STK-INP-0142` |
| `the_three_dll_failures_stay_distinguishable` | 0141/0142/0143 两两不同 |
| `a_matching_bitness_pe_passes_the_arch_check` | **负基线**：同位 PE 不被判成 0142/0143/0141 |
| `a_real_libass_passes_the_check_and_renders` | 真 libass 通过检查并真的产出 BDN XML |

这组测试**不需要外部素材**：位数检查发生在打开库之前，所以一个只有 PE 头的
假 DLL 就能触发它。`a_matching_bitness_pe_passes_the_arch_check` 是必需的负基线 ——
少了它，一个"什么都不接受"的实现也能让前三条全绿。

## 未做（诚实边界）

- **可选 hash allowlist 未实现**。当前的做法是：路径必须显式给（`--libass-dll` 或
  `STK_LIBASS_DLL`），哈希进报告与 `JobSnapshot`，但不做白名单强制。
  做成强制需要先定义"许可清单从哪来、谁维护、升级怎么办"，
  而 21 §6/§9 已经把 libass 的来源与哈希登记为**审计材料**而不是运行时门禁。
- **DLL 搜索路径加固**：已有（只接受显式给的路径，不做当前目录/PATH 搜索），
  但没有为它写一条专门的测试。这是一处已知的覆盖缺口，不主张已验收。
