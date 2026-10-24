[CmdletBinding()]
param([string]$Destination)
$ErrorActionPreference = 'Stop'
if (-not $Destination) { $Destination = Join-Path (Split-Path -Parent $PSScriptRoot) 'target\tools\innosetup-6.7.3' }
$compiler = Join-Path $Destination 'ISCC.exe'
if (-not (Test-Path -LiteralPath $compiler)) {
    $download = Join-Path ([IO.Path]::GetTempPath()) ('innosetup-' + [guid]::NewGuid().ToString('N') + '.exe')
    Invoke-WebRequest 'https://github.com/jrsoftware/issrc/releases/download/is-6_7_3/innosetup-6.7.3.exe' -OutFile $download
    $expected = '9c73c3bae7ed48d44112a0f48e66742c00090bdb5bef71d9d3c056c66e97b732'
    if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) {
        throw 'Inno Setup 下载文件 SHA-256 不匹配'
    }
    $process = Start-Process -FilePath $download -WindowStyle Hidden -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', '/NOICONS', ('/DIR="' + $Destination + '"')) -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Inno Setup 安装失败：$($process.ExitCode)" }
}
if (-not (Test-Path -LiteralPath $compiler)) { throw "找不到 Inno Setup 编译器：$compiler" }
Write-Output $compiler
