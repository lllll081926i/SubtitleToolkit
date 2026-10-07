# CLI 与自动化规范

可执行：`subtitletool.exe`（GUI）和 `subtitletool-cli.exe`。

## Commands
- `probe`；
- `convert`；
- `hdr`；
- `time`；
- `render`；
- `pgs adjust|validate|encode`；
- `batch`；
- `font scan|subset`；
- `mkv extract|remux`；
- `doctor`。

## Preset
`--preset foo.toml`；GUI 保存的 preset 与 CLI 完全同一 schema。

## JSON
`--json` 输出稳定 schema；进度走 stderr 或 JSONL，避免污染结果。

## Repro
GUI “复制 CLI 命令”功能可直接重放当前任务；诊断非常重要。
