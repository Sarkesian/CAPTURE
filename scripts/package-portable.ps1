# Builds a portable CAPTURE folder: dist\CAPTURE-<version>\
#
#   powershell -ExecutionPolicy Bypass -File scripts\package-portable.ps1
#
# Contents: CAPTURE.exe (UI), capture-worker.exe (engine), the five LGPL FFmpeg DLLs the
# engine links, licences and the quick-start guide. Settings and the WebView2 profile are
# created in .\data next to CAPTURE.exe on first run; recordings go to the capture folder.
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
    cargo build --release -p capture-engine --bin capture-worker
    if ($LASTEXITCODE) { throw 'worker build failed' }
    pnpm --dir app install --frozen-lockfile
    if ($LASTEXITCODE) { throw 'pnpm install failed' }
    pnpm --dir app tauri build --no-bundle
    if ($LASTEXITCODE) { throw 'app build failed' }

    $version = (Get-Content app\src-tauri\tauri.conf.json -Raw | ConvertFrom-Json).version
    $out = "$root\dist\CAPTURE-$version"
    if (Test-Path $out) { Remove-Item -Recurse -Force $out }
    New-Item -ItemType Directory -Force "$out\licenses" | Out-Null
    Copy-Item target\release\CAPTURE.exe, target\release\capture-worker.exe $out
    foreach ($dll in 'avcodec-62', 'avformat-62', 'avutil-60', 'swresample-6', 'swscale-9') {
        Copy-Item "third_party\ffmpeg\bin\$dll.dll" $out
    }
    Copy-Item third_party\ffmpeg\LICENSE.txt "$out\licenses\FFmpeg-LICENSE.txt"
    Copy-Item docs\THIRD_PARTY.md "$out\licenses\THIRD_PARTY.md"
    Copy-Item docs\QUICKSTART.md "$out\QUICKSTART.md"
    $size = (Get-ChildItem -Recurse $out | Measure-Object Length -Sum).Sum / 1MB
    Write-Host ("Portable build: {0} ({1:N0} MB)" -f $out, $size)
}
finally { Pop-Location }
