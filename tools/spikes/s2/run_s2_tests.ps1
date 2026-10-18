# S2 spike test runner (spike tooling, NOT production code).
# Runs ffmpeg encoder capability tests with verbatim command logging.
# All encode outputs go to a temp dir (not committed); logs go to docs/spikes/s2/raw/.
param(
  [Parameter(Mandatory = $true)][ValidateSet("single", "concurrent", "keyframes", "vp9", "probe")][string]$Phase,
  [string[]]$Encoders = @(),
  [int]$N = 2
)

$ErrorActionPreference = "Continue"
$Encoders = @($Encoders | ForEach-Object { $_ -split "," } | Where-Object { $_ })
$bin  = "C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\tools\ffmpeg\ffmpeg-master-latest-win64-lgpl\bin"
$ff   = Join-Path $bin "ffmpeg.exe"
$fp   = Join-Path $bin "ffprobe.exe"
$raw  = "C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\docs\spikes\s2\raw"
$outD = "C:\Users\elvis\AppData\Local\Temp\opencode\s2out"
New-Item -ItemType Directory -Force -Path $outD, $raw | Out-Null

function Get-Size([string]$p) { if (Test-Path $p) { (Get-Item $p).Length } else { -1 } }

function Get-Probe([string]$p, [string]$logPath) {
  if (-not (Test-Path $p)) { return }
  $dur = & $fp -v error -show_entries format=duration -of csv=p=0 $p 2>&1
  $packets = & $fp -v error -count_packets -select_streams v:0 -show_entries stream=nb_read_packets,codec_name,width,height,avg_frame_rate -of csv=p=0 $p 2>&1
  $kfs = & $fp -v error -select_streams v:0 -show_entries packet=pts_time,flags -of csv=p=0 $p 2>&1 | Where-Object { $_ -match ",K" }
  $maxGap = 0.0
  $prev = $null
  foreach ($kf in $kfs) {
    $t = [double]($kf -replace ",.*$", "")
    if ($null -ne $prev) { $gap = $t - $prev; if ($gap -gt $maxGap) { $maxGap = $gap } }
    $prev = $t
  }
  @"
== ffprobe $p ==
duration=$dur
stream=$packets
keyframe_count=$($kfs.Count)
keyframes=$(($kfs | ForEach-Object { $_ -replace ",.*$", "" }) -join " ")
max_keyframe_gap_s=$([math]::Round($maxGap, 4))
"@ | Out-File -Encoding utf8 -Append $logPath
}

function Start-EncodeProc([string]$enc, [string]$tag, [string[]]$extra) {
  # Verbatim command line is echoed into the log.
  $log = Join-Path $raw "$tag.log"
  $out = Join-Path $outD "$tag.mkv"
  if (Test-Path $out) { Remove-Item $out -Force }
  $argList = @("-hide_banner", "-nostdin", "-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=30", "-t", "10", "-g", "60") + $extra + @("-c:v", $enc, "-y", $out)
  "CMD: ffmpeg $((($argList | ForEach-Object { $_ }) -join ' '))" | Out-File -Encoding utf8 $log
  "START: $(Get-Date -Format o)" | Out-File -Encoding utf8 -Append $log
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $p = Start-Process -FilePath $ff -ArgumentList $argList -NoNewWindow -PassThru -RedirectStandardError "$log.err.tmp" -RedirectStandardOutput "$log.out.tmp"
  $null = $p.Handle  # cache handle so ExitCode is populated reliably on PS 5.1
  [pscustomobject]@{ Enc = $enc; Tag = $tag; Proc = $p; Sw = $sw; Out = $out; Log = $log }
}

function Complete-EncodeProc($job) {
  $job.Proc.WaitForExit()
  $job.Sw.Stop()
  $elapsed = [math]::Round($job.Sw.Elapsed.TotalSeconds, 2)
  Get-Content "$($job.Log).err.tmp" | Out-File -Encoding utf8 -Append $job.Log
  Remove-Item "$($job.Log).err.tmp", "$($job.Log).out.tmp" -Force -ErrorAction SilentlyContinue
  $size = Get-Size $job.Out
  $speedMatch = Select-String -Path $job.Log -Pattern "speed=\s*([0-9.]+)x" | Select-Object -Last 1
  $speed = if ($speedMatch) { $speedMatch.Matches.Groups[1].Value } else { "" }
  $errMatch = Select-String -Path $job.Log -Pattern "Error|failed|Failed" | Select-Object -First 1
  $errline = if ($errMatch) { $errMatch.Line } else { "" }
  "RESULT: tag=$($job.Tag) enc=$($job.Enc) exit=$($job.Proc.ExitCode) elapsed_s=$elapsed out_bytes=$size speed=$speed first_error=$errline" |
    Out-File -Encoding utf8 -Append $job.Log
  Get-Probe $job.Out $job.Log
  return [pscustomobject]@{ Tag = $job.Tag; Enc = $job.Enc; Exit = $job.Proc.ExitCode; Elapsed = $elapsed; Size = $size; Speed = $speed }
}

switch ($Phase) {
  "single" {
    foreach ($enc in $Encoders) {
      $r = Complete-EncodeProc (Start-EncodeProc $enc "single_$enc" @())
      "SINGLE: enc=$($r.Enc) exit=$($r.Exit) elapsed_s=$($r.Elapsed) out_bytes=$($r.Size) speed=$($r.Speed)"
    }
  }
  "concurrent" {
    $jobs = @()
    $i = 1
    while ($i -le $N) {
      foreach ($enc in $Encoders) {
        $jobs += Start-EncodeProc $enc "conc${N}_${enc}_p$i" @()
        $i++
        if ($i -gt $N) { break }
      }
    }
    $results = foreach ($j in $jobs) { Complete-EncodeProc $j }
    $results | ForEach-Object { "CONC${N}: tag=$($_.Tag) enc=$($_.Enc) exit=$($_.Exit) elapsed_s=$($_.Elapsed) out_bytes=$($_.Size) speed=$($_.Speed)" }
  }
  "keyframes" {
    foreach ($enc in $Encoders) {
      $r = Complete-EncodeProc (Start-EncodeProc $enc "kf_$enc" @("-force_key_frames", "expr:gte(t,n_forced*5)"))
      "KEYFRAMES: enc=$($r.Enc) exit=$($r.Exit) elapsed_s=$($r.Elapsed) out_bytes=$($r.Size)"
    }
  }
  "vp9" {
    # Capability smoke test only (non-default speed flags; NO performance conclusion).
    $r = Complete-EncodeProc (Start-EncodeProc "libvpx-vp9" "vp9_smoke" @("-deadline", "good", "-cpu-used", "4", "-row-mt", "1"))
    "VP9: exit=$($r.Exit) elapsed_s=$($r.Elapsed) out_bytes=$($r.Size)"
  }
}
