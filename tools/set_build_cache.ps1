param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$defaultCargoHome = Join-Path $repositoryRoot '.cargo-home'
$defaultTargetDir = Join-Path $repositoryRoot 'target'

function Test-CDrivePath([string]$PathValue) {
    if ([string]::IsNullOrWhiteSpace($PathValue)) {
        return $false
    }
    try {
        $fullPath = [System.IO.Path]::GetFullPath($PathValue)
        return [System.IO.Path]::GetPathRoot($fullPath).Equals('C:\', [System.StringComparison]::OrdinalIgnoreCase)
    }
    catch {
        return $true
    }
}

if ([string]::IsNullOrWhiteSpace($env:CARGO_HOME) -or (Test-CDrivePath $env:CARGO_HOME)) {
    $env:CARGO_HOME = $defaultCargoHome
}
if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR) -or (Test-CDrivePath $env:CARGO_TARGET_DIR)) {
    $env:CARGO_TARGET_DIR = $defaultTargetDir
}
$defaultCacheDir = Join-Path $env:CARGO_TARGET_DIR 'cache'
$defaultVcpkgDownloads = Join-Path $env:CARGO_TARGET_DIR 'vcpkg-downloads'
if ([string]::IsNullOrWhiteSpace($env:VCPKG_DOWNLOADS) -or (Test-CDrivePath $env:VCPKG_DOWNLOADS)) {
    $env:VCPKG_DOWNLOADS = $defaultVcpkgDownloads
}
$defaultVcpkgBinaryCache = Join-Path $defaultCacheDir 'vcpkg-binary'
if ([string]::IsNullOrWhiteSpace($env:VCPKG_DEFAULT_BINARY_CACHE) -or (Test-CDrivePath $env:VCPKG_DEFAULT_BINARY_CACHE)) {
    $env:VCPKG_DEFAULT_BINARY_CACHE = $defaultVcpkgBinaryCache
}

foreach ($cacheSetting in @(
    @{ Name = 'SCCACHE_DIR'; Path = (Join-Path $defaultCacheDir 'sccache') },
    @{ Name = 'CCACHE_DIR'; Path = (Join-Path $defaultCacheDir 'ccache') },
    @{ Name = 'PIP_CACHE_DIR'; Path = (Join-Path $defaultCacheDir 'pip') }
)) {
    $currentValue = [Environment]::GetEnvironmentVariable($cacheSetting.Name)
    if ([string]::IsNullOrWhiteSpace($currentValue) -or (Test-CDrivePath $currentValue)) {
        [Environment]::SetEnvironmentVariable($cacheSetting.Name, $cacheSetting.Path, 'Process')
    }
}

$temporaryDirectory = Join-Path $repositoryRoot 'target\tmp'
if ([string]::IsNullOrWhiteSpace($env:TEMP) -or (Test-CDrivePath $env:TEMP)) {
    $env:TEMP = $temporaryDirectory
}
if ([string]::IsNullOrWhiteSpace($env:TMP) -or (Test-CDrivePath $env:TMP)) {
    $env:TMP = $temporaryDirectory
}
if ([string]::IsNullOrWhiteSpace($env:TMPDIR) -or (Test-CDrivePath $env:TMPDIR)) {
    $env:TMPDIR = $temporaryDirectory
}

New-Item -ItemType Directory -Force -Path $env:CARGO_HOME, $env:CARGO_TARGET_DIR, $env:VCPKG_DOWNLOADS, $env:VCPKG_DEFAULT_BINARY_CACHE, $env:SCCACHE_DIR, $env:CCACHE_DIR, $env:PIP_CACHE_DIR, $temporaryDirectory | Out-Null
Write-Output "CARGO_HOME=$env:CARGO_HOME"
Write-Output "CARGO_TARGET_DIR=$env:CARGO_TARGET_DIR"
Write-Output "VCPKG_DOWNLOADS=$env:VCPKG_DOWNLOADS"
Write-Output "VCPKG_DEFAULT_BINARY_CACHE=$env:VCPKG_DEFAULT_BINARY_CACHE"
Write-Output "SCCACHE_DIR=$env:SCCACHE_DIR"
Write-Output "CCACHE_DIR=$env:CCACHE_DIR"
Write-Output "PIP_CACHE_DIR=$env:PIP_CACHE_DIR"
Write-Output "TEMP=$env:TEMP"
Write-Output "TMP=$env:TMP"
