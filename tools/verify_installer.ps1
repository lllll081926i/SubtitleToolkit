# 安装器的行为验证（28 §3 的五条要求）
#
# 立场：**能编译出 setup.exe 不等于它能装**。本脚本把安装器真的跑一遍，
# 逐条核对 28 §3 的五条要求，并在结束时报告通过/失败。
#
# 为什么不写成 Rust 集成测试：安装/卸载要起真进程、写 HKCU 与 %LOCALAPPDATA%，
# 而且必须**静默**跑（/VERYSILENT）。Rust 测试跑这个会污染开发机的真实注册表与
# 用户目录，而 CI 上又没有装 Inno Setup。一个显式的、开发者手动跑的脚本
# 比一条会留下副作用的"自动化测试"诚实。
#
# 用法：
#   .\tools\verify_installer.ps1                      # 用默认输出的安装器
#   .\tools\verify_installer.ps1 -Setup PATH\setup.exe
#
# 全程只写这些位置，并**在结束时清理**：
#   - 安装目录：target/tmp/installer-verify-<随机>
#   - HKCU 的 Classes\ 下与 AppId 键（删掉自己写的那几个）
#   - %LOCALAPPDATA%\subtitletool（**备份并恢复**，因为用户可能真在用）

[CmdletBinding()]
param(
    [string]$Setup
)

$ErrorActionPreference = 'Stop'

if ($env:GITHUB_ACTIONS -ne 'true') {
    throw '安装器行为验证会改写测试账户的关联与用户数据，只允许在 GitHub Actions 临时 Windows runner 中运行。'
}

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$appId = '{7E3A9C41-5B2D-4F18-9A6E-1C0D8B7F4A52}'
$dataDir = Join-Path $env:LOCALAPPDATA 'subtitletool'
# iss 里的 `#define AppId "{{7E3A…}}"` 外层双花括号是 **Inno [Setup] 的转义约定**
# （代表 AppId 的字面值），而 Pascal 里用的是 `{#AppIdRaw}` = 单花括号。
# 所以注册表里的键就是单花括号的那个 —— 验证脚本必须也按单花括号找。
$appKey = "HKCU:\Software\$appId"

if (-not $Setup) {
    # 默认取 target/installer 下最新的 setup.exe
    $metaJson = & cargo metadata --no-deps --format-version 1 2>&1
    if ($LASTEXITCODE -ne 0) { throw "cargo metadata 失败" }
    $targetDir = ($metaJson | ConvertFrom-Json).target_directory
    $cand = Get-ChildItem (Join-Path $targetDir 'installer') -Filter '*-setup.exe' -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $cand) { throw "找不到 setup.exe：先跑 .\tools\make_installer.ps1" }
    $Setup = $cand.FullName
}
if (-not (Test-Path -LiteralPath $Setup -PathType Leaf)) { throw "找不到 $Setup" }

$results = [System.Collections.Generic.List[object]]::new()
function Check {
    param([string]$What, [bool]$Ok, [string]$Detail = '')
    $results.Add([pscustomobject]@{ What = $What; Ok = $Ok; Detail = $Detail })
    $mark = if ($Ok) { 'PASS' } else { 'FAIL' }
    $color = if ($Ok) { 'Green' } else { 'Red' }
    Write-Host ("  [{0}] {1}{2}" -f $mark, $What, $(if ($Detail) { "  — $Detail" } else { '' })) -ForegroundColor $color
}

# --- 备份真实的用户数据目录（验证完恢复），别毁掉开发机上真在用的数据 -------------
$backup = $null
if (Test-Path -LiteralPath $dataDir) {
    $backup = Join-Path ([System.IO.Path]::GetTempPath()) ("stk-data-backup-" + [guid]::NewGuid().ToString('N'))
    Copy-Item -LiteralPath $dataDir -Destination $backup -Recurse -Force
    Write-Host "[verify] 已备份现有 $dataDir -> $backup" -ForegroundColor Yellow
    Remove-Item -LiteralPath $dataDir -Recurse -Force
}

$dest = Join-Path $root ("target\tmp\installer-verify-" + [guid]::NewGuid().ToString('N').Substring(0, 8))

function Cleanup-Assoc {
    foreach ($ext in @('.ass', '.ssa', '.srt', '.sup', '.xml')) {
        Remove-Item -LiteralPath "HKCU:\Software\Classes\$ext" -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath "HKCU:\Software\Classes\SubtitleToolkit$ext" -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# 清掉可能残留的旧路径（双花括号那个）：早期版本的 iss 把它写到了那里。
function Cleanup-LegacyAppKey {
    Remove-Item -LiteralPath 'HKCU:\Software\{{7E3A9C41-5B2D-4F18-9A6E-1C0D8B7F4A52}' -Recurse -Force -ErrorAction SilentlyContinue
}

try {
    Write-Host ""
    Write-Host "[verify] 安装器: $Setup" -ForegroundColor Cyan
    Write-Host "[verify] 安装目录: $dest" -ForegroundColor Cyan
    Write-Host ""

    # ---------------------------------------------------------------------------
    Write-Host "== 第一条：per-user 默认，不要求管理员 ==" -ForegroundColor Cyan
    Cleanup-Assoc
    Remove-Item -LiteralPath $dest -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $dest | Out-Null
    # 不给 /DIR 而用 /DIR 指定临时目录：默认路径会写到真实的 %LOCALAPPDATA%\Programs。
    # 静默安装不给 UAC 提示，所以"退出码 0"本身就说明它没要求提权。
    $p = Start-Process -FilePath $Setup -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/DIR=$dest" -Wait -PassThru
    Check "静默安装退出码 0（未触发 UAC）" ($p.ExitCode -eq 0) "exit=$($p.ExitCode)"

    $exes = Get-ChildItem -LiteralPath $dest -Filter '*.exe' -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -ne 'unins000.exe' }
    Check "三个可执行文件都已安装" ($exes.Count -eq 3) (($exes.Name | Sort-Object) -join ', ')

    # 关联项可能被上一次崩掉的验证脚本留下：装之前先清，
    # 否则“默认不写关联”这条会在残留上假 PASS。
    & (Join-Path $PSScriptRoot 'test_distribution.ps1') -PackageDir $dest

    # ---------------------------------------------------------------------------
    Write-Host "== 第二条：可选 all-users ==" -ForegroundColor Cyan
    # 这一条的可核对形式是安装器**声明**了可选提权（PrivilegesRequiredOverridesAllowed）。
    # 真跑 all-users 需要 UAC 交互，静默环境做不到 —— 不假装验证了它。
    $issText = Get-Content (Join-Path $root 'tools\subtitletool.iss') -Raw
    Check "声明了 PrivilegesRequired=lowest + 可覆盖" `
        ($issText -match 'PrivilegesRequired=lowest' -and $issText -match 'PrivilegesRequiredOverridesAllowed') `
        "静态声明，未做 UAC 实机验证（静默环境无法应答 UAC）"

    # ---------------------------------------------------------------------------
    Write-Host "== 第三条：文件关联默认不抢（opt-in） ==" -ForegroundColor Cyan
    Check ".ass 关联未被默认写入" (-not (Test-Path 'HKCU:\Software\Classes\.ass'))
    Check ".srt 关联未被默认写入" (-not (Test-Path 'HKCU:\Software\Classes\.srt'))

    # ---------------------------------------------------------------------------
    Write-Host "== 第四条：legacy 32 位宿主是可选 ==" -ForegroundColor Cyan
    Check "默认未装 x86 宿主" (-not (Test-Path (Join-Path $dest 'subtitletool-render-host-x86.exe')))
    $raw = Get-Content (Join-Path $root 'tools\subtitletool.iss') -Raw
    Check "iss 里有 legacy 任务项且默不勾" ($raw -match 'Name: "legacy"' -and $raw -match '(?s)Name: "legacy".*?Flags: unchecked')

    # ---------------------------------------------------------------------------
    Write-Host "== 第五条：卸载保留用户数据由用户选择 ==" -ForegroundColor Cyan
    # 5a：默认（保留）
    New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
    Set-Content -LiteralPath (Join-Path $dataDir 'fontdb-v1.json') -Value '{}'
    $kept = (Get-ItemProperty -LiteralPath $appKey -Name KeepUserDataOnUninstall -ErrorAction SilentlyContinue).KeepUserDataOnUninstall
    Check "安装器记下了保留选择" ($kept -eq '1') "KeepUserDataOnUninstall=$kept"
    $p = Start-Process -FilePath (Join-Path $dest 'unins000.exe') -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART" -Wait -PassThru
    Start-Sleep -Seconds 2
    Check "默认卸载后用户数据仍在" (Test-Path -LiteralPath $dataDir)
    Check "卸载器清掉了自己的注册表值" (-not (Test-Path -LiteralPath $appKey))

    # 5b：显式选「删除」
    Remove-Item -LiteralPath $dataDir -Recurse -Force -ErrorAction SilentlyContinue
    $p = Start-Process -FilePath $Setup -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/DIR=$dest" -Wait -PassThru
    New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
    Set-Content -LiteralPath (Join-Path $dataDir 'fontdb-v1.json') -Value '{}'
    # 模仿用户在安装页选了第二项。键可能在也可能不在，两种都用 -Force。
    if (-not (Test-Path -LiteralPath $appKey)) { New-Item -Path $appKey -Force | Out-Null }
    Set-ItemProperty -LiteralPath $appKey -Name KeepUserDataOnUninstall -Value '0'
    $p = Start-Process -FilePath (Join-Path $dest 'unins000.exe') -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART" -Wait -PassThru
    Start-Sleep -Seconds 2
    Check "选「删除」后用户数据被清掉" (-not (Test-Path -LiteralPath $dataDir))
}
finally {
    # --- 清理：安装目录、关联键、AppId 键 ---
    if (Test-Path -LiteralPath $dest) {
        # 可能还装着（前面的检查失败时）：先卸
        $un = Join-Path $dest 'unins000.exe'
        if (Test-Path -LiteralPath $un) {
            Start-Process -FilePath $un -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART" -Wait -ErrorAction SilentlyContinue
            Start-Sleep -Seconds 2
        }
        Remove-Item -LiteralPath $dest -Recurse -Force -ErrorAction SilentlyContinue
    }
    Cleanup-Assoc
    Cleanup-LegacyAppKey
    Remove-Item -LiteralPath $appKey -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $dataDir -Recurse -Force -ErrorAction SilentlyContinue
    if ($backup -and (Test-Path -LiteralPath $backup)) {
        Copy-Item -LiteralPath $backup -Destination $dataDir -Recurse -Force
        Remove-Item -LiteralPath $backup -Recurse -Force
        Write-Host "[verify] 已恢复原有的 $dataDir" -ForegroundColor Yellow
    }
}

Write-Host ""
$failed = @($results | Where-Object { -not $_.Ok })
Write-Host ("[verify] {0} 项检查，{1} 通过，{2} 失败" -f $results.Count, ($results.Count - $failed.Count), $failed.Count) `
    -ForegroundColor $(if ($failed.Count) { 'Red' } else { 'Green' })
if ($failed.Count) {
    $failed | ForEach-Object { Write-Host "  FAIL: $($_.What)  $($_.Detail)" -ForegroundColor Red }
    exit 1
}
exit 0
