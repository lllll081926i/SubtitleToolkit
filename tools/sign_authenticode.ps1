[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string[]]$Path,
    [string]$Thumbprint = $env:STK_SIGNING_CERT_THUMBPRINT,
    [string]$TimestampUrl = 'http://timestamp.digicert.com'
)

$ErrorActionPreference = 'Stop'

if (-not $Thumbprint) {
    throw '缺少签名证书指纹。请将证书导入当前用户的 Personal\My 证书存储，并设置 STK_SIGNING_CERT_THUMBPRINT。'
}
$Thumbprint = $Thumbprint -replace '\s', ''

$cert = Get-ChildItem Cert:\CurrentUser\My -ErrorAction SilentlyContinue |
    Where-Object { $_.Thumbprint -eq $Thumbprint } |
    Select-Object -First 1
if (-not $cert) { throw "当前用户 Personal\My 中找不到证书：$Thumbprint" }
if (-not $cert.HasPrivateKey) { throw '签名证书没有可用的私钥。' }
if ($cert.NotAfter -le (Get-Date)) { throw "签名证书已过期：$($cert.NotAfter.ToString('u'))" }
$hasCodeSigningEku = $cert.EnhancedKeyUsageList | Where-Object { $_.ObjectId.Value -eq '1.3.6.1.5.5.7.3.3' }
if (-not $hasCodeSigningEku) { throw '证书缺少 Code Signing EKU。' }

$signtool = Get-ChildItem (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin') -Filter signtool.exe -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.Directory.Name -eq 'x64' } |
    Sort-Object FullName -Descending |
    Select-Object -First 1 -ExpandProperty FullName
if (-not $signtool) { throw '找不到 Windows SDK x64\signtool.exe。请安装 Windows 10/11 SDK。' }

foreach ($file in $Path) {
    $resolved = (Resolve-Path -LiteralPath $file -ErrorAction Stop).Path
    if ((Get-Item -LiteralPath $resolved).PSIsContainer) { throw "签名目标必须是文件：$resolved" }

    Write-Host "[sign] 签名：$resolved" -ForegroundColor Cyan
    & $signtool sign /v /sha1 $Thumbprint /s My /fd SHA256 /tr $TimestampUrl /td SHA256 $resolved
    if ($LASTEXITCODE -ne 0) { throw "signtool sign 失败（退出码 $LASTEXITCODE）：$resolved" }

    & $signtool verify /pa /v $resolved
    if ($LASTEXITCODE -ne 0) { throw "signtool verify 失败（退出码 $LASTEXITCODE）：$resolved" }
}
