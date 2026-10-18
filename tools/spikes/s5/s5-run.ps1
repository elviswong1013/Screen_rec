# S5 spike runner: segmented MKV recording + hard-kill (taskkill /F) or graceful 'q' stop.
# Usage: .\s5-run.ps1 -Mode kill -Encoder h264_qsv -SegmentTime 5 -RunSeconds 13 -Tag crash_qsv_seg5 [-ForceKeyFrames]
# Outputs: segments in %TEMP%\opencode\s5\<Tag>\ ; logs in tools\spikes\s5\logs\<Tag>_*.log
param(
  [Parameter(Mandatory=$true)][ValidateSet("kill","grace")][string]$Mode,
  [Parameter(Mandatory=$true)][string]$Encoder,
  [Parameter(Mandatory=$true)][int]$SegmentTime,
  [Parameter(Mandatory=$true)][int]$RunSeconds,
  [Parameter(Mandatory=$true)][string]$Tag,
  [switch]$ForceKeyFrames,
  [switch]$RealTime   # add -re: pace lavfi input at native rate (content time ~= wall time, production-like)
)
$ErrorActionPreference = "Stop"
$ffprobe = Join-Path $PSScriptRoot "..\..\ffmpeg\ffmpeg-master-latest-win64-lgpl\bin\ffprobe.exe"
$ffmpeg  = Join-Path $PSScriptRoot "..\..\ffmpeg\ffmpeg-master-latest-win64-lgpl\bin\ffmpeg.exe"
$ffmpeg  = (Resolve-Path $ffmpeg).Path
$ffprobe = (Resolve-Path $ffprobe).Path
$logDir  = Join-Path $PSScriptRoot "logs"
$outDir  = Join-Path $env:TEMP ("opencode\s5\" + $Tag)
if (Test-Path $outDir) { Remove-Item "$outDir\*" -Recurse -Force -ErrorAction SilentlyContinue }
New-Item -ItemType Directory -Force -Path $outDir, $logDir | Out-Null

# baseline command from task spec (grace mode drops -nostdin so 'q' on stdin works)
$fargs = @("-hide_banner")
if ($RealTime) { $fargs += "-re" }
$fargs += @(
  "-f","lavfi","-i","testsrc2=size=1920x1080:rate=30",
  "-f","lavfi","-i","sine=frequency=440:sample_rate=48000",
  "-f","lavfi","-i","sine=frequency=880:sample_rate=48000",
  "-map","0:v","-map","1:a","-map","2:a",
  "-c:v",$Encoder,"-g","60",
  "-c:a","aac"
)
if ($ForceKeyFrames) { $fargs += @("-force_key_frames","expr:gte(t,n_forced*$SegmentTime)") }
$fargs += @(
  "-f","segment","-segment_time","$SegmentTime","-reset_timestamps","1","-segment_format","matroska",
  (Join-Path $outDir "out_%03d.mkv")
)
if (-not ($Mode -eq "grace")) { $fargs = @("-nostdin") + $fargs }

$argline = ($fargs | ForEach-Object { if ($_ -match '[\s"]') { '"' + ($_ -replace '"','\"') + '"' } else { $_ } }) -join " "
$argline | Out-File (Join-Path $logDir "$Tag`_cmd.txt") -Encoding utf8

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $ffmpeg
$psi.Arguments = $argline
$psi.UseShellExecute = $false
$psi.RedirectStandardError  = $true
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardInput  = ($Mode -eq "grace")
$p = New-Object System.Diagnostics.Process
$p.StartInfo = $psi

$sw = [System.Diagnostics.Stopwatch]::StartNew()
[void]$p.Start()
$pid2 = $p.Id
# REQ-20: drain stderr/stdout continuously; undrained pipe buffer fills (~4KB) and blocks ffmpeg
$errTask = $p.StandardError.ReadToEndAsync()
$outTask = $p.StandardOutput.ReadToEndAsync()
"spawn_pid=$pid2 start_wall=$(Get-Date -Format o)" | Out-File (Join-Path $logDir "$Tag`_meta.txt") -Encoding utf8
Start-Sleep -Seconds $RunSeconds
$sw.Stop()
$killWall = Get-Date -Format o

if ($Mode -eq "kill") {
  $tk = & taskkill /F /PID $pid2 2>&1 | Out-String
  $sw2 = [System.Diagnostics.Stopwatch]::StartNew()
  [void]$p.WaitForExit(15000)
  $sw2.Stop()
  $exitCode = $p.ExitCode
  "kill_wall=$killWall kill_cmd='taskkill /F /PID $pid2' taskkill_output=$($tk.Trim()) reap_ms=$($sw2.ElapsedMilliseconds) exit_code=$exitCode" |
    Out-File (Join-Path $logDir "$Tag`_meta.txt") -Append -Encoding utf8
} else {
  $sw2 = [System.Diagnostics.Stopwatch]::StartNew()
  $p.StandardInput.Write("q")
  $p.StandardInput.Flush()
  [void]$p.WaitForExit(15000)
  $sw2.Stop()
  $exitCode = $p.ExitCode
  "grace_wall=$killWall stdin='q' exit_wait_ms=$($sw2.ElapsedMilliseconds) exit_code=$exitCode" |
    Out-File (Join-Path $logDir "$Tag`_meta.txt") -Append -Encoding utf8
}

[void]$errTask.Wait(5000)
$errTask.Result | Out-File (Join-Path $logDir "$Tag`_stderr.log") -Encoding utf8
[void]$outTask.Wait(5000)
$outTask.Result | Out-File (Join-Path $logDir "$Tag`_stdout.log") -Encoding utf8

# progress tail: last "time=" seen before kill (content time produced)
$prog = Select-String -Path (Join-Path $logDir "$Tag`_stderr.log") -Pattern "time=(\d+):(\d+):(\d+\.\d+)" |
  Select-Object -Last 1
$meta = Get-Content (Join-Path $logDir "$Tag`_meta.txt") | Out-String
Write-Host "=== meta ==="; Write-Host $meta
Write-Host "=== last progress ==="; if ($prog) { $prog.Line } else { Write-Host "(none)" }
Write-Host "=== files ==="
Get-ChildItem $outDir | Select-Object Name, Length, LastWriteTime | Format-Table -AutoSize | Out-String -Width 200
# exit code of this script run
exit 0
