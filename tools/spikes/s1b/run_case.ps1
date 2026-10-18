# S1b fault-case runner (spike tooling): one pipefault case per invocation.
# Logs CMD/START/RESULT per run -> docs/spikes/s1b/raw/s1b_<Case>_rep<Rep>.log
# pipefault.exe itself writes the per-run JSON + ffmpeg stderr copies into the same raw dir.
param(
  [Parameter(Mandatory = $true)][string]$Case,
  [Parameter(Mandatory = $true)][ValidateRange(1, 9)][int]$Rep,
  [string]$ExtraArgs = ""
)
$ErrorActionPreference = "Continue"
$exe = "C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\tools\spikes\s1b\pipefault\target\release\pipefault.exe"
$raw = "C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\docs\spikes\s1b\raw"
New-Item -ItemType Directory -Force -Path $raw | Out-Null

$label = "s1b_${Case}_rep${Rep}"
$log = Join-Path $raw "$label.log"
$argList = @("--case", $Case, "--label", $label)
$extra = $ExtraArgs -split '\s+' | Where-Object { $_ }
if ($extra) { $argList += $extra }

"CMD: pipefault $($argList -join ' ')" | Out-File -Encoding utf8 $log
"START: $(Get-Date -Format o)" | Out-File -Encoding utf8 -Append $log
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$out = & $exe @argList 2>&1
$exit = $LASTEXITCODE
$sw.Stop()
$out | Out-File -Encoding utf8 -Append $log
"RESULT: label=$label exit=$exit elapsed_s=$([math]::Round($sw.Elapsed.TotalSeconds,2))" | Out-File -Encoding utf8 -Append $log
# quick integrity summary from the JSON (if produced)
$json = Join-Path $raw "$label.json"
if (Test-Path $json) {
  $d = Get-Content $json -Raw | ConvertFrom-Json
  "PROBE: frames_written=$($d.frames_written) packets_sum=$($d.packets_sum) ffmpeg_exit=$($d.ffmpeg_exit) write_error=$($d.write_error | ConvertTo-Json -Compress)" | Out-File -Encoding utf8 -Append $log
}
"RUN: case=$Case rep=$Rep exit=$exit elapsed_s=$([math]::Round($sw.Elapsed.TotalSeconds,2))"
