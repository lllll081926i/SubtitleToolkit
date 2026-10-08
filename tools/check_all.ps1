# Thin forwarder: the gate ladder has exactly one implementation (tools/check_all.sh).
# Kept because 28 sec 7 documents a Windows entry point.
# ASCII only: Windows PowerShell 5.1 reads .ps1 as ANSI.
$repo = Split-Path -Parent $PSScriptRoot
$bash = $null
$candidates = @()
$gitCommand = Get-Command 'git.exe' -ErrorAction SilentlyContinue
if ($gitCommand) {
    $gitRoot = Split-Path -Parent (Split-Path -Parent $gitCommand.Source)
    $candidates += Join-Path $gitRoot 'bin\bash.exe'
    $candidates += Join-Path $gitRoot 'usr\bin\bash.exe'
}
$candidates += @('C:\Program Files\Git\bin\bash.exe', 'C:\Program Files\Git\usr\bin\bash.exe', 'bash')
foreach ($c in $candidates) {
    if (Get-Command $c -ErrorAction SilentlyContinue) { $bash = $c; break }
}
if (-not $bash) {
    Write-Host 'bash not found - install Git for Windows or run: bash tools/check_all.sh'
    exit 1
}
Push-Location -LiteralPath $repo
try {
    & $bash 'tools/check_all.sh' @args
    $result = $LASTEXITCODE
} finally {
    Pop-Location
}
exit $result
