# CLI 与自动化规范

本文定义 CLI 的目标契约。命令是否已经可用，以[功能矩阵](03_FEATURE_MATRIX.md)和当前状态为准。规格中列出命令不代表命令已经实现。

## 1. 当前可执行文件

当前工作区生成 `subtitletool-cli.exe` 和 `subtitletool-render-host.exe`。桌面 GUI `subtitletool.exe` 尚未实现。

当前已实现命令：

- `probe`、`convert`、`time`、`hdr`；
- `render`、`batch`；
- `pgs adjust`、`pgs validate`、`pgs encode`；
- `font scan`、`font subset`、`mkv identify|extract|remux`、`cache stat|clear`、`doctor`。

全部已规划命令都已实现，`doctor` 的 `not_built` 为空。`--preset` 尚未实现（它不是命令，走参数层的明确拒绝）。

全局开关：`--json`、`--dry-run`、`--out`、`--overwrite`、`--no-font-cache`、
`--no-render-cache`（37 §2：不用磁盘渲染缓存）、`--inject-fonts`。

## 2. 命令与退出码

- `probe` 探测字幕格式与编码；视频输入需要 ffprobe，并只输出建议，不自动修改时间轴。
- `convert` 支持 ASS/SSA 与 SRT 之间的转换，并报告信息损失。
- `time` 执行偏移、帧率、时长和两点校准变换。
- `hdr` 修改 ASS 颜色字段。
- `render` 将 ASS 渲染为 BDN XML 与 PNG；需要可用的 libass DLL。
- `pgs adjust|validate|encode` 分别处理 PGS 快速变换、分层校验和 Basic 编码。Basic 编码不代表 Strict Blu-ray 合规。
- `batch` 从 JSON 任务规格运行多个输入与输出 profile。
- `font scan` 扫描 ASS 字体请求并生成报告；`--copy-fonts` 可交付完整字体文件。
- `font subset` 做安全字体子集（17 §5）：复杂脚本回退整份，其余做 cmap 级等价检查。
  `--embed-fonts --out X.ass` 把字体写进产物 ASS 的 `[Fonts]` 段；
  `--mkv-attachments` 生成可直接嗂给 `mkv remux --attach-font` 的附件目录。
- `mkv identify` 列出轨道与附件；`mkv extract` 抽出字幕轨或字体附件；`mkv remux` 重封装成新 MKV（绝不原地修改）。
- `doctor` 报告环境与未实现能力。
- `cache stat|clear` 报账或清理 37 §2 的磁盘渲染缓存（目录、条数、字节数、配额上限）。

退出码固定为：0 成功、1 执行失败、2 命令行用法错误。未实现的命令以执行失败返回，并在结果中指出能力未实现。

## 3. 输出与脚本调用

- `--json` 将结果写为单行 JSON 信封；stdout 只写结果，诊断写入 stderr。
- `--dry-run` 计算任务并报告预期输出，不落盘。
- `--out PATH` 指定输出路径或报告路径；覆盖策略由 `--overwrite skip|rename|replace` 控制。
- `@FILE` 从 UTF-8 参数文件读取参数，支持脚本和长命令重放。
- 默认覆盖策略为 `rename`；已有文件不应被直接截断。

## 4. 配置与 GUI 重放

[`36_PRESET_CONFIG_SCHEMA.md`](36_PRESET_CONFIG_SCHEMA.md) 是规划中的配置 schema，不代表 CLI 已能读取或写入 preset。GUI 尚未实现；未来 GUI 应复用 CLI 的任务模型，并能导出可重放的命令或任务规格。
