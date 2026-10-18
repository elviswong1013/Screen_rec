# S5 spike segment verification: per-segment ffprobe (duration/streams/frame count) + full decode check.
# Usage: .\s5-verify.ps1 -Tag crash_qsv_seg5
param(
  [Parameter(Mandatory=$true)][string]$Tag
)
$ErrorActionPreference = "Continue"
$ffprobe = (Resolve-Path (Join-Path $PSScriptRoot "..\..\ffmpeg\ffmpeg-master-latest-win64-lgpl\bin\ffprobe.exe")).Path
$ffmpeg  = (Resolve-Path (Join-Path $PSScriptRoot "..\..\ffmpeg\ffmpeg-master-latest-win64-lgpl\bin\ffmpeg.exe")).Path
$logDir  = Join-Path $PSScriptRoot "logs"
$outDir  = Join-Path $env:TEMP ("opencode\s5\" + $Tag)

$segs = Get-ChildItem $outDir -Filter "out_*.mkv" | Sort-Object Name
$summary = @()
foreach ($seg in $segs) {
  $name = $seg.Name
  $full = $seg.FullName
  $probeLog = Join-Path $logDir "$Tag`_probe_$name.txt"
  $decLog   = Join-Path $logDir "$Tag`_decode_$name.txt"

  # format/stream probe (json to log, key fields to summary)
  & $ffprobe -v error -show_format -show_streams -of json $full 2>$null | Out-File $probeLog -Encoding utf8
  $fmt = & $ffprobe -v error -show_entries format=duration,size -of default=nw=1 $full 2>&1
  $ns  = & $ffprobe -v error -show_entries format=nb_streams -of default=nw=1 $full 2>&1
  $vfr = & $ffprobe -v error -select_streams v:0 -count_frames -show_entries stream=nb_read_frames,width,height,codec_name -of default=nw=1 $full 2>&1
  $afr = & $ffprobe -v error -select_streams a -count_frames -show_entries stream=nb_read_frames -of csv=p=0 $full 2>&1
  # first video packet flags (keyframe check) + codec
  $firstPk = & $ffprobe -v error -select_streams v:0 -read_intervals "%+#1" -show_entries packet=flags,pts_time -of csv=p=0 $full 2>&1

  # full decode check
  & $ffmpeg -hide_banner -v error -i $full -f null - 2> $decLog
  $decExit = $LASTEXITCODE
  $errLines = @(Get-Content $decLog -ErrorAction SilentlyContinue | Where-Object { $_.Trim() -ne "" })

  $row = [ordered]@{
    segment = $name; size = $seg.Length
    ffprobe_exit = $LASTEXITCODE
    duration = (($fmt -join " ") -replace "duration=","")
    nb_streams = (($ns -join " ") -replace "nb_streams=","")
    video = (($vfr -join ",") -replace "`r|`n","")
    audio_frames = (($afr -join ";") -replace "`r|`n","")
    first_vpacket = ($firstPk -join "|")
    decode_exit = $decExit
    decode_err_lines = $errLines.Count
    decode_err_first = if ($errLines.Count -gt 0) { $errLines[0].Substring(0, [Math]::Min(120, $errLines[0].Length)) } else { "" }
  }
  $summary += [pscustomobject]$row
}
$summary | Format-Table -AutoSize -Wrap | Out-String -Width 250
$summary | ConvertTo-Json | Out-File (Join-Path $logDir "$Tag`_verify_summary.json") -Encoding utf8
exit 0
