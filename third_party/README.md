# 第三方来源与研究归档

`SOURCE_AUDIT_LOCK.json` 保存研究来源、版本、提交和哈希。`sources/` 包含 libass 与 ass2bdnxml 的上游源码归档；ass2bdnxml 另有解压目录供研究使用。归档均按来源锁记录 SHA-256，源码不是应用运行时依赖。

上游源码归档与来源锁的提交和 SHA-256 对应。ass2bdnxml 解压树曾与来源锁 commit 的 codeload 内容逐文件核对；压缩归档哈希单独锁定。codeload commit 包与归档的根目录名和 gzip 封装不同，因此两种归档的压缩包哈希不应相等。

这些文件不是本项目 Rust 源码。源码归档用于研究，不是应用运行时依赖目录；`VSFilterMod.dll` 是未集成的候选文件。libass 主代码采用 ISC，但归档还含 Unlicense、Public Domain、BSD-2-Clause 和 CC0-1.0 文件；ass2bdnxml 主代码采用 GPL-3.0-or-later，其中 `x86inc.asm` 有单独 ISC 声明。分发源码时必须保留各文件的上游许可声明；不要把 GPL/AGPL 实现直接复制或翻译进核心模块。

## libass 分发运行库

项目随附的 x64 libass 运行库含 libass 及 19 个依赖 DLL。逐文件哈希已与官方 MSYS2 UCRT64 包归档核对；`libass-9.dll` 与 [libass 0.17.5-1 官方包](https://packages.msys2.org/packages/mingw-w64-ucrt-x86_64-libass)一致。fontconfig 配置也与对应官方包内容逐文件核对。最终运行库来源、包版本、包归档 SHA-256、DLL SHA-256、SPDX 声明和源码包地址记录在[运行库锁](LIBASS_RUNTIME_LOCK.json)。

便携包和 Inno Setup 安装包均随附 20 个 DLL、fontconfig 配置、逐包 `.PKGINFO`/`.BUILDINFO`、许可证正文、源码包链接和运行库锁。构建脚本逐项校验 DLL 与配置文件哈希；CLI 只在可执行文件同目录存在 `libass-runtime.flag` 时自动加载随包 DLL。libass 主体 ISC 不代表整组 DLL 都是 ISC；运行链包含 LGPL、FreeType 双许可和 GCC Runtime Exception 等声明。依赖包 detached signature 未验证；官方包页面提供的 SHA-256 与下载归档一致。

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
