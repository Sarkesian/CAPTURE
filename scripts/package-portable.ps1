# Builds CAPTURE's one application payload, the portable zip made from it, and checksums.
# The Windows installer (scripts\build-installer.ps1) wraps exactly this payload.
#
#   powershell -ExecutionPolicy Bypass -File scripts\package-portable.ps1 [-WebView2Cab <path>] [-SkipBuild]
#
# dist\CAPTURE-<version>\                 the payload (also runnable in place)
#   CAPTURE.exe                           UI, tray, settings (Tauri 2 + WebView2)
#   manifest.json                         version + SHA-256/size of every program file
#   Runtime\capture-worker.exe            capture engine
#   Runtime\capture-hook.dll, capture-hook32.dll, capture-inject32.exe
#                                         in-game overlay hook (64/32-bit) and the 32-bit loader
#   Runtime\av*.dll, sw*.dll              FFmpeg 8.1 LGPL (unmodified, shared)
#   Runtime\WebView2\                     only with -WebView2Cab (Microsoft Fixed Version runtime)
#   Resources\QUICKSTART.md, Resources\Licenses\
#   Data\                                 created on first run (settings, cache, logs); never packaged
# dist\CAPTURE-<version>-portable.zip    the payload without Data\
#
# Rebuilding replaces program files only: a Data\ folder in dist\CAPTURE-<version>\ (someone's
# settings) is kept.
param([string]$WebView2Cab, [switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $env:RUSTUP_HOME) { $env:RUSTUP_HOME = 'D:\Programs\Rust\rustup' }
if (-not $env:CARGO_HOME) { $env:CARGO_HOME = 'D:\Programs\Rust\cargo' }
$env:PATH = "$env:CARGO_HOME\bin;$env:PATH"

if (-not (Test-Path "$root\third_party\ffmpeg\bin\avcodec-62.dll")) {
    & "$PSScriptRoot\fetch-deps.ps1"
}

Push-Location $root
try {
    if (-not $SkipBuild) {
        cargo build --release -p capture-engine --bin capture-worker -p capture-hook
        if ($LASTEXITCODE) { throw 'worker build failed' }
        # 32-bit games get a 32-bit hook, loaded by a 32-bit helper.
        cargo build --release -p capture-hook -p capture-inject32 --target i686-pc-windows-msvc
        if ($LASTEXITCODE) { throw '32-bit hook build failed (rustup target add i686-pc-windows-msvc)' }
        pnpm --dir app install --frozen-lockfile
        if ($LASTEXITCODE) { throw 'pnpm install failed' }
        pnpm --dir app tauri build --no-bundle
        if ($LASTEXITCODE) { throw 'app build failed' }
    }

    $version = (Get-Content app\src-tauri\tauri.conf.json -Raw | ConvertFrom-Json).version
    $out = "$root\dist\CAPTURE-$version"
    if (Test-Path $out) {
        Get-ChildItem $out -Force | Where-Object { $_.Name -ne 'Data' } | Remove-Item -Recurse -Force
    }
    New-Item -ItemType Directory -Force "$out\Runtime", "$out\Resources\Licenses" | Out-Null
    Copy-Item target\release\CAPTURE.exe $out
    Copy-Item target\release\capture-worker.exe "$out\Runtime"
    Copy-Item target\release\capture_hook.dll "$out\Runtime\capture-hook.dll"
    Copy-Item target\i686-pc-windows-msvc\release\capture_hook.dll "$out\Runtime\capture-hook32.dll"
    Copy-Item target\i686-pc-windows-msvc\release\capture-inject32.exe "$out\Runtime"
    foreach ($dll in 'avcodec-62', 'avformat-62', 'avutil-60', 'swresample-6', 'swscale-9') {
        Copy-Item "third_party\ffmpeg\bin\$dll.dll" "$out\Runtime"
    }
    Copy-Item third_party\ffmpeg\LICENSE.txt "$out\Resources\Licenses\FFmpeg-LICENSE.txt"
    Copy-Item docs\THIRD_PARTY.md "$out\Resources\Licenses\THIRD_PARTY.md"
    Copy-Item docs\QUICKSTART.md "$out\Resources\QUICKSTART.md"

    if ($WebView2Cab) {
        # Microsoft's Fixed Version runtime, extracted as-is. CAPTURE uses it automatically when
        # Runtime\WebView2\msedgewebview2.exe exists. Security fixes then need a new CAPTURE build.
        $wv = "$out\Runtime\WebView2"
        $tmp = "$root\target\webview2-cab"
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
        New-Item -ItemType Directory -Force $tmp | Out-Null
        expand.exe $WebView2Cab -F:* $tmp | Out-Null
        if ($LASTEXITCODE) { throw 'Could not extract the WebView2 cab' }
        $inner = Get-ChildItem $tmp -Directory | Select-Object -First 1
        Move-Item $inner.FullName $wv
    }

    # The manifest: every program file with its SHA-256 and size. Updates verify against it.
    $files = Get-ChildItem $out -Recurse -File -Force | Where-Object {
        $rel = $_.FullName.Substring($out.Length + 1)
        -not $rel.StartsWith('Data\', [StringComparison]::OrdinalIgnoreCase) -and $rel -ne 'manifest.json'
    } | Sort-Object FullName | ForEach-Object {
        [ordered]@{
            path   = $_.FullName.Substring($out.Length + 1)
            sha256 = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower()
            size   = $_.Length
        }
    }
    $manifest = [ordered]@{ product = 'CAPTURE'; version = $version; files = @($files) }
    [IO.File]::WriteAllText("$out\manifest.json", ($manifest | ConvertTo-Json -Depth 4), (New-Object Text.UTF8Encoding $false))

    $size = (Get-ChildItem -Recurse -File $out | Where-Object { -not $_.FullName.StartsWith("$out\Data", [StringComparison]::OrdinalIgnoreCase) } | Measure-Object Length -Sum).Sum / 1MB
    Write-Host ("Payload:      {0} ({1:N0} MB, {2} files)" -f $out, $size, $files.Count)

    # Portable zip: the payload without Data\ (never anyone's settings).
    $zip = "$root\dist\CAPTURE-$version-portable.zip"
    Remove-Item $zip -ErrorAction SilentlyContinue
    Compress-Archive -Path (Get-ChildItem $out -Force | Where-Object { $_.Name -ne 'Data' }).FullName -DestinationPath $zip -CompressionLevel Optimal
    Write-Host ("Portable zip: {0} ({1:N0} MB)" -f $zip, ((Get-Item $zip).Length / 1MB))
}
finally { Pop-Location }
