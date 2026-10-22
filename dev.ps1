[CmdletBinding()]
param(
    [ValidateSet('off', 'error', 'warn', 'info', 'debug', 'trace')]
    [string]$LogLevel = 'debug',
    [string]$LibassRuntimeDir
)

$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot
. (Join-Path $PSScriptRoot 'tools\set_build_cache.ps1')

if ([string]::IsNullOrWhiteSpace($env:STK_DATA_DIR)) {
    $env:STK_DATA_DIR = Join-Path $PSScriptRoot 'target\dev-data'
}
$env:STK_LOG = $LogLevel

if (-not [string]::IsNullOrWhiteSpace($LibassRuntimeDir)) {
    $runtimeDir = (Resolve-Path -LiteralPath $LibassRuntimeDir -ErrorAction Stop).Path
    $libassDll = Join-Path $runtimeDir 'libass-9.dll'
    $fontconfigDir = Join-Path $runtimeDir 'etc\fonts'
    if (-not (Test-Path -LiteralPath $libassDll -PathType Leaf)) {
        throw "运行库目录中缺少 libass-9.dll：$runtimeDir"
    }
    if (-not (Test-Path -LiteralPath $fontconfigDir -PathType Container)) {
        throw "运行库目录中缺少 etc\fonts：$runtimeDir"
    }
    $env:STK_LIBASS_DLL = $libassDll
    $env:FONTCONFIG_PATH = $fontconfigDir
    $env:PATH = "$runtimeDir;$env:PATH"
}

New-Item -ItemType Directory -Force -Path $env:STK_DATA_DIR | Out-Null
$logDir = Join-Path $env:STK_DATA_DIR 'logs'

Write-Host ''
Write-Host '启动 SubtitleToolkit GUI 开发模式（Cargo Debug）' -ForegroundColor Cyan
Write-Host "日志级别：$LogLevel"
Write-Host "日志目录：$logDir"
if ($env:STK_LIBASS_DLL) {
    Write-Host "libass：$env:STK_LIBASS_DLL"
}
Write-Host '关闭窗口或按 Ctrl+C 可停止。'
Write-Host ''

cargo run -p gui
$exitCode = $LASTEXITCODE

Write-Host ''
Write-Host "GUI 已退出，退出码：$exitCode"
if (Test-Path -LiteralPath $logDir) {
    Write-Host "日志文件：$logDir"
}
exit $exitCode
