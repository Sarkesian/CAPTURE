# Runs the real-app acceptance suites against a packaged build, each in an isolated copy with
# its own settings and capture folder under test-output\. Needs FAULTHEART (itch.io) installed.
#
#   powershell -ExecutionPolicy Bypass -File scripts\run-acceptance.ps1 [-Version 0.3.0]
param([string]$Version = '')
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
if (-not $Version) { $Version = (Get-Content app\src-tauri\tauri.conf.json -Raw | ConvertFrom-Json).version }
$game = 'D:\Programs\itch\Games\faultheart\FAULTHEART.exe'
$env:PATH = "$root\third_party\ffmpeg\bin;$env:PATH"
$results = @()

function Start-Run($name, $settingsJson) {
    $run = "$root\test-output\run-$name"
    Get-Process CAPTURE, capture-worker -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$root\test-output\*" } | Stop-Process -Force
    Start-Sleep -Milliseconds 500
    Remove-Item -Recurse -Force $run, "$root\test-output\cap-$name" -ErrorAction SilentlyContinue
    Copy-Item -Recurse "$root\dist\CAPTURE-$Version" $run
    Remove-Item -Recurse -Force "$run\data" -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force "$run\data" | Out-Null
    $settingsJson.Replace('ROOT', ("$root\test-output\cap-$name" -replace '\\', '\\')) | Set-Content -Encoding utf8 "$run\data\settings.json"
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9333'
    $p = Start-Process "$run\CAPTURE.exe" -WorkingDirectory $run -PassThru
    Remove-Item Env:\WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
    return $p
}
function Stop-Run($p) {
    if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
    Get-Process capture-worker -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$root\test-output\*" } | Stop-Process -Force
}

# 1) Auto detection, notice, numpad hotkeys
Get-Process FAULTHEART -ErrorAction SilentlyContinue | Stop-Process
$p = Start-Run 'auto' '{ "capture_root": "ROOT", "save_hotkey": "Num1", "replay_secs": 30, "first_run_done": true }'
node scripts\ui-auto-test.mjs $game "$root\test-output\shots-auto" 2>&1 | Tee-Object -Variable out1 | ForEach-Object { "  $_" }
$results += "auto: " + ($out1 | Select-Object -Last 1)
Stop-Run $p

# 2) Overlay, markers, review, sessions
$g = Start-Process $game -WorkingDirectory (Split-Path $game) -PassThru
Start-Sleep -Seconds 5
$p = Start-Run 'overlay' '{ "capture_root": "ROOT", "replay_enabled": true, "replay_secs": 30, "first_run_done": true }'
Start-Sleep -Seconds 12
node scripts\ui-overlay-test.mjs "$root\test-output\shots-overlay" "$root\test-output\cap-overlay" 2>&1 | Tee-Object -Variable out2 | ForEach-Object { "  $_" }
$results += "overlay: " + ($out2 | Select-Object -Last 1)
Stop-Run $p
Stop-Process -Id $g.Id -ErrorAction SilentlyContinue

""; "SUMMARY"; $results
