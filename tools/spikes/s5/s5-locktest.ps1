# S5 spike: file handle/lock release immediately after taskkill /F of the recording ffmpeg.
# Spawns a segmented recording, hard-kills it, then immediately tries rename/move/delete on segments.
param(
  [int]$RunSeconds = 13,
  [string]$Tag = "locktest_kill"
)
$ErrorActionPreference = "Continue"
$ffmpeg  = (Resolve-Path (Join-Path $PSScriptRoot "..\..\ffmpeg\ffmpeg-master-latest-win64-lgpl\bin\ffmpeg.exe")).Path
$logDir  = Join-Path $PSScriptRoot "logs"
$outDir  = Join-Path $env:TEMP ("opencode\s5\" + $Tag)
if (Test-Path $outDir) { Remove-Item "$outDir\*" -Recurse -Force -ErrorAction SilentlyContinue }
New-Item -ItemType Directory -Force -Path $outDir, $logDir | Out-Null

$fargs = @("-hide_banner","-nostdin","-re",
  "-f","lavfi","-i","testsrc2=size=1920x1080:rate=30",
  "-f","lavfi","-i","sine=frequency=440:sample_rate=48000",
  "-f","lavfi","-i","sine=frequency=880:sample_rate=48000",
  "-map","0:v","-map","1:a","-map","2:a",
  "-c:v","h264_qsv","-g","60","-c:a","aac",
  "-f","segment","-segment_time","5","-reset_timestamps","1","-segment_format","matroska",
  (Join-Path $outDir "out_%03d.mkv"))
$argline = $fargs -join " "
$argline | Out-File (Join-Path $logDir "$Tag`_cmd.txt") -Encoding utf8

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $ffmpeg; $psi.Arguments = $argline
$psi.UseShellExecute = $false
$psi.RedirectStandardError = $true; $psi.RedirectStandardOutput = $true
$p = New-Object System.Diagnostics.Process; $p.StartInfo = $psi
[void]$p.Start()
$errTask = $p.StandardError.ReadToEndAsync(); $outTask = $p.StandardOutput.ReadToEndAsync()
Start-Sleep -Seconds $RunSeconds
$tk = & taskkill /F /PID $p.Id 2>&1 | Out-String
$reap = [System.Diagnostics.Stopwatch]::StartNew()
[void]$p.WaitForExit(15000)
$reap.Stop()
"kill: $($tk.Trim()) reap_ms=$($reap.ElapsedMilliseconds)" | Tee-Object -FilePath (Join-Path $logDir "$Tag`_meta.txt")

# lock probes immediately after reap
$segs = Get-ChildItem $outDir -Filter "out_*.mkv" | Sort-Object Name
$results = @()
New-Item -ItemType Directory -Force -Path (Join-Path $outDir "_probe") | Out-Null
foreach ($seg in $segs) {
  # 1) in-place rename
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $renamed = $true; $renErr = ""
  try { Rename-Item $seg.FullName ($seg.Name + ".t") -ErrorAction Stop } catch { $renamed = $false; $renErr = $_.Exception.Message }
  $sw.Stop()
  if ($renamed) { Rename-Item (Join-Path $outDir ($seg.Name + ".t")) $seg.Name }
  # 2) move out and back
  $sw2 = [System.Diagnostics.Stopwatch]::StartNew()
  $moved = $true; $movErr = ""
  try { Move-Item $seg.FullName (Join-Path $outDir ("_probe\" + $seg.Name)) -ErrorAction Stop } catch { $moved = $false; $movErr = $_.Exception.Message }
  $sw2.Stop()
  if ($moved) { Move-Item (Join-Path $outDir ("_probe\" + $seg.Name)) $seg.FullName }
  # 3) delete copy (do NOT delete originals; test delete on a COPY to prove read-lock state)
  $copied = $true
  try { Copy-Item $seg.FullName (Join-Path $outDir ("_probe\" + $seg.Name + ".copy")) -Force -ErrorAction Stop } catch { $copied = $false }
  $del = $true; $delErr = ""
  if ($copied) { try { Remove-Item (Join-Path $outDir ("_probe\" + $seg.Name + ".copy")) -Force -ErrorAction Stop } catch { $del = $false; $delErr = $_.Exception.Message } }
  $results += [pscustomobject]@{ segment=$seg.Name; rename_ok=$renamed; rename_ms=$sw.ElapsedMilliseconds; rename_err=$renErr
    move_ok=$moved; move_ms=$sw2.ElapsedMilliseconds; move_err=$movErr; copy_delete_ok=$del; del_err=$delErr }
}
Remove-Item (Join-Path $outDir "_probe") -Recurse -Force -ErrorAction SilentlyContinue
$results | Format-Table -AutoSize | Out-String -Width 220
$results | ConvertTo-Json | Out-File (Join-Path $logDir "$Tag`_locks.json") -Encoding utf8
exit 0
