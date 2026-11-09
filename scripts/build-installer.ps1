# Wraps the tested portable payload (dist\CAPTURE-<version>\) in the Windows installer.
#
#   powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1
#
# Run scripts\package-portable.ps1 first. The payload is verified against its manifest and
# staged without any Data\ folder, so the installer contains exactly the program files of the
# portable zip, byte for byte. Output: dist\CAPTURE-<version>-setup.exe and dist\SHA256SUMS.txt.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$makensis = "$root\third_party\nsis\makensis.exe"
if (-not (Test-Path $makensis)) { & "$PSScriptRoot\fetch-deps.ps1" }

$version = (Get-Content "$root\app\src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json).version
$payload = "$root\dist\CAPTURE-$version"
$manifest = Get-Content "$payload\manifest.json" -Raw | ConvertFrom-Json
if ($manifest.version -ne $version) { throw "Payload is $($manifest.version), expected $version. Run package-portable.ps1." }

# Stage exactly the files in the manifest, verifying each one.
$stage = "$root\target\installer-payload"
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
foreach ($f in $manifest.files) {
    $src = Join-Path $payload $f.path
    $hash = (Get-FileHash $src -Algorithm SHA256).Hash.ToLower()
    if ($hash -ne $f.sha256) { throw "$($f.path) does not match the manifest" }
    $dst = Join-Path $stage $f.path
    New-Item -ItemType Directory -Force (Split-Path $dst) | Out-Null
    Copy-Item $src $dst
}
Copy-Item "$payload\manifest.json" $stage

$out = "$root\dist\CAPTURE-$version-setup.exe"
& $makensis /V2 /INPUTCHARSET UTF8 "/DVERSION=$version" "/DPAYLOAD=$stage" "/DOUTFILE=$out" "/DICON=$root\app\src-tauri\icons\icon.ico" "$root\installer\capture.nsi"
if ($LASTEXITCODE) { throw 'makensis failed' }
Write-Host ("Installer: {0} ({1:N0} MB)" -f $out, ((Get-Item $out).Length / 1MB))

# Checksums for everything a user downloads.
$sums = Get-ChildItem "$root\dist" -File | Where-Object { $_.Name -like "CAPTURE-$version-*" } | ForEach-Object {
    "{0}  {1}" -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name
}
[IO.File]::WriteAllLines("$root\dist\SHA256SUMS-$version.txt", $sums)
$sums | Write-Host
