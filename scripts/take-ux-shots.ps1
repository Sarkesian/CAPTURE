# Screenshots of every screen while CAPTURE records FAULTHEART, at a normal and a small size.
#   powershell -ExecutionPolicy Bypass -File scripts\take-ux-shots.ps1 [-Out docs\ux\after]
param([string]$Out = 'docs\ux\after')
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$Version = (Get-Content app\src-tauri\tauri.conf.json -Raw | ConvertFrom-Json).version
$game = 'D:\Programs\itch\Games\faultheart\FAULTHEART.exe'
$run = "$root\test-output\run-shots"; $cap = "$root\test-output\cap-shots"
Remove-Item -Recurse -Force $run, $cap -ErrorAction SilentlyContinue
Copy-Item -Recurse "$root\dist\CAPTURE-$Version" $run
Remove-Item -Recurse -Force "$run\data" -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$run\data" | Out-Null
'{ "capture_root": "ROOT", "replay_enabled": true, "replay_secs": 60, "first_run_done": true, "mic_enabled": true, "voice_enabled": true }'.Replace('ROOT', ($cap -replace '\\', '\\')) | Set-Content -Encoding utf8 "$run\data\settings.json"
$g = Start-Process $game -WorkingDirectory (Split-Path $game) -PassThru
Start-Sleep 5
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9333'
$p = Start-Process "$run\CAPTURE.exe" -WorkingDirectory $run -PassThru
Remove-Item Env:\WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
Start-Sleep 25  # let some replay accumulate
# Save one clip so Home, Library and the overlay show real thumbnails.
$code = "const pg=(await (await fetch('http://127.0.0.1:9333/json')).json()).find(t=>t.type==='page'&&!t.url.includes('#overlay'));const ws=new WebSocket(pg.webSocketDebuggerUrl);await new Promise(r=>ws.addEventListener('open',r,{once:true}));ws.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{expression:'window.__TAURI_INTERNALS__.invoke(\'save_replay\')',awaitPromise:true}}));ws.addEventListener('message',()=>process.exit(0));"
Set-Content -Encoding utf8 "$root\test-output\save.mjs" $code
node "$root\test-output\save.mjs"
Start-Sleep 8
node scripts\ui-shots.mjs $Out 1000 760
node scripts\ui-shots.mjs $Out 760 600
# The real window, including the native (DWM-themed) title bar.
Add-Type -AssemblyName System.Drawing
Add-Type -Namespace W -Name F -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(System.IntPtr h);
[DllImport("user32.dll")] public static extern bool ShowWindow(System.IntPtr h, int c);
[DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(System.IntPtr h, int a, out RECT r, int s);
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern System.IntPtr FindWindowEx(System.IntPtr p, System.IntPtr a, string c, string t);
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr h, out uint pid);
public struct RECT { public int L, T, R, B; }
'@
$h = [IntPtr]::Zero; $w = [IntPtr]::Zero
while (($w = [W.F]::FindWindowEx([IntPtr]::Zero, $w, [NullString]::Value, 'CAPTURE')) -ne [IntPtr]::Zero) { $o = 0; [void][W.F]::GetWindowThreadProcessId($w, [ref]$o); if ($o -eq $p.Id) { $h = $w; break } }
if ($h -ne [IntPtr]::Zero) {
  [void][W.F]::ShowWindow($h, 9); [void][W.F]::SetForegroundWindow($h); Start-Sleep 1
  $r = New-Object W.F+RECT; [void][W.F]::DwmGetWindowAttribute($h, 9, [ref]$r, 16)  # DWMWA_EXTENDED_FRAME_BOUNDS
  $bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
  $gfx = [System.Drawing.Graphics]::FromImage($bmp); $gfx.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
  $bmp.Save("$root\$Out\window-frame.png"); $gfx.Dispose(); $bmp.Dispose()
} else { 'main window not found for the frame shot' }
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Get-Process capture-worker -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$run\*" } | Stop-Process -Force
Stop-Process -Id $g.Id -ErrorAction SilentlyContinue
"shots in $Out"
