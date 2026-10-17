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

## 修复后结果

```
pgs validate --level standard --json
ok=True   summary=info=8623 warning=0 error=0 fatal=0
no_blocking=True   exit=0
```

剩余 `info` 全部是 `PGS.LC.01`（对象版本递增 redefinition），是合法的 Advanced
Author 对象复用，不是问题。

## 边界（不要把这次结果说过头）

- 本次只跑到 **standard** 层。第 5 层（T-STD 缓冲模型）与第 7 层（设备语料）仍未实现，
  strict 仍会给出 warning 并拒绝发徽章，见 [2026-10-09 审查第五节](2026-10-09-code-audit-defense.md)。
- `PGS.TML.02` 的新判据（段 PTS ≤ 集合 PCS PTS）是从「先解后显」推出的**必要条件**，
  尚未逐设备取证；它比旧规则弱且不再误杀，但没有得到真机确认。
- 这次只验证了**解析与校验**。AT-PGS-001 要求的「+1000ms 后 ODS payload hash 完全一致」
  属于 Fast Path 变换验证，需要单独执行。
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
