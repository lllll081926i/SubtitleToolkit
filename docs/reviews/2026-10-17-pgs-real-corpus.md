# 真实第三方 SUP 互操作裁决（Phase 3）

核对日期：2026-10-17。本文记录用一份**真实商业字幕**驱动 PGS 校验器得到的结果，
以及由此修正的两个真实现缺陷。证据可复现，不引用历史门阶。

## 语料

| 项 | 值 |
|---|---|
| 文件 | `D:\lllll\Videos\挽救计划.Project.Hail.Mary.2026.1080p.SDR.简英双语特效@KKYY.sup` |
| 大小 | 103,778,137 字节（98.97 MB） |
| SHA-256 | `694f2e0d5d7c3e5368b85f5b42330e0c8eccbd44de2a89984e0383481ba9ad31` |
| 画布 | 1920x1080 |
| 段数 | 49,659 |
| 来源 | 用户提供的第三方成品字幕（简英双语特效），非本项目自产 |
| 许可/使用范围 | 仅本机测试，不作为公共 fixture 提交 |

这满足 [24 §5](../specs/24_TEST_QA_CORPUS.md) 对「第三方真实 SUP」的要求，也把
AT-PGS-001 从「缺语料未覆盖」推进到「有真实样本、已执行」。

## 首轮结果（修复前）：21,237 条 error

`pgs validate --level standard` 报 `error=21237`，全部是 `PGS.STR.06`：

```
object 0 声明 data_length 5789，实收 5785 字节｜段偏移 150
object 1 声明 data_length 6151，实收 6147 字节｜段偏移 6222
```

差值**恒为 4**，且每个 display set 都命中。恒定差值是「解析器算错了」的信号，不是
「文件坏了」的信号 —— 损坏不会如此整齐。

## 缺陷一：ODS `data_length` 语义算错（假阳性）

直接读文件字节（第一个 ODS，偏移 150）：

```
00 00 | 00 | c0 | 00 16 9d | 01 2b | 00 36 | <RLE...>
obj_id  ver  seq  data_len    w=299   h=54
```

`data_length = 0x00169d = 5789`，实际 RLE 5785 字节，差 4 = 2(宽) + 2(高)。

三个独立来源一致确认规格：

- **ProofPGS** `PGS_FORMAT.md`：Object Data Length「**including** the 4 bytes for
  Width+Height」；「actual RLE data size is Object Data Length **minus 4**」。
- **libpgs** `OdsData.data_length` 注释：「includes 4 bytes for width+height」，
  其序列化器写作 `self.rle_data.len() as u32 + 4`。

**根因**：`data_length` 覆盖「宽高 + RLE」，我们的校验器却拿它与纯 RLE 字节数比较。
更关键的是**编码器也写错了**（`encoder.rs` 写 `rle.len()` 而漏掉那 4 字节），
读、写、校验三者「对错一致」，所以自产流测试全绿，直到拿真实文件才暴露。

**修复**：`validator/src/pgs.rs` 新增 `ODS_LENGTH_INCLUDES_DIMENSIONS` 与
`ods_length_matches()`；`codec-pgs/src/encoder.rs` 新增 `ODS_DIM_FIELDS`，写字段时加上。
两侧都补了锁定规格的回归用例。

## 缺陷二：`PGS.TML.02` 基于发明出来的不变量（假阳性）

修完缺陷一后暴露 `error=9951`，全部是 `PGS.TML.02`「display set 内各段时间戳不一致」。
原规则要求同一 display set 内所有段共享一对 PTS/DTS。

实测该文件的 display set 0：

```
PCS pts=303750  WDS pts=303702  PDS pts=297870  ODS pts=297961  END pts=297961
```

段间 PTS **各不相同**。权威来源确认这是正常行为：

- **FFmpeg** `supenc`（commit `be8610f`）：「Each segment gets its own PTS/DTS in
  the SUP header based on its role in the decode pipeline」，并给出逐段推导：
  PCS/WDS/PDS 用 base PTS；首个 ODS 的 PTS = DTS + 对象解码时间；后续 ODS 与 END
  沿用上一个 ODS 的时钟。

**根因**：原规则把一个**不存在**的不变量当成了规格。

**修复**：改为校验真实存在的不变量 —— 每段自己 `DTS ≤ PTS`（先解后显），且
任何段的 PTS 不得晚于它所属集合的 PCS（显示时数据必须已在解码器里）。
测试相应重写：新增「段间时钟不同不得报」与「先显后解 / 晚于 PCS 必须报」两条。

## 缺陷三：Fast Path 要求逐段 PTS 单调（假阳性）

修完前两个后，`pgs adjust --offset-ms 1000` 报 `STK-PGS-0090 PTS 非单调递增`：

```
offset=Some(32) 303750 -> 303702
```

`codec_pgs::fastpath::check_timeline` 要求**每一段的 PTS 全局非降** —— 与缺陷二同源的错误
假设。改为：**单调性只看 PCS**（display set 级时钟），段内仍查 `DTS ≤ PTS`。

## 缺陷四：对象定义判为“不跨 display set 持续”（假阳性）

接着 `pgs adjust` 报 `STK-VAL-0001 存在未定义的 composition object 引用 [0, 1, 2, 3]`。

`codec_pgs::display_set::unresolved_objects` 只在**同一个 display set 内**找 ODS 定义。
实测该文件的 set 168–182：

```
set 172 state=0x40(ACQ) refs=[0, 1] local_defs=[(0, 11), (1, 0)]   ← Acquisition Point 把对象都定义好
set 173 state=0x00(norm) refs=[2, 1] local_defs=[(2, 11)]          ← 只引用对象 1，不重定义
set 174 state=0x00(norm) refs=[0, 1] local_defs=[(0, 12)]          ← 同理
```

全文件有 **1,641 个**这样的集合。PGS 的对象定义本来就跨集合持续有效，
Acquisition Point 建立、Normal 集合复用。改为按集合顺序累积已定义 id。

（本项目自产的 Basic 流每个集合都自带定义，所以这个区别在自产流上永远看不出来。）

## 修复后结果：`standard` 校验与 Fast Path 均通过

```
pgs validate --level standard --json
ok=True   summary=info=8623 warning=0 error=0 fatal=0
no_blocking=True   exit=0
```

剩余 `info` 全部是 `PGS.LC.01`（对象版本递增 redefinition），是合法的 Advanced
Author 对象复用，不是问题。

### AT-PGS-001：Fast Path 后 ODS payload 完全一致（已执行）

```
pgs adjust --offset-ms 1000 --out shifted.sup   → exit 0
```

独立字节级对拍（49,659 段逐段比对）：

| 断言 | 结果 |
|---|---|
| 段数 | 49,659 → 49,659（不变） |
| 段类型序列 | 逐段一致 |
| **ODS payload 逐字节一致** | 11,286 个 ODS，不一致 0 |
| 非 ODS 段 payload 一致 | 不一致 0 |
| PTS 恰好 +90,000 tick | 不一致 0 |
| DTS 恰好 +90,000 tick | 不一致 0 |
| ODS 聚合 SHA-256 | `f46c26b1…db1008`（两份完全相同） |
| 平移后文件自校验 | `error=0`，`no_blocking=True` |

CLI 自报 `ods_invariant.objects = 11286`，与独立统计精确相等。

这满足 AT-PGS-001 的“ODS payload hash 完全一致”要求。

### AT-PGS-002 / AT-PGS-003：Forced 与调色板变换

同样在该文件上执行，均 exit 0，且独立对拍确认“只改目标字段、ODS 不变”：

| 操作 | 命令 | 变化的段类型 | ODS 摘要 | 段数 | forced 对象数 |
|---|---|---|---|---|---|
| 基线 | — | — | `f46c26b1…` | 49,659 | 0 |
| Forced-only | `--forced yes` | 仅 `PCS (0x16)` | 一致 | 不变 | 0 → 12,933 |
| 调色板 | `--luma-basis-points 9000` | 仅 `PDS (0x14)` | 一致 | 不变 | 保持 0 |

两份输出自校验均为 `error=0` / `no_blocking=True`。
AT-PGS-002 要求“Forced-only Fast Path 不重编码”、AT-PGS-003 要求“palette dim 只改 PDS”——
两条均满足：ODS 字节未动，改动只落在应当改的段类型上。

## 边界（不要把这次结果说过头）

- 本次只跑到 **standard** 层。第 5 层（T-STD 缓冲模型）与第 7 层（设备语料）仍未实现，
  strict 仍会给出 warning 并拒绝发徽章，见 [2026-10-09 审查第五节](2026-10-09-code-audit-defense.md)。
- `PGS.TML.02` 的新判据（段 PTS ≤ 集合 PCS PTS）与 `check_timeline` 的“只看 PCS 单调”
  是从“先解后显”与段角色推导出的**必要条件**，尚未逐设备取证；它们比旧规则弱且不再误杀，
  但没有得到真机确认。
- `unresolved_objects` 现在只按集合顺序累积定义，**不重置 epoch 边界**。真实文件里
  Epoch Start 之后会重新定义；若某个 epoch 依赖上一 epoch 的残留对象，当前实现会放过。
  这是一个偏宽的选择（不误杀），但严格的 epoch 语义仍需真机/规格语料裁决。
- 本次验证了**解析、校验与 Fast Path（时间平移 / Forced / 调色板）**。
  AT-PGS-001/002/003 均已在真实语料上执行并通过。
- 第 7 层**设备**语料仍未取得：AT-PGS-001–003 证明的是本工具的结构与字节级正确性，
  不能推出“主流播放器/蓝光机一定接受该输出”。
- MKVToolNix v102.0 已登记可用性与哈希（21 §9），但 AT-MKV-001 仍未实现。
- 该素材许可范围是「本机测试」，不得复制进公共 fixture。

## 复现命令

```powershell
. .\tools\set_build_cache.ps1
cargo build --release -p cli
.\target\release\subtitletool-cli.exe pgs validate `
  'D:\lllll\Videos\挽救计划.Project.Hail.Mary.2026.1080p.SDR.简英双语特效@KKYY.sup' `
  --level standard --json
```

日志：`target/gate/final5.log`。
