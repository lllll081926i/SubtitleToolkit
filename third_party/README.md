# 第三方来源与研究归档

`SOURCE_AUDIT_LOCK.json` 保存研究来源、版本、提交和哈希。`sources/` 保存研究用源码归档；当前工作树中的归档 SHA-256 与来源锁一致。Git 是否已收录这些文件，以工作树状态为准。

两份解压源码树都已与来源锁的上游 commit codeload 内容逐文件核对：libass 136 个文件、ass2bdnxml 24 个文件均无差异。压缩归档的 SHA-256 单独锁定；codeload commit 包与原归档的根目录名和 gzip 封装不同，所以不应要求两份压缩包哈希相等。

这些文件不是本项目 Rust 源码。源码归档用于研究，不是应用运行时依赖目录；`VSFilterMod.dll` 是未集成的候选文件。libass 主代码采用 ISC，但归档还含 Unlicense、Public Domain、BSD-2-Clause 和 CC0-1.0 文件；ass2bdnxml 主代码采用 GPL-3.0-or-later，其中 `x86inc.asm` 有单独 ISC 声明。分发源码时必须保留各文件的上游许可声明；不要把 GPL/AGPL 实现直接复制或翻译进核心模块。

`VSFilterMod.dll` 是随当前工作区提供的 x64 第三方候选二进制，版本资源显示 ProductVersion 2.5.2.2。PE 导出表包含 CSRI 符号，PDB 路径指向 `VSFilterMod` 构建目录；上游 `sorayuki/VSFilterMod` 项目声明 GPL-3.0，仓库更新时间与 DLL 时间戳接近。当前按 GPL-3.0 记录，但尚未把 DLL 哈希对应到官方 release 资产或确定的源码提交。当前项目没有实现 VSFilterMod/CSRI provider 的加载与隔离，也没有真实渲染验证，因此该 DLL 目前只作为项目材料保存，不代表可运行能力或可发布安装包内容。哈希与静态检查记录见来源锁。

- [来源锁](SOURCE_AUDIT_LOCK.json)
- [第三方声明](../THIRD_PARTY_NOTICES.md)
- [依赖与许可证规格](../docs/specs/21_DEPENDENCIES_LICENSES.md)
- [历史源码研究](../docs/research/40_SOURCE_CODE_AUDIT.md)
- [本次纠正的格式结论](../docs/reviews/2026-10-07.md)

来源锁中的本机 DLL/工具路径属于历史证据，不是安装要求。更新依赖时重新核验来源、许可证、归档哈希和实际互操作结果。
