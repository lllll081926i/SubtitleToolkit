# SubtitleToolkit portable 打包（28 §2 Artifacts、28 §4 Portable）
#
# 立场（写清楚，避免把"能打 zip"当成"已发布"）：
# * 本脚本只把**已经构建好并通过验收**的产物收进一个自包含目录，再打成 zip；
#   它不做签名、不做安装器、不动注册表（那些是 28 §3/§5，尚未实现）。
# * 随包提供已逐包核验来源、哈希和许可材料的 libass UCRT64 运行库。
# * 打包前重新验证内容哈希，并生成 SHA256SUMS.txt；第三方声明原样复制进包。
#
# 用法：
#   . .\tools\set_build_cache.ps1
#   .\tools\make_portable.ps1                 # 构建（release）后打包到 target/portable/
#   .\tools\make_portable.ps1 -NoBuild        # 只打包已存在的 release 产物
#   .\tools\make_portable.ps1 -OutDir D:\dist # 指定输出目录

[CmdletBinding()]
param(
    # 跳过 cargo build，直接用现有 release 产物（产物不存在时仍然报错，不静默产出空包）
    [switch]$NoBuild,
    # 输出目录；默认落在 Cargo 目标目录下的 portable/
    [string]$OutDir,
    # 不含版本号的包名前缀
    [string]$Name = "subtitletool"
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Push-Location $root
try {
    # --- 1. 定位 Cargo 目标目录（不猜路径：以 cargo metadata 为准）-----------------
    $metaJson = & cargo metadata --no-deps --format-version 1 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata 失败，无法确定目标目录：$metaJson"
    }
    $meta = $metaJson | ConvertFrom-Json
    $targetDir = $meta.target_directory
    $releaseDir = Join-Path $targetDir 'release'

    # --- 2. 可选构建 ---------------------------------------------------------------
    if (-not $NoBuild) {
        Write-Host "[portable] cargo build --workspace --release" -ForegroundColor Cyan
        & cargo build --workspace --release
        if ($LASTEXITCODE -ne 0) { throw "release 构建失败" }
    }

    # --- 3. 收集产物（缺一个就失败，不产出"看起来像发布包"的残缺 zip）----------------
    # 收集 CLI、renderer-host 与已接通真实 batch 执行链的 GUI。
    $wanted = @(
        'subtitletool-cli.exe',
        'subtitletool-render-host.exe',
        'subtitletoolkit-gui.exe'
    )
    $payload = @()
    foreach ($exe in $wanted) {
        $p = Join-Path $releaseDir $exe
        if (-not (Test-Path -LiteralPath $p -PathType Leaf)) {
            throw "缺少产物：$p（先不带 -NoBuild 运行一次）"
        }
        $payload += , @{ Src = $p; Name = $exe }
    }

    # --- 4. 组装目录 ---------------------------------------------------------------
    $stamp = Get-Date -Format 'yyyyMMdd'
    $pkgName = "$Name-portable-$stamp"
    if (-not $OutDir) { $OutDir = Join-Path $targetDir 'portable' }
    $stage = Join-Path $OutDir $pkgName
    if (Test-Path -LiteralPath $stage) {
        # 只清理本脚本自己上次生成的暂存目录（同一个 $stage 路径），不动别的资产
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $stage | Out-Null

    foreach ($item in $payload) {
        Copy-Item -LiteralPath $item.Src -Destination (Join-Path $stage $item.Name) -Force
    }

    # 随包文档：许可正文、第三方声明与 SBOM（21 §11、28 §2）
    $docs = @(
        @{ Rel = 'LICENSE-MIT';      Name = 'LICENSE-MIT' },
        @{ Rel = 'LICENSE-APACHE';   Name = 'LICENSE-APACHE' },
        @{ Rel = 'THIRD_PARTY_NOTICES.md'; Name = 'THIRD_PARTY_NOTICES.md' },
        # 28 §2 把 checksums + SBOM + third-party notices 列为同一组交付物。
        # SBOM 必须跟着包走：它的用途就是让人在拿到包的地方核对依赖。
        @{ Rel = 'sbom.cdx.json';    Name = 'sbom.cdx.json' },
        @{ Rel = 'README.md';        Name = 'README.md' }
    )
    foreach ($d in $docs) {
        $src = Join-Path $root $d.Rel
        if (-not (Test-Path -LiteralPath $src -PathType Leaf)) {
            throw "缺少随包文档：$($d.Rel)"
        }
        Copy-Item -LiteralPath $src -Destination (Join-Path $stage $d.Name) -Force
    }

    # Runtime loader only accepts the colocated DLL when this marker is present.
    & (Join-Path $PSScriptRoot 'copy_libass_runtime.ps1') -StageDir $stage

    # 28 §4：便携包默认就带上 portable.flag，让解包即用、不往用户机器上的
    # %LOCALAPPDATA% 写任何东西。数据落在同目录的 data/（core-model::DataDir 负责解析）。
    # 删掉这个文件就会退回安装版行为——这是用户的显式选择，不是我们替他决定的事。
    $flagPath = Join-Path $stage 'portable.flag'
    $flagText = @"
# SubtitleToolkit 便携模式标记（28 §4）
#
# 这个文件存在时，程序把数据写成同级目录下的 data/：
# 字体库缓存（fontdb-v1.json）等运行状态都落在包里，不碰用户机器上的 %%LOCALAPPDATA%%。
#
# 想改用安装版行为，删掉本文件即可。
# 想指定一个完全自定义的位置，设环境变量 STK_DATA_DIR（优先级高于本文件）。
"@
    [System.IO.File]::WriteAllText($flagPath, $flagText, [System.Text.UTF8Encoding]::new($false))

    # --- 5. 校验和 ---------------------------------------------------------------
    # 覆盖**包内所有文件**，不只是 exe：许可正文、第三方声明与 portable.flag
    # 都属于"这东西是不是我要的那一份"的一部分，漏了它们就等于校验和无法证明整包完整。
    $sumsPath = Join-Path $stage 'SHA256SUMS.txt'
    $sumLines = Get-ChildItem -LiteralPath $stage -File -Recurse |
        Where-Object { $_.Name -ne 'SHA256SUMS.txt' } |
        ForEach-Object {
            $h = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant()
            $relativePath = [IO.Path]::GetRelativePath($stage, $_.FullName).Replace('\', '/')
            "$h  $relativePath"
        }
    # 用 LF 与 ASCII，让校验和文件在跨平台工具里也能直接用
    [System.IO.File]::WriteAllText($sumsPath, (($sumLines -join "`n") + "`n"), [System.Text.UTF8Encoding]::new($false))

    # --- 6. 打 zip ---------------------------------------------------------------
    $zipPath = Join-Path $OutDir "$pkgName.zip"
    if (Test-Path -LiteralPath $zipPath) { Remove-Item -LiteralPath $zipPath -Force }
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zipPath -CompressionLevel Optimal

    $zipHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $zipPath).Hash.ToLowerInvariant()
    Write-Host ""
    Write-Host "[portable] 完成" -ForegroundColor Green
    Write-Host "  目录: $stage"
    Write-Host "  zip : $zipPath"
    Write-Host "  zip SHA-256: $zipHash"
    Write-Host ""
    Write-Host "  包内文件：" -ForegroundColor Cyan
    Get-ChildItem -LiteralPath $stage -File -Recurse | ForEach-Object {
        Write-Host "    $([IO.Path]::GetRelativePath($stage, $_.FullName).Replace('\', '/'))"
    }
    Write-Host ""
    Write-Host "  说明：本包包含已核验的 libass DLL、fontconfig 配置及随附许可证材料。" -ForegroundColor Yellow
    Write-Host "  便携：包内自带 portable.flag，数据（字体库缓存等）落在同级 data/，不写 %LOCALAPPDATA%。" -ForegroundColor Yellow
    Write-Host "  未完成项：Authenticode 签名见 28 节；运行库来源和哈希见包内 licenses/LIBASS_RUNTIME_LOCK.json。" -ForegroundColor Yellow
}
finally {
    Pop-Location
}
