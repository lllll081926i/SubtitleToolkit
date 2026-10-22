# SubtitleToolkit 安装器打包（28 §3）
#
# 立场（与 make_portable.ps1 同一条）：
# * 默认不签名；使用 -Sign 时，在证书库可用的前提下签名程序文件和 setup.exe。
# * 收进安装器的文件清单与便携包**同一条规则**（同样的程序、运行库和文档），
#   两处各列一份清单迟早会漂移，而"安装版能跑、便携版缺文件"没人会立刻发现。
#
# 用法：
#   . .\tools\set_build_cache.ps1
#   .\tools\make_installer.ps1                 # 构建（release）后打安装器
#   .\tools\make_installer.ps1 -NoBuild        # 只打包已存在的 release 产物
#   .\tools\make_installer.ps1 -OutDir D:\dist
#
# ISCC 的定位顺序：-IsccPath 参数 → PATH → 常见安装位置（含 per-user 安装）。
# 找不到就明确报错并给出获取方式，不静默产出"没有安装器的成功"。

[CmdletBinding()]
param(
    [switch]$NoBuild,
    [switch]$Sign,
    [string]$OutDir,
    # 显式指定 ISCC.exe；不给就按顺序自动找
    [string]$IsccPath,
    # 安装包版本号；默认从 workspace 的 Cargo.toml 读
    [string]$Version
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Push-Location $root
try {
    # --- 1. 定位 ISCC --------------------------------------------------------------
    function Find-Iscc {
        param([string]$Explicit)
        if ($Explicit) {
            if (Test-Path -LiteralPath $Explicit -PathType Leaf) { return $Explicit }
            throw "-IsccPath 指向的文件不存在：$Explicit"
        }
        $cmd = Get-Command iscc -ErrorAction SilentlyContinue
        if ($cmd) { return $cmd.Source }
        # per-user 与 machine-wide 两处；版本目录按降序找最新
        $candidates = @()
        foreach ($base in @("$env:LOCALAPPDATA\Programs", $env:ProgramFiles, ${env:ProgramFiles(x86)})) {
            if (-not $base) { continue }
            if (-not (Test-Path -LiteralPath $base)) { continue }
            $candidates += Get-ChildItem -LiteralPath $base -Directory -ErrorAction SilentlyContinue |
                Where-Object { $_.Name -like 'Inno Setup*' } |
                Sort-Object Name -Descending |
                ForEach-Object { Join-Path $_.FullName 'ISCC.exe' }
        }
        foreach ($c in $candidates) {
            if (Test-Path -LiteralPath $c -PathType Leaf) { return $c }
        }
        return $null
    }

    $iscc = Find-Iscc -Explicit $IsccPath
    if (-not $iscc) {
        throw @"
找不到 Inno Setup 的命令行编译器 ISCC.exe。

安装方式（任一）：
  winget install JRSoftware.InnoSetup
  或从 https://jrsoftware.org/isdl.php 下载安装

也可以用 -IsccPath 显式指定，例如：
  .\tools\make_installer.ps1 -IsccPath 'C:\path\to\ISCC.exe'
"@
    }
    Write-Host "[installer] ISCC: $iscc" -ForegroundColor Cyan

    # --- 2. 版本号（从 Cargo.toml 读，不手抄）--------------------------------------
    if (-not $Version) {
        $metaJson = & cargo metadata --no-deps --format-version 1 2>&1
        if ($LASTEXITCODE -ne 0) { throw "cargo metadata 失败：$metaJson" }
        $meta = $metaJson | ConvertFrom-Json
        $Version = ($meta.packages | Where-Object { $_.name -eq 'cli' } | Select-Object -First 1).version
        if (-not $Version) {
            # 退回根 Cargo.toml 的 workspace.package.version
            $line = Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' |
                Select-Object -First 1
            if ($line) { $Version = $line.Matches[0].Groups[1].Value }
        }
        if (-not $Version) { throw "读不到版本号：传 -Version 显式指定" }
    }
    Write-Host "[installer] 版本: $Version" -ForegroundColor Cyan

    # --- 3. 定位目标目录与构建 -----------------------------------------------------
    $metaJson = & cargo metadata --no-deps --format-version 1 2>&1
    if ($LASTEXITCODE -ne 0) { throw "cargo metadata 失败：$metaJson" }
    $targetDir = ($metaJson | ConvertFrom-Json).target_directory
    $releaseDir = Join-Path $targetDir 'release'

    if (-not $NoBuild) {
        Write-Host "[installer] cargo build --workspace --release" -ForegroundColor Cyan
        & cargo build --workspace --release
        if ($LASTEXITCODE -ne 0) { throw "release 构建失败" }
    }

    # --- 4. 组装 stage（与便携包**同一批文件**）------------------------------------
    $wanted = @(
        'subtitletool-cli.exe',
        'subtitletool-render-host.exe',
        'subtitletoolkit-gui.exe'
    )
    if (-not $OutDir) { $OutDir = Join-Path $targetDir 'installer' }
    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
    $stage = Join-Path $OutDir 'stage'
    if (Test-Path -LiteralPath $stage) {
        # 只清理本脚本自己上次生成的暂存目录
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $stage | Out-Null

    foreach ($exe in $wanted) {
        $p = Join-Path $releaseDir $exe
        if (-not (Test-Path -LiteralPath $p -PathType Leaf)) {
            throw "缺少产物：$p（先不带 -NoBuild 运行一次）"
        }
        Copy-Item -LiteralPath $p -Destination (Join-Path $stage $exe) -Force
    }

    $docs = @('LICENSE-MIT', 'LICENSE-APACHE', 'THIRD_PARTY_NOTICES.md', 'sbom.cdx.json', 'README.md')
    foreach ($d in $docs) {
        $src = Join-Path $root $d
        if (-not (Test-Path -LiteralPath $src -PathType Leaf)) {
            throw "缺少随安装包文档：$d"
        }
        Copy-Item -LiteralPath $src -Destination (Join-Path $stage $d) -Force
    }
    & (Join-Path $PSScriptRoot 'copy_libass_runtime.ps1') -StageDir $stage

    if ($Sign) {
        $signTargets = @('subtitletool-cli.exe', 'subtitletool-render-host.exe', 'subtitletoolkit-gui.exe') |
            ForEach-Object { Join-Path $stage $_ }
        & (Join-Path $PSScriptRoot 'sign_authenticode.ps1') -Path $signTargets
    }

    $sumLines = Get-ChildItem -LiteralPath $stage -File -Recurse |
        Where-Object { $_.Name -ne 'SHA256SUMS.txt' } |
        ForEach-Object {
            $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant()
            $relativePath = [IO.Path]::GetRelativePath($stage, $_.FullName).Replace('\', '/')
            "$hash  $relativePath"
        }
    [System.IO.File]::WriteAllText(
        (Join-Path $stage 'SHA256SUMS.txt'),
        (($sumLines -join "`n") + "`n"),
        [System.Text.UTF8Encoding]::new($false)
    )
    # 刻意**不**放 portable.flag：安装版就该落 %LOCALAPPDATA%（28 §4）。
    # 放进来的话安装版会静默变成便携行为，而用户以为自己装的是正常程序。

    # --- 5. 编译安装器 -------------------------------------------------------------
    $iss = Join-Path $root 'tools\subtitletool.iss'
    if (-not (Test-Path -LiteralPath $iss -PathType Leaf)) { throw "找不到 $iss" }

    $args = @(
        "/DStageDir=$stage",
        "/DAppVersion=$Version",
        "/DOutDir=$OutDir",
        $iss
    )
    Write-Host "[installer] ISCC 编译中…" -ForegroundColor Cyan
    & $iscc @args
    if ($LASTEXITCODE -ne 0) { throw "ISCC 编译失败（退出码 $LASTEXITCODE）" }

    $setup = Join-Path $OutDir "SubtitleToolkit-$Version-setup.exe"
    if (-not (Test-Path -LiteralPath $setup -PathType Leaf)) {
        throw "ISCC 报了成功但产物不在：$setup"
    }
    if ($Sign) {
        & (Join-Path $PSScriptRoot 'sign_authenticode.ps1') -Path $setup
    }
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $setup).Hash.ToLowerInvariant()
    $size = (Get-Item -LiteralPath $setup).Length

    Write-Host ""
    Write-Host "[installer] 完成" -ForegroundColor Green
    Write-Host "  安装器: $setup"
    Write-Host "  大小  : $([math]::Round($size / 1MB, 2)) MiB"
    Write-Host "  SHA-256: $hash"
    Write-Host ""
    Write-Host "  行为（28 §3）：per-user 默认不要求管理员；可在安装页选 all-users；" -ForegroundColor Cyan
    Write-Host "  文件关联默认不勾（opt-in）；legacy 32 位宿主为可选且当前未随包提供；" -ForegroundColor Cyan
    Write-Host "  卸载时由用户选择是否保留 presets/logs。" -ForegroundColor Cyan
    Write-Host ""
    if (-not $Sign) {
        Write-Host "  未完成项：本次未启用 Authenticode 签名；未签名文件可能触发 SmartScreen 警告。" -ForegroundColor Yellow
    }
    Write-Host "  说明：本包包含已核验的 libass DLL、fontconfig 配置及随附许可证材料。" -ForegroundColor Yellow
    Write-Host "  SHA256SUMS.txt 覆盖安装目录中的所有文件。" -ForegroundColor Yellow
}
finally {
    Pop-Location
}
