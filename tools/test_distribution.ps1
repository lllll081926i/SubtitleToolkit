[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$PackageDir)
$ErrorActionPreference = 'Stop'
$package = (Resolve-Path -LiteralPath $PackageDir).Path
$scratch = Join-Path ([IO.Path]::GetTempPath()) ('stk-package-check-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $scratch | Out-Null
$saved = @{}
foreach ($key in @('STK_DATA_DIR', 'STK_LIBASS_DLL', 'STK_RENDERER_HOST', 'STK_FONTS_DIR', 'FONTCONFIG_PATH', 'FONTCONFIG_FILE')) {
    $saved[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
    Set-Item -LiteralPath ('Env:' + $key) -Value $null
}
try {
    $env:STK_DATA_DIR = Join-Path $scratch 'data'
    foreach ($line in Get-Content -LiteralPath (Join-Path $package 'SHA256SUMS.txt')) {
        if ($line -notmatch '^([0-9a-f]{64})  (.+)$') { throw "校验和格式错误：$line" }
        $expected = $Matches[1]
        $relative = $Matches[2]
        $file = [IO.Path]::GetFullPath((Join-Path $package $relative))
        if (-not $file.StartsWith($package + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
            throw "校验和路径超出软件包：$relative"
        }
        if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) {
            throw "软件包校验失败：$relative"
        }
    }
    foreach ($required in @('subtitletool-cli.exe', 'subtitletoolkit-gui.exe', 'subtitletool-render-host.exe', 'libass-runtime.flag', 'licenses/LIBASS_RUNTIME_LOCK.json', 'THIRD_PARTY_NOTICES.md', 'sbom.cdx.json')) {
        if (-not (Test-Path -LiteralPath (Join-Path $package $required) -PathType Leaf)) { throw "发布包缺少必需文件：$required" }
    }
    $runtime = Get-Content (Join-Path $package 'licenses/LIBASS_RUNTIME_LOCK.json') -Raw | ConvertFrom-Json
    foreach ($artifact in $runtime.artifacts) {
        $dll = Join-Path $package $artifact.name
        if ((Get-FileHash -LiteralPath $dll -Algorithm SHA256).Hash.ToLowerInvariant() -ne $artifact.sha256) {
            throw "发布包运行库与锁不一致：$($artifact.name)"
        }
    }
    $cli = Join-Path $package 'subtitletool-cli.exe'
    $doctor = & $cli doctor --json
    if ($LASTEXITCODE -ne 0) { throw "软件包 doctor 失败：$doctor" }
    $report = ($doctor | ConvertFrom-Json).data
    if (-not $report.render_host.available) { throw "软件包渲染宿主不可用：$doctor" }
    $ass = Join-Path $scratch 'sample.ass'
    $text = @'
[Script Info]
ScriptType: v4.00+
PlayResX: 1920
PlayResY: 1080
[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Arial,64,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,0,2,20,20,60,1
[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,SubtitleToolkit
'@
    [IO.File]::WriteAllText($ass, $text, [Text.UTF8Encoding]::new($false))
    & $cli render $ass --out (Join-Path $scratch 'bdn\sample.xml') --json
    if ($LASTEXITCODE -ne 0) { throw '包内 ASS → BDN 渲染失败' }
    if (-not (Get-ChildItem (Join-Path $scratch 'bdn') -Recurse -Filter '*.png')) { throw 'BDN 没有生成 PNG' }
    $sup = Join-Path $scratch 'sample.sup'
    & $cli pgs encode $ass --out $sup --json
    if ($LASTEXITCODE -ne 0) { throw '包内 ASS → SUP 转换失败' }
    $validation = & $cli pgs validate $sup --level standard --json
    if ($LASTEXITCODE -ne 0) { throw '包内 SUP Standard 校验失败' }
    if (-not ($validation | ConvertFrom-Json).data.no_blocking_findings) { throw "包内 SUP 存在阻断问题：$validation" }
    Write-Host '[distribution] 文件校验、宿主握手、BDN/PNG 和 SUP 转换通过。' -ForegroundColor Green
}
finally {
    foreach ($key in $saved.Keys) { Set-Item -LiteralPath ('Env:' + $key) -Value $saved[$key] }
    Write-Host "[distribution] 验证记录：$scratch"
}
