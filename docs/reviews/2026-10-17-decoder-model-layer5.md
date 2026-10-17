# 第 5 层解码器模型裁决（13 §6 Decoder Model）

日期：2026-10-17
范围：`WBS-V003`（缓冲/解码器模型）、`ISSUES` 中"Strict validator 需要缓冲模型"一条
代码：`crates/validator/src/decoder_model.rs`、`crates/codec-pgs/src/encoder.rs`

## 1. 为什么要先用真实语料反推，而不是照抄理论公式

13 §6 把第 5 层列为 "coded object buffer、window/object decode time、acquisition"。
可查到的理论模型（US7620297B2 / US8638861B2，FFmpeg `supenc.c` 的编码侧实现 `be8610f`）给的是
**编码器的前向模型**：

```
PCS/WDS/PDS: DTS = PTS - decode_duration
First ODS  : DTS = PTS - decode_duration;  PTS = DTS + ceil(w*h*FREQ/Rd)
Epoch Start: DECODE_DURATION = ceil(FREQ * video_w * video_h / Rc)
```

先拿这套公式去量真实的商业蓝光流，结果是 **0% 命中**：

| 量 | 理论公式预期 | 真实语料实测（98.97 MB，9951 个显示集） |
| --- | --- | --- |
| `PDS` 的 `PTS - DTS` | `decode_duration`（>0） | **恒为 0**（8558/8558） |
| `END` 的 `PTS - DTS` | `decode_duration`（>0） | **恒为 0**（9951/9951） |
| `ODS` 的 `PTS - DTS` | `ceil(w*h*FREQ/Rd)` | 1..3494 tick，与 `w*h*FREQ/Rd` **无相关**（11286 条 0% 命中） |

结论：**这套公式在真实流上不成立**。如果照它实现，会给每一份真实文件刷上万条假阳性，
用户会直接弃用这一层——那比不实现更糟。

## 2. 实测出来的不变式（每条都在真实语料上 100% 成立）

于是改为反推**真实流满足的那套**不变式。全部用同一份 98.97 MB 真实商业 SUP 验证：

| # | 不变式 | 实测 |
| --- | --- | --- |
| 1 | 每个显示集内，所有段的 PTS ≤ 该集 PCS 的 PTS | 9951/9951 **100%** |
| 2 | 每个显示集内，所有段的 DTS ≤ 自己的 PTS | 9951/9951 **100%** |
| 3 | 全局 DTS 单调不减 | 49659 段，**0 次递减** |
| 4 | `END.PTS` == 同集最后一个 `ODS` 的 PTS | 8520/8520 **100%** |
| 5 | `PDS` 与 `END` 的 `DTS == PTS` | 各 100% |

真实流的实际形状（真实语料第一个显示集）：

```
PCS PTS=303750 DTS=297870   ← PCS 是呈现时刻
WDS PTS=303702 DTS=297870
PDS PTS=297870 DTS=297870   ← 本集的共同解码时刻，无耗时
ODS PTS=297961 DTS=297870
END PTS=297961 DTS=297961   ← 对齐最后 ODS，无耗时
```

**PTS 不是单调的**（40% 的相邻对递减），那是对的：数据可以提前传，解码顺序由 DTS 保证。
这一条很关键——按 PTS 单调去校验会满盘假阳性。

## 3. 用真实语料抳出来的两个**我们自己的**编码器缺陷

写这层的收益不只是"多了一项检查"，而是它立刻暴露了 `pgs-author` 的两处错误：

### 3.1 `PDS` 被写成了"有解码耗时"

旧代码：`push(&mut segments, SEG_PDS, &pds, frame.pts, dts)` —— PTS 取呈现时刻、DTS 取提前量。
真实流：`PDS` 的 PTS **等于**它自己的 DTS，且等于本集的共同解码时刻。

修正：`push(..., SEG_PDS, &pds, dts, dts)`。

### 3.2 `END` 同样被写成了"有解码耗时"

旧代码：`push(&mut segments, SEG_END, &[], frame.pts, dts)`。
真实流：`END.DTS == END.PTS == 最后 ODS 的 PTS`，100% 满足。

修正：`push(..., SEG_END, &[], frame.pts, frame.pts)`。

两处都是"从'所有段都有解码耗时'这个没经检验的假设推出来"的产物。
它们此前测不出来，是因为校验器自己也有同样的假设——**对错一致**。

## 4. 本层**不做**的两件事（写进报告，不藏在注释里）

报告在跑完层 5 后固定追加一条 `PGS.DEC.20`（info），明确写出未含范围：

* **不算 coded object buffer 的占用曲线**：真实流不带"缓冲区尺寸"字段，
  `Rx`/`Rd` 这些速率在流里无从观测。可以按假设的速率算一条曲线，
  但那是在验我们的假设，不是在验这个文件。
* **不判 acquisition point / seamless 分支**：需要 PCS composition state 的
  严格定义与"哪些集可作为入口"的真盘裁决（13 §10 已列该缺口）。

所以报告说的是"层 5 已执行的不变式集合（5 条）"，不是"层 5 完成"。

## 5. 覆盖缺口的账目更正

`ValidationReport::COVERAGE_GAP_CHECKS` 原来是 `["PGS.DEC.01", "PGS.DEV.01"]`。
层 5 落地后 `PGS.DEC.01` 被回收为**真不变式**（"段 PTS 晚于本集 PCS"），
所以它必须从缺口清单里移出——否则一条真实的时序警告会被读成"层没做"，
而徽章却被扣在别的原因上。现在缺口只剩 `PGS.DEV.01`（第 7 层设备语料）。

`strict_badge_allowed` 因此仍恒为 false（第 7 层没法在本机补），
但现在报告能说清**为什么**：CLI 新增 `coverage_gap` 字段列出真的没跑的层。

## 6. 验收证据

| 判据 | 入口 |
| --- | --- |
| 真实商业语料上 5 条不变式 0 违例 | `crates/validator/tests/decoder_model_real.rs`（49659 段 / 9951 集，实测输出） |
| 同一语料被改坏后必须报出来 | 同上第二个用例（反向对照） |
| 每条不变式各有一个能生效的反例 | `decoder_model::tests::each_invariant_has_a_working_counterexample` |
| 真实流的三种骨架都通过 | `the_three_real_world_shapes_all_pass`（8520/1393/38 次的实测分布） |
| 不变式之间的蕴含关系有记录 | `key_invariants_are_related_and_that_is_documented` |
| 未含范围写进报告 | `the_report_states_what_layer_5_did_not_check` |
| 修好的编码器产物能过本层 | `pgs-author` 的 `authored_stream_is_valid_and_keeps_canvas_positions` |

语料通过 `STK_REAL_SUP` 指定；没有时**明确打印跳过理由**，不算通过。
`STK_REQUIRE_REAL_SUP=1` 可在门阶上要求它必须存在。

## 7. 仍未覆盖（不声称通过）

- **第 7 层设备语料**：需要真机/播放器矩阵，本机无法取得。这是徽章仍被拦下的唯一原因。
- **缓冲区占用曲线**与 **acquisition/seamless 分支**：见 §4。
- **只有一份语料**：一条真实 SUP 上的 100% 不等于"所有商业流都这样"。
  不变式取的是**必要条件**（不满足一定有问题），不是充分条件；
  这一层不会因为"过了"就宣称文件合规。该语料是简英双语特效字幕
  （9951 个显示集、11286 个 ODS），不含滚动字幕/裁剪窗口等更复杂形态。
- **`PDS` 的 `PTS` 是否必须等于 DTS**：实测 100%，但那也可能是这家 authoring 工具的
  习惯。因此 `PGS.DEC.04` 定为 **warning 而非 error**——不满足不阻断流水线。
