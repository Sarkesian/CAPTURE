# Fetches CAPTURE's pinned third-party binaries into third_party/ and verifies SHA-256.
# Nothing is installed system-wide and nothing is written outside this repository.
#
#   powershell -ExecutionPolicy Bypass -File scripts\fetch-deps.ps1 [-WithObs]
#
# - FFmpeg 8.1.3 LGPL shared (BtbN autobuild-2026-11-07-12-47): runtime encode/mux + ffprobe
# - libclang 18.1.1 (PyPI wheel): build-time only, for generating FFmpeg Rust bindings
# - OBS Studio 33.0.1 portable (-WithObs): only to re-run the Phase 0 libobs comparison spike
param([switch]$WithObs)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$tp = Join-Path $root 'third_party'
$dl = Join-Path $tp '_dl'
New-Item -ItemType Directory -Force $dl | Out-Null

function Get-Pinned($url, $file, $sha256) {
    $path = Join-Path $dl $file
    if (-not (Test-Path $path)) {
        Write-Host "Downloading $file"
        Invoke-WebRequest -Uri $url -OutFile $path -UseBasicParsing
    }
    $actual = (Get-FileHash $path -Algorithm SHA256).Hash.ToLower()
    if ($actual -ne $sha256) { Remove-Item $path; throw "SHA-256 mismatch for $file (got $actual)" }
    return $path
}

$ff = 'ffmpeg-n8.1.3-34-ga35157abb9-win64-lgpl-shared-8.1'
$zip = Get-Pinned "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-11-07-12-47/$ff.zip" "$ff.zip" '1ae156959395f0f0b3ad97674de02dc21fb3bbee01142a0d8f744467b5cd5886'
if (-not (Test-Path (Join-Path $tp 'ffmpeg\bin\ffmpeg.exe'))) {
    Expand-Archive $zip -DestinationPath $tp -Force
    Rename-Item (Join-Path $tp $ff) 'ffmpeg'
}

$whl = 'libclang-18.1.1-py2.py3-none-win_amd64.whl'
$path = Join-Path $dl $whl
if (-not (Test-Path $path)) {
    python -I -m pip download libclang==18.1.1 --no-deps --only-binary=:all: --platform win_amd64 -d $dl -q
}
if (-not (Test-Path (Join-Path $tp 'libclang'))) {
    Copy-Item $path "$path.zip"
    Expand-Archive "$path.zip" -DestinationPath (Join-Path $tp 'libclang') -Force
    Remove-Item "$path.zip"
}

if ($WithObs) {
    $obs = 'OBS-Studio-33.0.1-Windows-x64.zip'
    $z = Get-Pinned "https://github.com/obsproject/obs-studio/releases/download/33.0.1/$obs" $obs '0d9436451c2d772b33428e3fdae9776f071895c335de17283176306fb72651d3'
    if (-not (Test-Path (Join-Path $tp 'obs\bin\64bit\obs64.exe'))) {
        Expand-Archive $z -DestinationPath (Join-Path $tp 'obs') -Force
        New-Item -ItemType File -Force (Join-Path $tp 'obs\obs_portable_mode.txt') | Out-Null
    }
}

Write-Host ''
Write-Host 'Build environment for FFmpeg bindings:'
Write-Host "  `$env:FFMPEG_DIR    = '$tp\ffmpeg'"
Write-Host "  `$env:LIBCLANG_PATH = '$tp\libclang\libclang-18.1.1.data\platlib\clang\native'"
