# Benchmark slicing a saved .voxl scene on one or both backends via the control API.
# Usage: powershell -File scripts/bench/bench-slice.ps1 [-Voxl bench\filled-16k.voxl] [-Backends cpu,gpu]
param(
  [string]$Voxl = "",
  [string[]]$Backends = @('cpu', 'gpu'),
  [int]$BasePort = 8810,
  # Force one AA config on every backend (DF_SLICE_AA_MODE/LEVEL) so outputs
  # are comparable — e.g. -AaMode Coverage -AaLevel 4x. Empty = profile AA.
  [string]$AaMode = "",
  [string]$AaLevel = ""
)
$repo = (Resolve-Path "$PSScriptRoot\..\..").Path
$exe  = Join-Path $repo "src-tauri\target\release\dragonfruit-desktop.exe"
if (-not $Voxl) { $Voxl = Join-Path $repo "bench\filled-16k.voxl" }
$outDir = Join-Path $repo "bench"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

function Bench($backend, $port) {
  $base = "http://127.0.0.1:$port"
  Write-Host "`n######## BENCH: $backend ########"
  Get-Process dragonfruit-desktop -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 900
  $env:DF_CONTROL_PORT = "$port"
  $env:DF_SLICE_BACKEND = if ($backend -eq 'gpu') { 'gpu' } else { '' }
  $env:DF_SLICE_AA_MODE = $AaMode
  $env:DF_SLICE_AA_LEVEL = $AaLevel
  function Cmd($op, $p, $t = 900) {
    if ($null -eq $p) { $p = @{} }
    $b = @{ op = $op; params = $p } | ConvertTo-Json -Depth 8 -Compress
    $tmp = "$env:TEMP\dfctl.json"; Set-Content $tmp $b -Encoding ascii -NoNewline
    ((& curl.exe -s -S -X POST "$base/command" -H "Content-Type: application/json" --data "@$tmp" --max-time $t) | ConvertFrom-Json)
  }
  $proc = Start-Process -FilePath $exe -ArgumentList "`"$Voxl`"" -PassThru
  for ($i=0;$i -lt 60;$i++){ Start-Sleep -Milliseconds 750; try{ Invoke-RestMethod "$base/health" -TimeoutSec 3|Out-Null; break }catch{} }

  $loadSw = [Diagnostics.Stopwatch]::StartNew()
  $n = 0
  for ($t=0; $t -lt 90; $t++){ Start-Sleep -Seconds 2; try { $n = (Cmd "scene.list" @{}).result.models.Count } catch {}; if ($n -gt 0){ break } }
  $loadSw.Stop()
  Write-Host ("scene loaded: {0} models in {1:N1}s" -f $n, $loadSw.Elapsed.TotalSeconds)
  if ($n -le 0) {
    Get-Process dragonfruit-desktop -ErrorAction SilentlyContinue | Stop-Process -Force
    return [pscustomobject]@{ backend=$backend; load_s=$null; slice_s=$null; mb=$null; note='scene load failed' }
  }

  $out = Join-Path $outDir "bench_$backend.ctb"
  if (Test-Path $out) { Remove-Item $out -Force }
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $s = Cmd "slice" @{ output_path = $out } 900
  $sw.Stop()
  Get-Process dragonfruit-desktop -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  if ($s.ok) {
    $mb = if (Test-Path $out) { (Get-Item $out).Length / 1MB } else { 0 }
    Write-Host ("SLICE {0}: {1:N1}s wall -> {2:N1} MB" -f $backend.ToUpper(), $sw.Elapsed.TotalSeconds, $mb)
    [pscustomobject]@{ backend=$backend; load_s=[math]::Round($loadSw.Elapsed.TotalSeconds,1); slice_s=[math]::Round($sw.Elapsed.TotalSeconds,1); mb=[math]::Round($mb,1); note='' }
  } else {
    Write-Host "SLICE FAILED: $($s.error)"
    [pscustomobject]@{ backend=$backend; load_s=[math]::Round($loadSw.Elapsed.TotalSeconds,1); slice_s=$null; mb=$null; note="failed: $($s.error)" }
  }
}

$results = @()
$port = $BasePort
foreach ($b in $Backends) { $results += Bench $b $port; $port += 2 }
Write-Host "`n================ RESULTS ================"
$results | Format-Table -AutoSize
