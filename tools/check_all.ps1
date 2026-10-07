# Thin forwarder: the gate ladder has exactly one implementation (tools/check_all.sh).
# Kept because 28 sec 7 documents a Windows entry point.
# ASCII only: Windows PowerShell 5.1 reads .ps1 as ANSI.
$repo = Split-Path -Parent $PSScriptRoot
$bash = $null
foreach ($c in @('bash', 'C:\Program Files\Git\bin\bash.exe', 'C:\Program Files\Git\usr\bin\bash.exe')) {
    if (Get-Command $c -ErrorAction SilentlyContinue) { $bash = $c; break }
}
if (-not $bash) {
    Write-Host 'bash not found - install Git for Windows or run: bash tools/check_all.sh'
    exit 1
}
$argLine = ($args -join ' ')
& $bash "tools/check_all.sh" $argLine
exit $LASTEXITCODE
