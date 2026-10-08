# 第三方来源与研究归档

`SOURCE_AUDIT_LOCK.json` 保存研究来源、版本、提交和哈希。`sources/` 用于研究源码归档。2026-10-08 本次依赖补齐期间，两份已跟踪归档在工作区显示删除；原锁与 Git 基准保留历史证据。未恢复这些并行变更，也不将当前缺失描述为哈希通过。

两份解压源码树都已与来源锁的上游 commit codeload 内容逐文件核对：libass 136 个文件、ass2bdnxml 24 个文件均无差异。压缩归档的 SHA-256 单独锁定；codeload commit 包与原归档的根目录名和 gzip 封装不同，所以不应要求两份压缩包哈希相等。

这些文件不是本项目 Rust 源码。源码归档用于研究，不是应用运行时依赖目录；`VSFilterMod.dll` 是未集成的候选文件。libass 主代码采用 ISC，但归档还含 Unlicense、Public Domain、BSD-2-Clause 和 CC0-1.0 文件；ass2bdnxml 主代码采用 GPL-3.0-or-later，其中 `x86inc.asm` 有单独 ISC 声明。分发源码时必须保留各文件的上游许可声明；不要把 GPL/AGPL 实现直接复制或翻译进核心模块。

## libass 开发期运行库

用户提供的 `libass-9.dll` 与 [MSYS2 UCRT64 libass 0.17.5-1 包](https://packages.msys2.org/packages/mingw-w64-ucrt-x86_64-libass)内的文件逐字节一致；官方包压缩 SHA-256 为 `4385fd47a77784478878f0f03f2d9afa59a5ac71facb7cad1dfe19c555e75762`。DLL 为 x64，SHA-256 为 `cb2b46133b3bd9630384148bd7229e987af9703208d4329fc2215f4170b8dc6b`。

原目录 `D:\xia\mingw-w64-ucrt-x86_64-libass-0.17.5-1-any.pkg` 不包含其依赖 DLL。按 `.BUILDINFO` 的 UCRT64 版本从官方仓库补齐包，并按 PE 导入链复制 20 个 DLL（libass 本体加 19 个依赖）到本目录；未复制不需要的 EXE、静态库或其他 HarfBuzz 扩展 DLL。下载与解压材料位于非 C 盘的 `target/native-msys2-20261008/`。

设置 `STK_LIBASS_DLL` 后已通过真实 libass、renderer-host 和 CLI BDN 渲染测试。真实测试发现并修正 `ass_read_memory` 第四参数误传 track 名的问题；当前显式传 `UTF-8`。逐包版本、下载 URL、压缩包摘要、逐 DLL 摘要、PE 导入与测试范围见[运行库锁](LIBASS_RUNTIME_LOCK.json)。包元数据与可用许可正文见 [`licenses/msys2/`](licenses/msys2/)。

这些是用户授权纳入的开发期运行材料。libass 主体 ISC 不代表整组 DLL 都是 ISC；运行链包含 LGPL 等包声明，包声明也不替代逐文件许可裁决。尚未完成产品分发义务复核，也未验证依赖包 detached signature，不作为已通过审计的安装包内容。

```powershell
. .\tools\set_build_cache.ps1
$env:STK_LIBASS_DLL = (Resolve-Path '.\third_party\libass-9.dll').Path
$env:STK_REQUIRE_LIBASS = '1'
cargo test -p renderer-host --test host_isolation
cargo test -p render-libass --test render_real
cargo test -p cli --test render
```

## VSFilterMod 候选

`VSFilterMod.dll` 是随当前工作区提供的 x64 第三方候选二进制，版本资源显示 ProductVersion 2.5.2.2。PE 导出表包含 CSRI 符号，PDB 路径指向 `VSFilterMod` 构建目录；上游 `sorayuki/VSFilterMod` 项目声明 GPL-3.0，仓库更新时间与 DLL 时间戳接近。当前按 GPL-3.0 记录，但尚未把 DLL 哈希对应到官方 release 资产或确定的源码提交。当前项目没有实现 VSFilterMod/CSRI provider 的加载与隔离，也没有真实渲染验证，因此该 DLL 目前只作为项目材料保存，不代表可运行能力或可发布安装包内容。哈希与静态检查记录见来源锁。

- [来源锁](SOURCE_AUDIT_LOCK.json)
- [第三方声明](../THIRD_PARTY_NOTICES.md)
- [依赖与许可证规格](../docs/specs/21_DEPENDENCIES_LICENSES.md)
- [历史源码研究](../docs/research/40_SOURCE_CODE_AUDIT.md)
- [代码审查与修复](../docs/reviews/2026-10-08.md)
- [文档优化与 libass 接入记录](../docs/reviews/2026-10-08-docs.md)

来源锁中的本机 DLL/工具路径属于历史证据，不是安装要求。更新依赖时重新核验来源、许可证、归档哈希和实际互操作结果。
