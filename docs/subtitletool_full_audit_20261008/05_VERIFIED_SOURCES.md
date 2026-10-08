# 本轮联网核验台账（2026-10-08）

> 来源分三种：**官方/标准**（可直接支撑API、时间或许可声明）、**开源项目的作者文档/源码**（证明其实现，不自动成为蓝光正式规范）、**待本机验证**（现有文件写了但本次未运行）。静态检查≠功能已经测试通过。

| 序号 | 主题和来源 | 本轮得到的可采信结论 | 证据等级 | 后续行动 |
|---|---|---|---|---|
| E01 | Iced 0.14 [GitHub release](https://github.com/iced-rs/iced/releases/tag/0.14.0) / [Changelog](https://github.com/iced-rs/iced/blob/master/CHANGELOG.md) | 0.14版有time-travel、E2E、headless、hot reload、table/grid；当前master为0.15开发分支，不能用master接口代替0.14。 | 官方仓库+版本说明 | 固定Cargo版本、单独Windows drag-drop PoC |
| E02 | [libass ass.h](https://github.com/libass/libass/blob/master/libass/ass.h) / [repo](https://github.com/libass/libass) | 渲染前需frame_size、storage_size、fonts；LayoutRes会优先影响部分源尺寸/PAR计算；ISC；0.17.5为2026-06-24发布版本。 | 官方源码/项目 | 固定tag/hash并做native test |
| E03 | [VSScript4 API](https://www.vapoursynth.com/doc/api/vsscript4.h.html) / [官方示例](https://github.com/vapoursynth/vapoursynth/blob/master/sdk/vsscript_example.c) | 通过`getVSScriptAPI(version)`做接口协商，不能复制旧工程VSAPI偏移或乱拼导出名。 | 官方接口 | 在Windows x86/x64实测兼容矩阵 |
| E04 | [FFmpeg pgssubdec.c](https://ffmpeg.org/doxygen/trunk/pgssubdec_8c_source.html) | PDS条目读取`color_id,Y,Cr,Cb,alpha`五个字节；旧AssRocket把RGB直接塞YCrCb是错的。**不要将五字节条目误说成四字节**。 | 开源参考解码器源码 | 编写4/5字节反例+独立decoder对照 |
| E05 | [ass2bdnxml release notes](https://github.com/cubicibo/ass2bdnxml/releases) | 2160p是exotic/non-Blu-ray输出；UHD BD PGS不得与3840×2160位图混淆。 | 开源作者说明，非完整官方Blu-ray规范 | 正式Strict需要额外标准或authoring实验台核对 |
| E06 | [SUPer](https://github.com/cubicibo/SUPer) | `--max-kbps`测试输出码率而不是保证流量被shape；ahead/layout/acquisition等才属于实际策略。 | 作者文档 | 审计Strict时记录其参考commit |
| E07 | [Windows 10生命周期](https://learn.microsoft.com/en-us/lifecycle/announcements/windows-10-22h2-end-of-support-update) / [当前状态](https://learn.microsoft.com/en-gb/windows/release-health/status-windows-10-22h2) | Win10 22H2于2025-10-14结束常规支持；ESU或LTSC是另一个支持问题。 | Microsoft官方 | 文档/CI/安装器支持矩阵更正 |
| E08 | [libimagequant LICENSE](https://github.com/ImageOptim/libimagequant/blob/main/README.md) / [Cargo.toml](https://github.com/ImageOptim/libimagequant/blob/main/Cargo.toml) | 4.x GPLv3+ / 商业双许可，不应默认硬依赖于目标许可未定的核心。 | 上游许可证与Crate manifest | 许可证Gate继续保留 |
| E09 | [FFmpeg法律说明](https://ffmpeg.org/legal.html) | 根据配置可能是LGPL/GPL，捆绑Windows二进制时必须检查构建配置/分发和源码义务。 | FFmpeg官方 | ffprobe分发方案逐build固定 |
| E10 | [Microsoft字体FAQ](https://learn.microsoft.com/en-us/typography/fonts/font-faq) / [OpenType fsType](https://learn.microsoft.com/en-us/typography/opentype/spec/os2) | Windows随系统字体一般不能直接重新分发；文档嵌入有明确许可证和fsType约束；字体subset/附入MKV不代表自动获得再分发权。 | Microsoft/OpenType官方 | `font-service`增加license gate |
| E11 | [ITU-R BT.2408-6](https://www.itu.int/dms_pub/itu-r/opb/rep/R-REP-BT.2408-6-2023-PDF-E.pdf), PDF正文第5页第2.1节（PDF第7页） | 203 cd/m²是HDR参考白/Graphics White范畴的参考级别，不是ASS文件8位颜色能够独立保证的显示亮度。 | ITU公开技术报告 | 明确颜色预调整与真实显示亮度区别 |
| E12 | [Inno Setup 7 downloads](https://jrsoftware.org/isdl.php) / [revision history](https://jrsoftware.org/files/is7-whatsnew.htm) | 7.1.0发布于2026-08-12；提供x64编译器/安装器，商业用途有独立许可注意事项。 | 官方发布页 | 锁版本，安装打包CI验收 |
| E13 | [MKVToolNix](https://mkvtoolnix.download/doc/) / [项目代码说明](https://gitlab.com/mbunkus/mkvtoolnix) | 成熟容器/轨道工具；源码与二进制分发许可证必须再次对照项目原始仓库。 | 官方项目文档，许可细节需锁定版本 | 选择用户外部工具/随包并做合规决策 |
| E14 | [OxideAV subtitle image](https://github.com/OxideAV/oxideav-sub-image) | 有纯Rust PGS解码/编码实现面，但并未证明达到Strict Authoring级别。 | 开源项目README | Phase0先执行corpus bakeoff |
| E15 | [ASSfonts Rust](https://github.com/kotorimiku/assfonts) | 提供字幕字体匹配、字形统计与子集化实现思路；subset需要用渲染差分测试证明GSUB/GPOS与复杂脚本兼容。 | 作者项目说明+上轮部分源码 | 先做字体健康检查，subset降级为可选 |

## 已核查但不能直接称为“标准正确”的事项

1. VSFilterMod历史多个分支、CSRI接口与扩展滤镜实际功能仍必须在**真实Windows进程**加载对应DLL才能确定；文件名和README不足以证明。
2. PGS的“真正蓝光严格兼容”需要官方Authoring约束、buffer验证模型、标准解码器、硬件样本交叉校验。开源参考不能独自替代权威规范。
3. ASS→HDR的“203nit”不能脱离播放器色彩合成管线实现绝对显示亮度，UI应明确是字幕颜色预调整。
4. 许可证静态查验不构成法律意见。GPL/AGPL“通过进程隔离”不自动带来任何法律豁免。
5. 本次没有获取所有第三方仓库的提交hash，也没有静态编译或运行它们；旧V3.1 `40_SOURCE_CODE_AUDIT.md` 是先前逐源码查看的记录，尚不是可复现的完整仓库审计。

## 建议的证据记录 JSON 结构

```json
{
  "repo": "owner/project",
  "source_url": "https://...",
  "tag": "release_tag_or_commit",
  "commit_sha": "40_char_sha",
  "files_reviewed": ["src/path.ext"],
  "blob_sha256": {"src/path.ext":"hex"},
  "license_spdx": "verified SPDX expression",
  "license_source": "source path or release URL",
  "check_date": "2026-10-08",
  "verified_in_windows_vm": false,
  "tests": []
}
```
