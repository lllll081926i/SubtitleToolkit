[CmdletBinding()]
param(
    [switch]$NoBuild,
    [switch]$Sign,
    [string]$OutDir,
    [string]$IsccPath,
    [string]$Tag
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    . (Join-Path $PSScriptRoot 'set_build_cache.ps1')
    $json = & cargo metadata --locked --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw '无法读取 Cargo workspace' }
    $meta = $json | ConvertFrom-Json
    $version = ($meta.packages | Where-Object name -eq 'cli').version
    if ($Tag -and $Tag -notmatch ('^v' + [regex]::Escape($version) + '(?:-[0-9A-Za-z.-]+)?$')) {
        throw "标签 $Tag 与 Cargo 版本 $version 不匹配"
    }
    if (-not $OutDir) { $OutDir = Join-Path $meta.target_directory ('release-packages-' + [guid]::NewGuid().ToString('N')) }
    if (Test-Path -LiteralPath $OutDir) { throw "发版目录已存在，请选择新目录以保留旧产物：$OutDir" }
    New-Item -ItemType Directory -Path $OutDir | Out-Null
    $OutDir = (Resolve-Path -LiteralPath $OutDir).Path
    if (-not $NoBuild) {
        & cargo build --locked --workspace --release
        if ($LASTEXITCODE -ne 0) { throw 'Release 构建失败' }
    }
    & (Join-Path $PSScriptRoot 'make_portable.ps1') -NoBuild -Sign:$Sign -OutDir (Join-Path $OutDir 'portable')
    & (Join-Path $PSScriptRoot 'make_installer.ps1') -NoBuild -Sign:$Sign -OutDir (Join-Path $OutDir 'installer') -IsccPath $IsccPath -Version $version
    $portable = Get-ChildItem (Join-Path $OutDir 'portable') -Filter '*.zip' | Select-Object -First 1
    $unpacked = Join-Path $OutDir 'unpacked'
    Expand-Archive -LiteralPath $portable.FullName -DestinationPath $unpacked
    & (Join-Path $PSScriptRoot 'test_distribution.ps1') -PackageDir $unpacked
    $assets = Join-Path $OutDir 'assets'
    New-Item -ItemType Directory -Path $assets | Out-Null
    Copy-Item -LiteralPath $portable.FullName -Destination (Join-Path $assets "SubtitleToolkit-$version-windows-x64-portable.zip")
    Copy-Item -LiteralPath (Join-Path $OutDir "installer\SubtitleToolkit-$version-setup.exe") -Destination $assets
    $commit = & git rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw '无法读取发布提交' }
    $manifest = @{ version = $version; tag = $Tag; commit = $commit; signed = [bool]$Sign; target = 'x86_64-pc-windows-msvc' }
    $manifest | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $assets 'release-manifest.json') -Encoding utf8
    $lines = Get-ChildItem -LiteralPath $assets -File | Sort-Object Name | ForEach-Object {
        '{0}  {1}' -f (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant(), $_.Name
    }
    [IO.File]::WriteAllText((Join-Path $assets 'SHA256SUMS.txt'), ($lines -join "`n") + "`n", [Text.UTF8Encoding]::new($false))
    Write-Host "[release] 已验证的发布资产：$assets" -ForegroundColor Green
}
finally { Pop-Location }
