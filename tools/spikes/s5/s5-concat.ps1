# S5 spike concat finalize: build ffconcat list from *complete* segments, run concat -c copy (with and without -map 0), verify.
# Usage: .\s5-concat.ps1 -Tag crash_qsv_seg5 -Segments out_000.mkv,out_001.mkv
param(
  [Parameter(Mandatory=$true)][string]$Tag,
  [Parameter(Mandatory=$true)][string]$Segments   # comma-separated COMPLETE segments only
)
$ErrorActionPreference = "Continue"
$ffprobe = (Resolve-Path (Join-Path $PSScriptRoot "..\..\ffmpeg\ffmpeg-master-latest-win64-lgpl\bin\ffprobe.exe")).Path
$ffmpeg  = (Resolve-Path (Join-Path $PSScriptRoot "..\..\ffmpeg\ffmpeg-master-latest-win64-lgpl\bin\ffmpeg.exe")).Path
$logDir  = Join-Path $PSScriptRoot "logs"
$outDir  = Join-Path $env:TEMP ("opencode\s5\" + $Tag)

$segList = $Segments -split "," | ForEach-Object { $_.Trim() } | Where-Object { $_ -ne "" }
# ffconcat list with plain relative filenames (avoids Windows backslash escaping issues)
$listPath = Join-Path $outDir "list.txt"
$lines = @("ffconcat version 1.0") + ($segList | ForEach-Object { "file '$_'" })
$lines | Out-File $listPath -Encoding ascii
Write-Host "=== list.txt ==="; Get-Content $listPath

Push-Location $outDir
$sw = [System.Diagnostics.Stopwatch]::StartNew()
& $ffmpeg -hide_banner -y -f concat -safe 0 -i list.txt -c copy final_nomap.mkv 2> (Join-Path $logDir "$Tag`_concat_nomap.log")
$exitNomap = $LASTEXITCODE
$sw.Stop()
$tNomap = $sw.ElapsedMilliseconds

$sw.Restart()
& $ffmpeg -hide_banner -y -f concat -safe 0 -i list.txt -map 0 -c copy final.mkv 2> (Join-Path $logDir "$Tag`_concat_map0.log")
$exitMap = $LASTEXITCODE
$sw.Stop()
$tMap = $sw.ElapsedMilliseconds
Pop-Location

foreach ($f in @("final_nomap.mkv","final.mkv")) {
  $full = Join-Path $outDir $f
  if (Test-Path $full) {
    $dur = & $ffprobe -v error -show_entries format=duration -of default=nw=1 $full 2>&1
    $st  = & $ffprobe -v error -show_entries stream=index,codec_type,codec_name -of csv=p=0 $full 2>&1
    Write-Host "=== $f === duration: $($dur -join ' ')"; Write-Host "streams:"; $st
  } else { Write-Host "=== $f === MISSING" }
}
"nomap_exit=$exitNomap nomap_ms=$tNomap | map0_exit=$exitMap map0_ms=$tMap" |
  Out-File (Join-Path $logDir "$Tag`_concat_meta.txt") -Encoding utf8
Write-Host "concat timing: no-map=${tNomap}ms  with-map0=${tMap}ms"
exit 0
