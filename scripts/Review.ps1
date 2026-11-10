# Launch this candidate with disposable sample history, isolated from the installed app.
param([switch]$InitializeOnly)
$ErrorActionPreference = 'Stop'
$settingsDir = Join-Path $PSScriptRoot 'Data\Settings'
$settingsFile = Join-Path $settingsDir 'settings.json'
$media = Join-Path $PSScriptRoot 'ReviewCaptures'
if (-not (Test-Path -LiteralPath $settingsFile)) {
    foreach ($session in Get-ChildItem -LiteralPath (Join-Path $media 'Background Recording') -Directory) {
        $manifest = Get-Content -LiteralPath (Join-Path $session.FullName 'session.json') -Raw | ConvertFrom-Json
        $journal = Join-Path $session.FullName 'events.jsonl'
        $lines = foreach ($line in Get-Content -LiteralPath $journal) {
            $event = $line | ConvertFrom-Json
            if ($event.kind -eq 'screenshot' -and $event.path) {
                $event.path = Join-Path (Join-Path (Join-Path $media $manifest.game) 'Screenshots') ([IO.Path]::GetFileName($event.path))
            }
            $event | ConvertTo-Json -Depth 8 -Compress
        }
        [IO.File]::WriteAllLines($journal, [string[]]$lines, [Text.UTF8Encoding]::new($false))
    }
    New-Item -ItemType Directory -Force -Path $settingsDir | Out-Null
    $settings = @{version=3;capture_root=$media;replay_enabled=$false;history_secs=21600;target=@{kind='none'};first_run_done=$true;close_to_tray=$false;save_hotkey='';hotkeys=@{marker='';overlay='';session='';screenshot='';mic_mute=''};notify_on_save=$false;min_free_gb=1}
    [IO.File]::WriteAllText($settingsFile, ($settings | ConvertTo-Json -Depth 6), [Text.UTF8Encoding]::new($false))
}
if(-not $InitializeOnly){Start-Process -FilePath (Join-Path $PSScriptRoot 'CAPTURE.exe') -ArgumentList '--library' -WorkingDirectory $PSScriptRoot}
