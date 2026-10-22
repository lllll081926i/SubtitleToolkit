[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$StageDir
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$thirdParty = Join-Path $root 'third_party'
$lockPath = Join-Path $thirdParty 'LIBASS_RUNTIME_LOCK.json'
$lock = Get-Content -LiteralPath $lockPath -Raw | ConvertFrom-Json

if ($lock.artifacts.Count -ne 20) {
    throw "运行库锁应包含 20 个 DLL，实际为 $($lock.artifacts.Count)"
}

foreach ($artifact in $lock.artifacts) {
    $relativePath = $artifact.path -replace '^third_party[\\/]+', ''
    $source = Join-Path $thirdParty $relativePath
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "缺少锁定的 libass 依赖：$source"
    }
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $source).Hash.ToLowerInvariant()
    if ($actual -ne $artifact.sha256) {
        throw "libass DLL 哈希不匹配：$($artifact.name)；锁定 $($artifact.sha256)，实测 $actual"
    }
    Copy-Item -LiteralPath $source -Destination (Join-Path $StageDir $artifact.name) -Force
}

foreach ($supportFile in $lock.support_files) {
    $source = Join-Path $thirdParty $supportFile.path
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "缺少锁定的 fontconfig 配置：$source"
    }
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $source).Hash.ToLowerInvariant()
    if ($actual -ne $supportFile.sha256) {
        throw "fontconfig 文件哈希不匹配：$($supportFile.path)"
    }
    $destination = Join-Path $StageDir $supportFile.path
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
    Copy-Item -LiteralPath $source -Destination $destination -Force
}

$licenseRoot = Join-Path $StageDir 'licenses\msys2'
New-Item -ItemType Directory -Force -Path $licenseRoot | Out-Null
foreach ($package in $lock.license_packages) {
    $source = Join-Path $thirdParty "licenses\msys2\$package"
    if (-not (Test-Path -LiteralPath $source -PathType Container)) {
        throw "缺少第三方包许可证与元数据：$source"
    }
    Copy-Item -LiteralPath $source -Destination (Join-Path $licenseRoot $package) -Recurse -Force
}

Copy-Item -LiteralPath $lockPath -Destination (Join-Path $StageDir 'licenses\LIBASS_RUNTIME_LOCK.json') -Force
$marker = Join-Path $StageDir 'libass-runtime.flag'
[System.IO.File]::WriteAllText(
    $marker,
    "SubtitleToolkit verified bundled libass runtime`n",
    [System.Text.UTF8Encoding]::new($false)
)

Write-Host "[runtime] 已核验并复制 $($lock.artifacts.Count) 个 DLL、$($lock.support_files.Count) 个字体配置文件及 $($lock.license_packages.Count) 组许可证材料。" -ForegroundColor Green
