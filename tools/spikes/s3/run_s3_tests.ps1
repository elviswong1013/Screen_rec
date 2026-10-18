# S3 spike test runner (spike tooling, NOT production code).
# Scenario matrix for "application (process-tree) loopback" audio isolation:
#   a      : only A playing -> record A -> 440 present, 880 <= -40 dB
#   b      : A+B playing -> device-mix control + record A (440 only) + record B (880 only)
#   c      : target stops mid-recording -> natural vs --pad-silence duration behaviour
#   d      : two concurrent clients in one process (record A and B simultaneously)
#   e      : include_tree vs exclude_tree semantics with a parent/child process pair
#   extra  : probe, phantom PID, silent PID, STA / pre-STA apartment behaviour
# Raw logs -> docs/spikes/s3/raw/. WAV/CSV -> temp dir (regenerable).
param(
  [ValidateSet('a', 'b', 'c', 'd', 'e', 'extra', 'all')][string]$Phase = 'all',
  [int]$Seconds = 8,
  [int]$KillAtS = 3
)

$ErrorActionPreference = 'Stop'
$repo = 'C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec'
$exe = Join-Path $repo 'tools\spikes\s3\apploop\target\release\apploop.exe'
$bin = Join-Path $repo 'tools\ffmpeg\ffmpeg-master-latest-win64-lgpl\bin'
$ffmpeg = Join-Path $bin 'ffmpeg.exe'
$ffplay = Join-Path $bin 'ffplay.exe'
$outD = 'C:\Users\elvis\AppData\Local\Temp\opencode\s3run'
$raw = Join-Path $repo 'docs\spikes\s3\raw'
New-Item -ItemType Directory -Force -Path $outD, $raw | Out-Null

$script:results = @()
$script:curPhase = ''
$script:cdur = @{}

function Q([string]$s) { if ($s -match '\s') { '"' + $s + '"' } else { $s } }
function Note([string]$m) { Write-Host $m }

function Assert([string]$name, [bool]$ok, [string]$detail) {
  $script:results += [pscustomobject]@{ phase = $script:curPhase; check = $name; ok = $ok; detail = $detail }
  $tag = if ($ok) { '[PASS]' } else { '[FAIL]' }
  Write-Host "$tag $name | $detail"
}

function Ensure-Tones {
  & $ffmpeg -hide_banner -v error -y -f lavfi -i 'sine=frequency=440:sample_rate=48000:duration=120' -ac 2 -c:a pcm_f32le "$outD\tone_a_440.wav"
  & $ffmpeg -hide_banner -v error -y -f lavfi -i 'sine=frequency=880:sample_rate=48000:duration=120' -ac 2 -c:a pcm_f32le "$outD\tone_b_880.wav"
}

function Write-Environment {
  $cv = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
  $lines = @(
    "date=$(Get-Date -Format o)"
    "product=$($cv.ProductName) display=$($cv.DisplayVersion) build=$($cv.CurrentBuildNumber).$($cv.UBR) ($([System.Environment]::OSVersion.Version))"
    "apploop_exe=$exe"
    "apploop_sha256=$((Get-FileHash $exe -Algorithm SHA256).Hash)"
    "apploop_bytes=$((Get-Item $exe).Length)"
    "wasapi_crate=0.26.0"
    "ffmpeg=$(& $ffmpeg -hide_banner -version 2>&1 | Select-Object -First 1)"
  )
  $lines | Out-File -Encoding utf8 "$raw\environment.txt"
}

function Start-Tone([string]$wav, [string]$tag) {
  $p = Start-Process -FilePath $ffplay -ArgumentList @('-nodisp', '-autoexit', '-hide_banner', '-v', 'error', '-i', $wav) -PassThru -WindowStyle Hidden
  Note "  ffplay[$tag] PID=$($p.Id)"
  Start-Sleep -Milliseconds 900
  return $p
}

function Stop-Procs($procs) {
  foreach ($p in $procs) { if ($null -ne $p) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue } }
}

function Start-Apploop([string]$tag, [string[]]$recordArgs) {
  $argList = @('record') + ($recordArgs | ForEach-Object { Q $_ })
  $p = Start-Process -FilePath $exe -ArgumentList $argList -PassThru -NoNewWindow `
    -RedirectStandardOutput "$raw\$tag.console.log" -RedirectStandardError "$raw\$tag.stderr.log"
  $null = $p.Handle  # cache handle so ExitCode is populated reliably on PS 5.1
  return $p
}

function Wait-Apploop($p, [string]$tag, [int]$timeoutS) {
  $exited = $p.WaitForExit($timeoutS * 1000)
  if (-not $exited) {
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    Note "  [WARN] $tag timeout ${timeoutS}s -> killed"
    return [pscustomobject]@{ TimedOut = $true; ExitCode = -1 }
  }
  return [pscustomobject]@{ TimedOut = $false; ExitCode = $p.ExitCode }
}

function Invoke-ApploopRun([string]$tag, [string[]]$recordArgs, [int]$timeoutS) {
  $p = Start-Apploop $tag $recordArgs
  return Wait-Apploop $p $tag $timeoutS
}

function Invoke-Analyze([string]$tag, [string]$wav) {
  $csv = "$outD\$tag.csv"
  $lines = & $exe analyze --input $wav --freqs 440,880 --threshold-db -40 --csv $csv 2>&1
  $lines | Out-File -Encoding utf8 "$raw\$tag.analysis.log"
  return $lines
}

function KeyLine($lines, [string]$pat) {
  $m = @($lines) | Select-String -SimpleMatch $pat | Select-Object -First 1
  if ($m) { $m.Line } else { '' }
}

function NumFrom([string]$line, [string]$key) {
  if ($line -match ([regex]::Escape($key) + '=(-?[0-9.]+)')) { [double]$Matches[1] } else { $null }
}

function DurationOf($lines) { NumFrom (KeyLine $lines 'BASIC frames=') 'duration_s' }

function Assert-ProcessIsolation([string]$label, $lines, [string]$refFreq, [string]$leakFreq) {
  $t = KeyLine $lines "TONE freq=$refFreq"
  $l = KeyLine $lines "LEAK freq=$leakFreq"
  $okTone = ($t -ne '') -and ((NumFrom $t 'db_rel_ref') -ge -3.0)
  $okLeak = ($l -ne '') -and ((NumFrom $l 'rel_db') -le -40.0)
  Assert "$label : tone $refFreq present" $okTone $t
  Assert "$label : leak $leakFreq <= -40 dB" $okLeak $l
}

function New-Spectrogram([string]$tag, [string]$wav) {
  if (Test-Path $wav) {
    & $ffmpeg -hide_banner -v error -y -i $wav -lavfi 'showspectrumpic=s=1200x600:legend=1' "$raw\$tag.spectrum.png"
  }
}

# ------------------------------------------------------------------ scenarios

function Run-PhaseA {
  $pa = Start-Tone "$outD\tone_a_440.wav" 'A'
  try {
    $r = Invoke-ApploopRun 'a_onlyA' @('--pid', "$($pa.Id)", '--out', "$outD\a_onlyA.wav", '--log', "$raw\a_onlyA.run.log") ($Seconds + 20)
    Assert 'a: record completed' ((-not $r.TimedOut) -and ($r.ExitCode -eq 0)) "exit=$($r.ExitCode) timedOut=$($r.TimedOut)"
    $lines = @(Invoke-Analyze 'a_onlyA' "$outD\a_onlyA.wav")
    Assert-ProcessIsolation 'a' $lines '440' '880'
    New-Spectrogram 'a_onlyA' "$outD\a_onlyA.wav"
  } finally { Stop-Procs @($pa) }
}

function Run-PhaseB {
  $pa = Start-Tone "$outD\tone_a_440.wav" 'A'
  $pb = Start-Tone "$outD\tone_b_880.wav" 'B'
  try {
    # Control: whole-system (device) loopback must contain BOTH tones.
    $r = Invoke-ApploopRun 'b_control_devmix' @('--pid', '0', '--device-loopback', '--out', "$outD\b_control_devmix.wav", '--log', "$raw\b_control_devmix.run.log") ($Seconds + 20)
    $lc = @(Invoke-Analyze 'b_control_devmix' "$outD\b_control_devmix.wav")
    $d440 = NumFrom (KeyLine $lc 'TONE freq=440') 'db_fs'
    $d880 = NumFrom (KeyLine $lc 'TONE freq=880') 'db_fs'
    $both = ($null -ne $d440) -and ($null -ne $d880)
    Assert 'b-control: device mix contains both tones' $both "440=$d440 dBFS, 880=$d880 dBFS, diff=$([math]::Round([math]::Abs($d440 - $d880), 2)) dB"
    New-Spectrogram 'b_control_devmix' "$outD\b_control_devmix.wav"

    # Per-process recordings while A+B are both playing.
    $r = Invoke-ApploopRun 'b_ab_recordA' @('--pid', "$($pa.Id)", '--out', "$outD\b_ab_recordA.wav", '--log', "$raw\b_ab_recordA.run.log") ($Seconds + 20)
    $la = @(Invoke-Analyze 'b_ab_recordA' "$outD\b_ab_recordA.wav")
    Assert-ProcessIsolation 'b-recordA' $la '440' '880'
    New-Spectrogram 'b_ab_recordA' "$outD\b_ab_recordA.wav"

    $r = Invoke-ApploopRun 'b_ab_recordB' @('--pid', "$($pb.Id)", '--out', "$outD\b_ab_recordB.wav", '--log', "$raw\b_ab_recordB.run.log") ($Seconds + 20)
    $lb = @(Invoke-Analyze 'b_ab_recordB' "$outD\b_ab_recordB.wav")
    Assert-ProcessIsolation 'b-recordB' $lb '880' '440'
    New-Spectrogram 'b_ab_recordB' "$outD\b_ab_recordB.wav"
  } finally { Stop-Procs @($pa, $pb) }
}

function Run-PhaseC {
  foreach ($pad in @($false, $true)) {
    $tag = if ($pad) { 'c_stop_padded' } else { 'c_stop_natural' }
    $pa = Start-Tone "$outD\tone_a_440.wav" 'A'
    $args = @('--pid', "$($pa.Id)", '--out', "$outD\$tag.wav", '--log', "$raw\$tag.run.log")
    if ($pad) { $args += '--pad-silence' }
    $p = Start-Apploop $tag $args
    Start-Sleep -Seconds $KillAtS
    Note "  [c] killing ffplay A at t~${KillAtS}s (tag=$tag)"
    Stop-Process -Id $pa.Id -Force -ErrorAction SilentlyContinue
    $r = Wait-Apploop $p $tag ($Seconds + 20)
    $lines = @(Invoke-Analyze $tag "$outD\$tag.wav")
    $script:cdur[$tag] = DurationOf $lines
    (Get-Content "$raw\$tag.run.log") | Select-String -Pattern 'HEARTBEAT|FLAG|TS STEP|RESULT|PAD:' | Out-File -Encoding utf8 "$raw\$tag.key.log"
    New-Spectrogram $tag "$outD\$tag.wav"
    Note "  [c] $tag duration=$($script:cdur[$tag])s exit=$($r.ExitCode)"
  }
  Assert 'c: natural duration preserved after target stops (stream keeps delivering packets)' ($script:cdur['c_stop_natural'] -ge ($Seconds - 0.1)) "duration=$($script:cdur['c_stop_natural'])s requested=${Seconds}s"
  Assert 'c: padded duration == requested (padding keeps timeline)' ([math]::Abs($script:cdur['c_stop_padded'] - $Seconds) -le 0.05) "duration=$($script:cdur['c_stop_padded'])s"
  $csv = Import-Csv "$outD\c_stop_natural.csv"
  $early = [double](($csv | Where-Object { [double]$_.t_mid_s -lt 3.0 } | Select-Object -First 1).db_440)
  $late = [double](($csv | Where-Object { [double]$_.t_mid_s -gt 4.0 } | Select-Object -First 1).db_440)
  Assert 'c: tone stops after target kill (content becomes digital silence)' (($early -gt -30) -and ($late -le -100)) "db_440 early=$early dBFS late=$late dBFS"
}

function Run-PhaseD {
  $pa = Start-Tone "$outD\tone_a_440.wav" 'A'
  $pb = Start-Tone "$outD\tone_b_880.wav" 'B'
  try {
    $wa = "$outD\d_dual_A.wav"
    $wb = "$outD\d_dual_B.wav"
    $r = Invoke-ApploopRun 'd_dual' @('--pid', "$($pa.Id)", '--out', $wa, '--pid', "$($pb.Id)", '--out', $wb) ($Seconds + 25)
    foreach ($pair in @(@($wa, 'd_dual_A'), @($wb, 'd_dual_B'))) {
      $src = [System.IO.Path]::ChangeExtension($pair[0], 'run.log')
      Copy-Item $src "$raw\$($pair[1]).run.log" -Force -ErrorAction SilentlyContinue
    }
    $la = @(Invoke-Analyze 'd_dual_A' $wa)
    $lb = @(Invoke-Analyze 'd_dual_B' $wb)
    Assert-ProcessIsolation 'd-dual-A' $la '440' '880'
    Assert-ProcessIsolation 'd-dual-B' $lb '880' '440'
    $act = @(Select-String -Path "$raw\d_dual.console.log" -Pattern 'ACTIVATE: ok' -ErrorAction SilentlyContinue).Count
    Assert 'd: both clients activated concurrently in one process' ($act -eq 2) "ACTIVATE ok lines=$act"
  } finally { Stop-Procs @($pa, $pb) }
}

function Run-PhaseE {
  $wavA = "$outD\tone_a_440.wav"
  $wrapper = "$outD\ffplay_a_host.cmd"
  "@echo off`r`n`"$ffplay`" -nodisp -autoexit -hide_banner -v error -i `"$wavA`"" | Out-File -Encoding ascii $wrapper
  $parent = Start-Process -FilePath $env:ComSpec -ArgumentList @('/c', (Q $wrapper)) -PassThru -WindowStyle Hidden
  Start-Sleep -Milliseconds 1200
  $child = Get-CimInstance Win32_Process -Filter "ParentProcessId=$($parent.Id)" -ErrorAction SilentlyContinue | Where-Object { $_.Name -eq 'ffplay.exe' } | Select-Object -First 1
  Note "  [e] parent(cmd) PID=$($parent.Id) child(ffplay) PID=$(if ($child) { $child.ProcessId } else { 'not found' })"
  $pb = Start-Tone "$outD\tone_b_880.wav" 'B'
  try {
    $r = Invoke-ApploopRun 'e_tree_include' @('--pid', "$($parent.Id)", '--out', "$outD\e_tree_include.wav", '--log', "$raw\e_tree_include.run.log") ($Seconds + 20)
    $li = @(Invoke-Analyze 'e_tree_include' "$outD\e_tree_include.wav")
    Assert-ProcessIsolation 'e-include(A tree)' $li '440' '880'
    $r = Invoke-ApploopRun 'e_tree_exclude' @('--pid', "$($parent.Id)", '--out', "$outD\e_tree_exclude.wav", '--log', "$raw\e_tree_exclude.run.log", '--exclude-tree') ($Seconds + 20)
    $le = @(Invoke-Analyze 'e_tree_exclude' "$outD\e_tree_exclude.wav")
    Assert-ProcessIsolation 'e-exclude(A tree)' $le '880' '440'
  } finally {
    Stop-Procs @($pb)
    if ($child) { Stop-Process -Id $child.ProcessId -Force -ErrorAction SilentlyContinue }
    Stop-Process -Id $parent.Id -Force -ErrorAction SilentlyContinue
  }
}

function Run-PhaseExtra {
  & $exe probe | Out-File -Encoding utf8 "$raw\probe.log"

  # 1) Phantom PID (no such process).
  $r = Invoke-ApploopRun 'extra_phantom_pid' @('--pid', '999999', '--out', "$outD\extra_phantom_pid.wav", '--log', "$raw\extra_phantom_pid.run.log") 25
  $act = (Select-String -Path "$raw\extra_phantom_pid.run.log" -Pattern 'ACTIVATE' -ErrorAction SilentlyContinue | Select-Object -First 1).Line
  Assert 'extra: phantom PID handled without hang' (-not $r.TimedOut) "$act (exit=$($r.ExitCode))"
  $pktLine = (Select-String -Path "$raw\extra_phantom_pid.run.log" -Pattern 'RESULT packets=' -ErrorAction SilentlyContinue | Select-Object -First 1).Line
  Assert 'extra: phantom PID activation succeeds -> no error signal (finding)' ($act -match 'ACTIVATE: ok') "$act ; $pktLine"

  # 2) pre-STA (simulated UI thread): STA first, then MTA must fail RPC_E_CHANGED_MODE.
  $pa = Start-Tone "$outD\tone_a_440.wav" 'A'
  try {
    $r = Invoke-ApploopRun 'extra_pre_sta' @('--pid', "$($pa.Id)", '--out', "$outD\extra_pre_sta.wav", '--log', "$raw\extra_pre_sta.run.log", '--pre-sta') 30
    $logText = Get-Content "$raw\extra_pre_sta.run.log" -Raw
    Assert 'extra: pre-STA thread -> MTA init returns RPC_E_CHANGED_MODE' ($logText -match 'initialize_mta\(\) hr=0x80010106') 'see extra_pre_sta.run.log'
    $actLine = (Select-String -Path "$raw\extra_pre_sta.run.log" -Pattern 'ACTIVATE' -ErrorAction SilentlyContinue | Select-Object -First 1).Line
    Assert 'extra: pre-STA activation outcome recorded' $true "$actLine (exit=$($r.ExitCode) timedOut=$($r.TimedOut))"
  } finally { Stop-Procs @($pa) }

  # 3) STA-only capture thread.
  $pa = Start-Tone "$outD\tone_a_440.wav" 'A'
  try {
    $r = Invoke-ApploopRun 'extra_sta' @('--pid', "$($pa.Id)", '--out', "$outD\extra_sta.wav", '--log', "$raw\extra_sta.run.log", '--sta') 30
    $logText = Get-Content "$raw\extra_sta.run.log" -Raw
    $actLine = (Select-String -Path "$raw\extra_sta.run.log" -Pattern 'ACTIVATE' -ErrorAction SilentlyContinue | Select-Object -First 1).Line
    Assert 'extra: STA-only capture outcome recorded' $true "sta_ok=$($logText -match 'initialize_sta\(\) hr=0x00000000') $actLine (exit=$($r.ExitCode) timedOut=$($r.TimedOut))"
  } finally { Stop-Procs @($pa) }

  # 4) Silent PID (process exists, renders no audio).
  $silent = Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile', '-Command', 'Start-Sleep', '60') -PassThru -WindowStyle Hidden
  try {
    $r = Invoke-ApploopRun 'extra_silent_pid' @('--pid', "$($silent.Id)", '--out', "$outD\extra_silent_pid.wav", '--log', "$raw\extra_silent_pid.run.log") ($Seconds + 20)
    $pktLine = (Select-String -Path "$raw\extra_silent_pid.run.log" -Pattern 'RESULT packets=' -ErrorAction SilentlyContinue | Select-Object -First 1).Line
    $flagLine = (Select-String -Path "$raw\extra_silent_pid.run.log" -Pattern 'RESULT flags' -ErrorAction SilentlyContinue | Select-Object -First 1).Line
    $frames = NumFrom $pktLine 'frames'
    Assert 'extra: silent PID -> full-rate packets without render session (no SILENT flag)' (($frames -gt 0) -and (($flagLine -match 'silent_packets=0'))) "$pktLine | $flagLine"
    $lines = @(Invoke-Analyze 'extra_silent_pid' "$outD\extra_silent_pid.wav")
    $peak = NumFrom (KeyLine $lines 'BASIC peak_dbfs=') 'peak_dbfs'
    Assert 'extra: silent PID recording is digital silence' ($null -ne $peak -and $peak -le -100) "peak_dbfs=$peak"
  } finally { Stop-Process -Id $silent.Id -Force -ErrorAction SilentlyContinue }
}

function Write-Summary {
  $script:results | Format-Table -AutoSize | Out-String -Width 220 | Write-Host
  $script:results | ConvertTo-Csv -NoTypeInformation | Out-File -Encoding utf8 "$raw\_summary.csv"
  $fails = @($script:results | Where-Object { -not $_.ok }).Count
  Write-Host "TOTAL $($script:results.Count) checks, $fails failed"
  $digest = foreach ($f in Get-ChildItem "$raw\*.analysis.log" | Sort-Object Name) {
    "== $($f.Name) =="
    Get-Content $f.FullName | Where-Object { $_ -match 'BASIC|FFT|TONE|FLOOR|GLOBAL_PEAK|LEAK|VERDICT' }
  }
  $digest | Out-File -Encoding utf8 "$raw\_evidence_fft.txt"
}

# ---------------------------------------------------------------------- main

if (-not (Test-Path $exe)) { throw "apploop.exe not found: $exe (build the crate first)" }

Ensure-Tones
Write-Environment
$phases = if ($Phase -eq 'all') { @('a', 'b', 'c', 'd', 'e', 'extra') } else { @($Phase) }
foreach ($ph in $phases) {
  $script:curPhase = $ph
  Note "===== PHASE $ph ====="
  switch ($ph) {
    'a' { Run-PhaseA }
    'b' { Run-PhaseB }
    'c' { Run-PhaseC }
    'd' { Run-PhaseD }
    'e' { Run-PhaseE }
    'extra' { Run-PhaseExtra }
  }
}
Write-Summary
