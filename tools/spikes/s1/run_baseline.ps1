# S1 baseline runner (spike tooling): pure-ffmpeg testsrc2 encode matrix.
# Configs: encoder x concurrency (1/2/3) x 3 reps. Logs -> docs/spikes/s1/raw/.
param(
  [Parameter(Mandatory = $true)][ValidateSet("h264_qsv", "h264_mf")][string]$Encoder,
  [Parameter(Mandatory = $true)][ValidateRange(1, 3)][int]$N,
  [Parameter(Mandatory = $true)][ValidateRange(1, 9)][int]$Rep
)
$ErrorActionPreference = "Continue"
$bin = "C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\tools\ffmpeg\ffmpeg-master-latest-win64-lgpl\bin"
$ff = Join-Path $bin "ffmpeg.exe"
$fp = Join-Path $bin "ffprobe.exe"
$raw = "C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\docs\spikes\s1\raw"
$outD = "C:\Users\elvis\AppData\Local\Temp\opencode\s1out\baseline"
New-Item -ItemType Directory -Force -Path $outD | Out-Null

$argList = @("-hide_banner", "-nostdin", "-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=30", "-t", "10", "-g", "60", "-c:v", $Encoder, "-f", "segment", "-segment_time", "5", "-reset_timestamps", "1", "-segment_format", "matroska")
$jobs = @()
for ($i = 1; $i -le $N; $i++) {
  $tag = "baseline_${Encoder}_${N}x_rep${Rep}_p$i"
  $dir = Join-Path $outD $tag
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  $out = Join-Path $dir "out_%05d.mkv"
  $log = Join-Path $raw "$tag.log"
  "CMD: ffmpeg $(($argList + @('-y', $out)) -join ' ')" | Out-File -Encoding utf8 $log
  "START: $(Get-Date -Format o)" | Out-File -Encoding utf8 -Append $log
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $p = Start-Process -FilePath $ff -ArgumentList ($argList + @("-y", $out)) -NoNewWindow -PassThru -RedirectStandardError "$log.err.tmp" -RedirectStandardOutput "$log.out.tmp"
  $null = $p.Handle
  $jobs += [pscustomobject]@{ Tag = $tag; Dir = $dir; Proc = $p; Sw = $sw; Log = $log }
}
foreach ($j in $jobs) {
  $j.Proc.WaitForExit()
  $j.Sw.Stop()
  $elapsed = [math]::Round($j.Sw.Elapsed.TotalSeconds, 2)
  Get-Content "$($j.Log).err.tmp" | Out-File -Encoding utf8 -Append $j.Log
  Remove-Item "$($j.Log).err.tmp", "$($j.Log).out.tmp" -Force -ErrorAction SilentlyContinue
  $speedMatch = Select-String -Path $j.Log -Pattern "speed=\s*([0-9.]+)x" | Select-Object -Last 1
  $speed = if ($speedMatch) { $speedMatch.Matches.Groups[1].Value } else { "" }
  "RESULT: tag=$($j.Tag) exit=$($j.Proc.ExitCode) elapsed_s=$elapsed speed=$speed" | Out-File -Encoding utf8 -Append $j.Log
  # segment verification (sum durations & packets across segments)
  $segs = Get-ChildItem $j.Dir -Filter "out_*.mkv" | Sort-Object Name
  $durSum = 0.0; $pktSum = 0; $nseg = 0
  foreach ($s in $segs) {
    $d = & $fp -v error -show_entries format=duration -of csv=p=0 $s.FullName 2>$null
    $p = & $fp -v error -count_packets -select_streams v:0 -show_entries stream=nb_read_packets -of csv=p=0 $s.FullName 2>$null
    $durSum += [double]$d; $pktSum += [int]$p; $nseg++
  }
  "PROBE: segs=$nseg dur_sum=$([math]::Round($durSum,3)) packets_sum=$pktSum" | Out-File -Encoding utf8 -Append $j.Log
  "BASELINE: enc=$Encoder n=$N rep=$Rep exit=$($j.Proc.ExitCode) elapsed_s=$elapsed speed=${speed}x segs=$nseg dur_sum=$([math]::Round($durSum,3)) packets_sum=$pktSum"
}
