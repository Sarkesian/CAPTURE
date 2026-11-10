param([Parameter(Mandatory=$true)][string]$OutputDirectory, [string]$Baseline='D:\Programs\CAPTURE')
$ErrorActionPreference='Stop'
$repo=Split-Path -Parent $PSScriptRoot
$version=(Get-Content -LiteralPath "$repo\app\src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json).version
$out=Join-Path ([IO.Path]::GetFullPath($OutputDirectory)) "CAPTURE-$version"
if(Test-Path -LiteralPath $out){throw "Candidate output already exists: $out"}
$baselineManifest=Get-Content -LiteralPath (Join-Path $Baseline 'manifest.json') -Raw | ConvertFrom-Json
if($baselineManifest.version -ne '0.6.4'){throw 'This UI candidate requires the verified 0.6.4 baseline runtime'}
New-Item -ItemType Directory -Path $out | Out-Null
foreach($entry in $baselineManifest.files){
    $source=Join-Path $Baseline $entry.path
    if((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLower() -ne $entry.sha256){throw "Baseline hash mismatch: $source"}
    $dest=Join-Path $out $entry.path
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $dest) | Out-Null
    Copy-Item -LiteralPath $source -Destination $dest
}
Copy-Item -LiteralPath "$repo\target\release\CAPTURE.exe" -Destination "$out\CAPTURE.exe" -Force
Copy-Item -LiteralPath "$PSScriptRoot\Review.ps1" -Destination $out
Copy-Item -LiteralPath "$repo\docs\REPLAY_OVERHAUL.md" -Destination "$out\Resources\REPLAY_REVIEW.md"
$fixtures=Join-Path $repo 'test-output\native-replay-media'
New-Item -ItemType Directory -Path "$out\ReviewCaptures\Background Recording" | Out-Null
# Only the five deliberately generated fixtures, never the live regression captures/catalog.
foreach($session in Get-ChildItem -LiteralPath "$fixtures\Background Recording" -Directory | Where-Object {$_.Name -like '2026-11-10_12-00-0*'}){
    Copy-Item -LiteralPath $session.FullName -Destination "$out\ReviewCaptures\Background Recording" -Recurse
}
foreach($game in @('Hollow Knight','The Witcher 3')){
    New-Item -ItemType Directory -Force -Path "$out\ReviewCaptures\$game\Screenshots" | Out-Null
    Get-ChildItem -LiteralPath "$fixtures\$game\Screenshots" -Filter 'fixture-*.png' | Copy-Item -Destination "$out\ReviewCaptures\$game\Screenshots"
}
$readme=@"
CAPTURE $version — Replay review candidate

Extract the whole folder. Run Review.ps1 to initialize its own disposable ReviewCaptures
and Data folders, then open Replay. Recording starts OFF. No installed settings are imported.
If you run CAPTURE.exe directly before Review.ps1, it uses normal first-run defaults instead.

The samples represent 2-minute, 30-minute, 2-hour, 4-hour and 6-hour spans using short
encoded windows separated by intentional gaps. They are not continuous hours of gameplay.
The six-hour sample has a dense group near 2:00:06 and a final highlight near 5:59:52.
Open Moments, select a marker or screenshot, use Clip around or Select clip, adjust the
handles, and Save Clip. Saves go only to this review folder's Library.

The Runtime files are byte-identical to the installed 0.6.4 release. Only CAPTURE.exe
contains the new UI. Nothing has been installed. See Resources\REPLAY_REVIEW.md for the
implementation report, verified behavior, limitations and source rollback point.
"@
[IO.File]::WriteAllText("$out\REVIEW.txt",$readme,[Text.UTF8Encoding]::new($false))
$files=@(Get-ChildItem -LiteralPath $out -Recurse -File | Where-Object { -not $_.FullName.StartsWith("$out\ReviewCaptures\",[StringComparison]::OrdinalIgnoreCase) } | Sort-Object FullName | ForEach-Object {
    [ordered]@{path=$_.FullName.Substring($out.Length+1);sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLower();size=$_.Length}
})
$manifest=[ordered]@{product='CAPTURE';version=$version;files=$files}
[IO.File]::WriteAllText("$out\manifest.json",($manifest|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))
$zip="$out-portable.zip"
Compress-Archive -LiteralPath $out -DestinationPath $zip -CompressionLevel Optimal
[IO.File]::WriteAllText("$zip.sha256",((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLower()+"  "+[IO.Path]::GetFileName($zip)+"`n"),[Text.UTF8Encoding]::new($false))
Write-Output $zip
