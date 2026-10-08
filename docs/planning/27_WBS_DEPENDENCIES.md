# WBS 与依赖 DAG

## CORE
- C001 error/model/result
- C002 rational/time
- C003 canvas/profile
- C004 plan schema

## FORMAT
- F001 detector -> C001
- F010 SRT -> C002
- F020 ASS parser -> C002
- F021 ASS lossless writer -> F020
- F030 BDN -> C002,C003
- F040 PGS parser -> C002
- F041 PGS writer -> F040

## TRANSFORM
- T010 FPS -> C002
- T020 HDR -> F020
- T030 resolution -> F020,C003
- T040 safe normalize -> F010/F020

## GUI
- U001 Iced shell -> C001
- U002 drag/drop -> U001,F001
- U003 tool forms -> U001,C003
- U004 output profile table -> U003,C004
- U005 queue drawer -> task engine

## RENDER
- R001 libass FFI -> F020,C003
- R010 host protocol -> C001
- R011 host x64 -> R010
- R012 host x86 -> R010（仅在支持 x86 provider 时）
- R020 CSRI adapter -> R011/R012（独立可选 provider，不是 libass/BDN 前置）
- R030 VapourSynth -> R011（legacy 可选）
- R031 AviSynth -> R011/R012（legacy 可选）

## IMAGE/BDN
- I001 RGBA/bbox -> R001
- I002 quantizer -> I001
- B001 PNG -> I002
- B002 BDN writer -> F030,B001

## PGS
- P001 decode -> F040
- P002 fast transform -> F040
- P003 basic author -> I002,F041
- P004 validator basic -> F040
- P010 advanced author -> P003 + research corpus
- P011 strict validator -> P010

## TASK
- J001 job model -> C004
- J002 scheduler -> J001
- J003 cache -> J002
- J004 transaction/verify -> J002
- J005 resume -> J003/J004

## ADAPTER
- A001 ffprobe -> process runner
- A002 mkvtoolnix -> process runner
- A003 font service -> Windows 字体目录/注册表枚举 + 字体表解析；DirectWrite 权威解析尚未实现

## Critical Path
CLI 文本路径：`C001/C002 -> F010/F020/F021 -> T010/T020/T030 -> J004 -> CLI verify`。

渲染交付路径：`F020/C003 -> R001 + R010/R011 -> I001/I002 -> B001/B002 或 P003 -> validator`。

PGS Strict、CSRI、x86、legacy adapter、GUI 和 MKV 不属于上述核心路径。它们各自有独立证据门槛；外部裁决未完成时，不得把 Basic 输出升级成 Strict 合规声明。
