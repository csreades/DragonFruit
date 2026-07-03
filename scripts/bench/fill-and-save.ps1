# Fill the bed from an STL via the control API and save a reusable .voxl.
# Usage: powershell -File scripts/bench/fill-and-save.ps1 -Stl <path> [-Spacing 0.5] [-Out bench\filled-16k.voxl]
param(
  [int]$Port = 8796,
  [Parameter(Mandatory = $true)][string]$Stl,
  [double]$Spacing = 0.5,
  [string]$Printer = "Saturn 4 Ultra 16K",
  [string]$Out = ""
)
$repo = (Resolve-Path "$PSScriptRoot\..\..").Path
$exe  = Join-Path $repo "src-tauri\target\release\dragonfruit-desktop.exe"
if (-not $Out) { $Out = Join-Path $repo "bench\filled-16k.voxl" }
$base = "http://127.0.0.1:$Port"
New-Item -ItemType Directory -Force -Path (Split-Path $Out) | Out-Null
if (Test-Path $Out) { Remove-Item $Out -Force }
$env:DF_CONTROL_PORT = "$Port"; $env:DF_SLICE_BACKEND = ""

function Cmd($op, $p, $t = 600) {
  if ($null -eq $p) { $p = @{} }
  $b = @{ op = $op; params = $p } | ConvertTo-Json -Depth 8 -Compress
  $tmp = "$env:TEMP\dfctl.json"; Set-Content $tmp $b -Encoding ascii -NoNewline
  $r = (& curl.exe -s -S -X POST "$base/command" -H "Content-Type: application/json" --data "@$tmp" --max-time $t) | ConvertFrom-Json
  if (-not $r.ok) { throw "op $op failed: $($r.error)" }
  return $r.result
}

Get-Process dragonfruit-desktop -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 900
$proc = Start-Process -FilePath $exe -PassThru
Write-Host "pid $($proc.Id)"
try {
  for ($i=0;$i -lt 60;$i++){ Start-Sleep -Milliseconds 750; try{ Invoke-RestMethod "$base/health" -TimeoutSec 3|Out-Null; break }catch{} }
  Start-Sleep -Seconds 15
  Write-Host "printer: $((Cmd 'printer.set' @{name=$Printer}).name)"
  $l = Cmd "mesh.load" @{ path = $Stl } 600
  Write-Host "loaded: $($l.loaded)"
  $f = Cmd "scene.fillPlate" @{ id = $l.created[0]; spacing_mm = $Spacing } 600
  Write-Host "filled: $($f.total_models) models @ $($Spacing)mm"
  $s = Cmd "scene.save" @{ path = $Out } 300
  Write-Host "saved -> $($s.saved_path)"
  Start-Sleep -Seconds 2
  if (Test-Path $Out) { "VOXL on disk: {0:N0} bytes  {1}" -f (Get-Item $Out).Length, $Out } else { "ERROR: voxl not written" }
}
finally {
  Get-Process dragonfruit-desktop -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
}
