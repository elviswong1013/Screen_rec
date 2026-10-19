# ============================================================================
# tools/e2e/region-e2e.ps1 — 区域框选截图端到端验收（真实 GUI / 真实输入）
#
# 覆盖 docs/milestones/m12-a-report.md §2.4.1 的人工步骤，自动化为可重复脚本：
#   a) 底层放已知图案窗口 → 打开覆盖层（真实点击「截图 → 框选区域截图」）→ 真实拖拽
#      → 断言 PNG 落盘、物理尺寸 = 拖拽矩形、像素与覆盖前底层完全一致（无遮罩/边框污染）；
#   b) Esc → 无新文件、无残留 region-overlay 窗口、应用仍存活；
#   c) 输出目录不可写（路径中间是普通文件）→ 覆盖层保持可见并显示错误、无静默失败。
#
# 输入注入：优先 SendInput；部分会话/钩子会静默丢弃 SendInput（返回 1 但光标不动），
# 脚本启动时自检，必要时回退到 Win32 mouse_event/keybd_event（同样是 OS 级真实输入，
# 非 webview 内部脚本调用）。
#
# 前置：Windows 桌面会话（可见桌面）、主显示器 150% 缩放、应用主窗口 1024x700 逻辑尺寸。
# 用法：
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools/e2e/region-e2e.ps1 `
#     -EvidenceDir docs/milestones/m12-d-evidence
# ============================================================================
[CmdletBinding()]
param(
  [string] $AppExe = '',
  [string] $EvidenceDir = '',
  [ValidateSet('all', 'draw', 'esc', 'failure', 'record')] [string] $Scenario = 'all',
  [int[]] $DragRect = @(600, 300, 900, 600), # x, y, w, h（物理像素；宽高按 3 的倍数便于 150% 下 CSS 整数）
  [switch] $KeepAppRunning
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Import-Module (Join-Path $PSScriptRoot 'RegionE2E.psm1') -Force
Initialize-E2E

if (-not $AppExe) {
  $release = Join-Path $repoRoot 'target\release\screen-rec.exe'
  $debug = Join-Path $repoRoot 'target\debug\screen-rec.exe'
  $AppExe = if (Test-Path $release) { $release } else { $debug }
}
if (-not (Test-Path $AppExe)) { throw "找不到应用可执行文件：$AppExe" }
if (-not $EvidenceDir) { $EvidenceDir = Join-Path $repoRoot 'docs\milestones\m12-d-evidence' }
New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
$work = Join-Path $env:TEMP 'screen-rec-e2e'
New-Item -ItemType Directory -Force -Path $work | Out-Null

$dragX, $dragY, $dragW, $dragH = $DragRect
$cfgPath = Join-Path $env:LOCALAPPDATA 'com.screenrec.desktop\settings.json'
$targetStop = Join-Path $work 'target.stop'
$results = New-Object System.Collections.ArrayList

function Add-Result {
  param([string] $Name, [bool] $Pass, [string] $Detail, [string] $Evidence = '')
  [void]$results.Add([pscustomobject]@{ Scenario = $Name; Result = $(if ($Pass) { 'PASS' } else { 'FAIL' }); Detail = $Detail; Evidence = $Evidence })
  Write-Host ("[{0}] {1} — {2}" -f $(if ($Pass) { 'PASS' } else { 'FAIL' }), $Name, $Detail)
}

function Start-TargetWindow {
  param([int] $X, [int] $Y, [int] $W, [int] $H)
  Remove-Item "$targetStop*" -ErrorAction SilentlyContinue
  $script = Join-Path $PSScriptRoot 'New-SolidTarget.ps1'
  $argLine = "-NoProfile -STA -ExecutionPolicy Bypass -File `"$script`" -X $X -Y $Y -Width $W -Height $H -StopFile `"$targetStop`""
  $p = Start-Process -FilePath 'powershell.exe' -ArgumentList $argLine -PassThru
  $deadline = [DateTime]::UtcNow.AddSeconds(15)
  while ([DateTime]::UtcNow -lt $deadline) {
    if (Test-Path "$targetStop.rect") { return $p }
    Start-Sleep -Milliseconds 200
  }
  throw '底层目标窗口启动超时'
}

function Stop-TargetWindow {
  param($Process)
  New-Item -ItemType File -Force -Path $targetStop | Out-Null
  Start-Sleep -Milliseconds 800
  if ($Process -and -not $Process.HasExited) { $Process.Kill() }
}

function Get-OutputDir {
  return (Get-Content $cfgPath -Raw | ConvertFrom-Json).output_dir
}

function Restore-Settings {
  if ($script:settingsBackup -and (Test-Path $script:settingsBackup)) {
    Copy-Item $script:settingsBackup $cfgPath -Force
  }
}

function Compare-PngToBaseline {
  param([string] $PngPath, [string] $BaselinePath)
  Add-Type -AssemblyName System.Drawing
  $img = [System.Drawing.Bitmap]::FromFile($PngPath)
  $base = [System.Drawing.Bitmap]::FromFile($BaselinePath)
  try {
    if ($img.Width -ne $base.Width -or $img.Height -ne $base.Height) {
      return [pscustomobject]@{ Pass = $false; Detail = "尺寸不一致 png=$($img.Width)x$($img.Height) baseline=$($base.Width)x$($base.Height)"; MeanDiff = -1; MaxDiff = -1; Changed = -1 }
    }
    $n = 0; $sum = 0; $max = 0; $changed = 0
    for ($y = 0; $y -lt $img.Height; $y++) {
      for ($x = 0; $x -lt $img.Width; $x++) {
        $a = $img.GetPixel($x, $y); $b = $base.GetPixel($x, $y)
        $d = [math]::Abs($a.R - $b.R) + [math]::Abs($a.G - $b.G) + [math]::Abs($a.B - $b.B)
        $sum += $d; if ($d -gt $max) { $max = $d }; if ($d -gt 6) { $changed++ }; $n++
      }
    }
    $mean = $sum / [math]::Max(1, $n)
    [pscustomobject]@{
      Pass = ($mean -le 1.0 -and $max -le 6 -and $changed -eq 0)
      Detail = "meanDiff=$([math]::Round($mean,4)) maxDiff=$max changedPixels=$changed/$n"
      MeanDiff = $mean; MaxDiff = $max; Changed = $changed
    }
  } finally {
    $img.Dispose(); $base.Dispose()
  }
}

function Get-NewRegionShots {
  param([string[]] $Before)
  $dir = Get-OutputDir
  if (-not (Test-Path $dir)) { return @() }
  @(Get-ChildItem $dir -Filter 'shot_region_*.png' -ErrorAction SilentlyContinue |
    Where-Object { $Before -notcontains $_.FullName } |
    Select-Object -ExpandProperty FullName)
}

function Wait-NewRegionShot {
  param([string[]] $Before, [int] $TimeoutSec = 15)
  $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSec)
  while ([DateTime]::UtcNow -lt $deadline) {
    $new = @(Get-NewRegionShots -Before $Before)
    if ($new.Count -gt 0) { return $new[0] }
    Start-Sleep -Milliseconds 250
  }
  return $null
}

function Wait-OverlayGone {
  param([int] $TimeoutMs = 8000)
  return (Wait-NoTopLevelWindow -Title 'region-overlay' -TimeoutMs $TimeoutMs)
}

function Get-OverlayWindows {
  @(Get-TopLevelWindows | Where-Object { $_.Title -eq 'region-overlay' })
}

function Test-ErrorHintVisible {
  <#
    .SYNOPSIS 覆盖层错误提示（.region-hint.err）在窗口顶部中央呈现红色系提示条。
    .DESCRIPTION 只扫描覆盖层顶部中央区域（提示条位置），统计“红色占优”像素数量；
                 正常提示条为深灰蓝，红色占优像素极少。
  #>
  param([Parameter(Mandatory)] [string] $ScreenshotPath)
  Add-Type -AssemblyName System.Drawing
  $bmp = [System.Drawing.Bitmap]::FromFile($ScreenshotPath)
  try {
    $hits = 0
    $x0 = [int]($bmp.Width * 0.30)
    $x1 = [int]($bmp.Width * 0.70)
    for ($x = $x0; $x -lt $x1; $x += 2) {
      for ($y = 0; $y -lt [math]::Min(140, $bmp.Height); $y += 2) {
        $c = $bmp.GetPixel($x, $y)
        $redness = [int]$c.R - [int](($c.G + $c.B) / 2)
        if ($redness -gt 25 -and $c.R -gt 60) { $hits++ }
      }
    }
    return $hits
  } finally { $bmp.Dispose() }
}

function Get-MediaFiles {
  param([string] $Dir)
  if (-not (Test-Path $Dir)) { return @() }
  @(Get-ChildItem $Dir -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Extension -in '.mkv', '.mp4' } |
    Select-Object -ExpandProperty FullName)
}

function Wait-NewMediaFile {
  param([string] $Dir, [string[]] $Before, [int] $TimeoutSec = 40)
  $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSec)
  while ([DateTime]::UtcNow -lt $deadline) {
    $new = @(Get-MediaFiles -Dir $Dir | Where-Object { $Before -notcontains $_ })
    if ($new.Count -gt 0) { return $new[0] }
    Start-Sleep -Milliseconds 300
  }
  return $null
}

function Get-BandMeanDb {
  param([string] $Ffmpeg, [string] $File, [int] $Hz)
  $ffArgs = @('-hide_banner', '-nostdin', '-i', $File, '-vn', '-af', "bandpass=f=${Hz}:width_type=h:w=20,volumedetect", '-f', 'null', '-')
  $argLine = ($ffArgs | ForEach-Object {
      if ($_ -match '\s') { '"' + ($_ -replace '"', '\"') + '"' } else { $_ }
    }) -join ' '
  $psi = New-Object System.Diagnostics.ProcessStartInfo
  $psi.FileName = $Ffmpeg
  $psi.Arguments = $argLine
  $psi.UseShellExecute = $false
  $psi.RedirectStandardError = $true
  $psi.RedirectStandardOutput = $true
  $psi.CreateNoWindow = $true
  $p = [System.Diagnostics.Process]::Start($psi)
  $err = $p.StandardError.ReadToEnd()
  [void]$p.StandardOutput.ReadToEnd()
  $p.WaitForExit()
  $m = [regex]::Match($err, 'mean_volume:\s*(-?[0-9.]+)\s*dB')
  if (-not $m.Success) { return [double]::NegativeInfinity }
  return [double]$m.Groups[1].Value
}

function Get-GoertzelDb {
  <# 单频点 DFT（Goertzel，等价 FFT 的一个 bin）：真实成品音轨内容判据。 #>
  param(
    [Parameter(Mandatory)] [string] $Ffmpeg,
    [Parameter(Mandatory)] [string] $File,
    [Parameter(Mandatory)] [int] $Hz,
    [int] $Rate = 48000
  )
  $tmp = Join-Path $work ("goertzel-" + [Guid]::NewGuid().ToString('N') + ".f32")
  try {
    & $Ffmpeg -v error -i $File -vn -map 0:a:0 -f f32le -ac 1 -ar $Rate -y $tmp
    if (-not (Test-Path $tmp)) { return [double]::NegativeInfinity }
    $bytes = [System.IO.File]::ReadAllBytes($tmp)
    $n = [int][Math]::Floor($bytes.Length / 4)
    if ($n -lt ($Rate / 2)) { return [double]::NegativeInfinity }
    $use = [Math]::Min($n, $Rate * 5)
    $start = [int](($n - $use) / 2)
    $coeff = 2.0 * [Math]::Cos(2.0 * [Math]::PI * $Hz / $Rate)
    $s1 = 0.0; $s2 = 0.0
    for ($i = 0; $i -lt $use; $i++) {
      $s = [BitConverter]::ToSingle($bytes, ($start + $i) * 4) + $coeff * $s1 - $s2
      $s2 = $s1; $s1 = $s
    }
    $p = $s1 * $s1 + $s2 * $s2 - $coeff * $s1 * $s2
    return 10.0 * [Math]::Log10([Math]::Max($p / $use, 1e-20))
  } finally {
    Remove-Item $tmp -Force -ErrorAction SilentlyContinue
  }
}

$script:settingsBackup = Join-Path $work 'settings.backup.json'
$targetProcess = $null
$appProcess = $null
$toneProcess = $null
$tone660Process = $null
$tone880Process = $null
$summaryPath = Join-Path $EvidenceDir 'summary.md'

try {
  # ── 环境与清理（覆盖层会临时撑大虚拟屏幕，先关掉旧实例再量基准尺寸） ─────────
  if (Test-Path $cfgPath) { Copy-Item $cfgPath $script:settingsBackup -Force }
  Get-Process screen-rec -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Milliseconds 900
  $vs = Get-MonitorUnion
  Write-Host "显示器并集（物理）：$($vs.X),$($vs.Y) $($vs.Width)x$($vs.Height)"

  # ── a) 框选 → PNG；b) Esc ──────────────────────────────────────────────
  if ($Scenario -in @('all', 'draw', 'esc')) {
    $targetProcess = Start-TargetWindow -X $dragX -Y $dragY -W $dragW -H $dragH
    Write-Host "底层目标窗口物理矩形: $((Get-Content "$targetStop.rect" -Raw).Trim())"
    $baseline = Save-ScreenBitmap -Path (Join-Path $EvidenceDir 'baseline.png') -X $dragX -Y $dragY -Width $dragW -Height $dragH
    Write-Host "基线截图: $baseline"

    $deskBefore = Save-ScreenBitmap -Path (Join-Path $work 'desktop-before.png') -X $vs.X -Y $vs.Y -Width $vs.Width -Height $vs.Height
    $statsBefore = Get-ImageStats -Path $deskBefore -Step 19

    $appProcess = Start-Process -FilePath $AppExe -PassThru
    Start-Sleep -Seconds 6
    # 主窗口挪到底层目标右侧，避免挡住目标区域（目标窗口 TopMost，会盖住主窗口）
    $appWin = Find-AppWindow -ProcessId $appProcess.Id
    Move-Window -Handle $appWin.Handle -X ($dragX + $dragW + 40) -Y $dragY
    $open = Open-RegionOverlayFromUi -ProcessId $appProcess.Id -WorkDir 'm12d'
    $overlay = $open.Overlay
    Start-Sleep -Milliseconds 1200

    $children = Get-ChildWindows -Handle $overlay.Handle
    $webviewReady = @($children | Where-Object { $_.Class -eq 'Chrome_WidgetWin_1' -and $_.Visible }).Count -gt 0
    Add-Result -Name 'a0. 覆盖层 WebView2 初始化' -Pass $webviewReady `
      -Detail ("overlay=" + "$($overlay.Rect.Width)x$($overlay.Rect.Height)" + "；children=" + (($children | ForEach-Object { "$($_.Class)[$($_.Visible)]" }) -join ', ')) `
      -Evidence (Join-Path $EvidenceDir 'overlay-mask.png')

    $deskMask = Save-ScreenBitmap -Path (Join-Path $EvidenceDir 'overlay-mask.png') -X $vs.X -Y $vs.Y -Width $vs.Width -Height $vs.Height
    $statsMask = Get-ImageStats -Path $deskMask -Step 19
    $ratio = if ($statsBefore.Mean -gt 0) { $statsMask.Mean / $statsBefore.Mean } else { 0 }
    Add-Result -Name 'a1. 覆盖层遮罩真实绘制' -Pass ($ratio -lt 0.75) `
      -Detail "桌面亮度均值 $($statsBefore.Mean) → $($statsMask.Mean)（ratio=$([math]::Round($ratio,3))；遮罩 alpha=0.55）" `
      -Evidence $deskMask

    $outDir = Get-OutputDir
    $before = @(Get-ChildItem $outDir -Filter 'shot_region_*.png' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty FullName)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    Send-Drag -X1 $dragX -Y1 $dragY -X2 ($dragX + $dragW) -Y2 ($dragY + $dragH) -Steps 16 -StepDelayMs 30
    $png = Wait-NewRegionShot -Before $before -TimeoutSec 15
    $closed = Wait-OverlayGone -TimeoutMs 8000
    $sw.Stop()
    if (-not $png) {
      Add-Result -Name 'a2. 拖拽生成 PNG' -Pass $false -Detail '15s 内输出目录无新 shot_region_*.png' -Evidence $deskMask
    } else {
      $copy = Join-Path $EvidenceDir 'region-shot.png'
      Copy-Item $png $copy -Force
      Add-Type -AssemblyName System.Drawing
      $img = [System.Drawing.Bitmap]::FromFile($png)
      $sizeOk = ($img.Width -eq $dragW -and $img.Height -eq $dragH)
      $sizeText = "$($img.Width)x$($img.Height)（期望 $($dragW)x$($dragH)）"
      $img.Dispose()
      Add-Result -Name 'a2. 拖拽生成 PNG' -Pass $true -Detail "$png（覆盖层已关闭=$closed，耗时 $($sw.ElapsedMilliseconds)ms）" -Evidence $copy
      Add-Result -Name 'a3. PNG 物理尺寸 = 拖拽矩形' -Pass $sizeOk -Detail $sizeText -Evidence $copy
      $cmp = Compare-PngToBaseline -PngPath $png -BaselinePath $baseline
      Add-Result -Name 'a4. 成品 = 覆盖前底层（无遮罩/边框污染）' -Pass $cmp.Pass -Detail $cmp.Detail -Evidence $copy
    }

    if ($Scenario -in @('all', 'esc')) {
      $beforeEsc = @(Get-ChildItem $outDir -Filter 'shot_region_*.png' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty FullName)
      $null = Open-RegionOverlayFromUi -ProcessId $appProcess.Id -WorkDir 'm12d'
      Start-Sleep -Milliseconds 900
      Send-Key -VirtualKey 0x1B # VK_ESCAPE
      $gone = Wait-OverlayGone -TimeoutMs 8000
      Start-Sleep -Milliseconds 700
      $afterEsc = @(Get-ChildItem $outDir -Filter 'shot_region_*.png' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty FullName)
      $escNew = @($afterEsc | Where-Object { $beforeEsc -notcontains $_ })
      $alive = [bool](Get-Process -Id $appProcess.Id -ErrorAction SilentlyContinue)
      $residual = (Get-OverlayWindows).Count
      $escShot = Save-ScreenBitmap -Path (Join-Path $EvidenceDir 'after-esc.png') -X $vs.X -Y $vs.Y -Width $vs.Width -Height $vs.Height
      Add-Result -Name 'b1. Esc 关闭、无新文件、无残留窗口' -Pass ($gone -and $escNew.Count -eq 0 -and $alive -and $residual -eq 0) `
        -Detail "gone=$gone newPng=$($escNew.Count) appAlive=$alive residualOverlay=$residual" -Evidence $escShot
    }
  }

  # ── c) 失败路径（输出目录不可写） ─────────────────────────────────────────
  if ($Scenario -in @('all', 'failure')) {
    if ($Scenario -eq 'all') {
      Get-Process -Id $appProcess.Id -ErrorAction SilentlyContinue | Stop-Process -Force
      Start-Sleep -Milliseconds 900
    }
    $blocker = Join-Path $work 'blocker.file'
    New-Item -ItemType File -Force -Path $blocker | Out-Null
    $badDir = Join-Path $blocker 'sub'
    $settings = Get-Content $cfgPath -Raw | ConvertFrom-Json
    $settings.output_dir = $badDir
    [System.IO.File]::WriteAllText($cfgPath, ($settings | ConvertTo-Json -Depth 6), [System.Text.UTF8Encoding]::new($false))
    try {
      Get-Process screen-rec -ErrorAction SilentlyContinue | Stop-Process -Force
      Start-Sleep -Milliseconds 900
      $appProcess = Start-Process -FilePath $AppExe -PassThru
      Start-Sleep -Seconds 6
      $appWin = Find-AppWindow -ProcessId $appProcess.Id
      Move-Window -Handle $appWin.Handle -X ($dragX + $dragW + 40) -Y $dragY
      $null = Open-RegionOverlayFromUi -ProcessId $appProcess.Id -WorkDir 'm12d'
      Start-Sleep -Milliseconds 900
      Send-Drag -X1 $dragX -Y1 $dragY -X2 ($dragX + 480) -Y2 ($dragY + 360) -Steps 10 -StepDelayMs 30
      Start-Sleep -Seconds 3
      $ovStill = Get-OverlayWindows | Where-Object { $_.Visible }
      $failShot = Save-ScreenBitmap -Path (Join-Path $EvidenceDir 'failure-output-unavailable.png') -X $vs.X -Y $vs.Y -Width $vs.Width -Height $vs.Height
      $errPixels = Test-ErrorHintVisible -ScreenshotPath $failShot
      $dirCreated = Test-Path $badDir
      $pass = ($null -ne $ovStill) -and ($errPixels -gt 20) -and (-not $dirCreated)
      Add-Result -Name 'c1. 失败时覆盖层保持可见并显示错误' -Pass $pass `
        -Detail "overlayVisible=$($null -ne $ovStill) errorHintPixels=$errPixels outputDirCreated=$dirCreated" -Evidence $failShot
      Send-Key -VirtualKey 0x1B
      $null = Wait-OverlayGone -TimeoutMs 6000
    } finally {
      Restore-Settings
    }
  }

  # ── d) 区域录制（REQ-21/22/23 贯通：真实拖拽 → 开录 → 成品尺寸/无覆盖层/轨数/音调） ──
  if ($Scenario -eq 'record') {
    $recX = $dragX + 240
    $recY = $dragY + 380
    $recW = 400
    $recH = 120
    $targetProcess = Start-TargetWindow -X $dragX -Y $dragY -W $dragW -H $dragH
    Write-Host "录制目标窗口物理矩形: $((Get-Content "$targetStop.rect" -Raw).Trim())"

    # 区域默认全音轨（系统正例）；隔离双应用子场景稍后用真实 UI 切到「隔离」。
    if (Test-Path $cfgPath) {
      $settings = Get-Content $cfgPath -Raw | ConvertFrom-Json
      $settings | Add-Member -NotePropertyName default_audio_mode -NotePropertyValue 'all_tracks' -Force
      $settings | Add-Member -NotePropertyName output_container -NotePropertyValue 'mkv' -Force
      $settings | Add-Member -NotePropertyName audio_sample_rate -NotePropertyValue 48000 -Force
      [System.IO.File]::WriteAllText($cfgPath, ($settings | ConvertTo-Json -Depth 6), [System.Text.UTF8Encoding]::new($false))
    }

    $ffmpeg = Join-Path $repoRoot 'tools\ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin\ffmpeg.exe'
    $ffprobe = Join-Path $repoRoot 'tools\ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin\ffprobe.exe'
    $ffplay = Join-Path $repoRoot 'tools\ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin\ffplay.exe'
    $toneWav660 = Join-Path $work 'tone-660.wav'
    $toneWav880 = Join-Path $work 'tone-880.wav'
    & $ffmpeg -y -hide_banner -loglevel error -f lavfi -i 'sine=frequency=660:sample_rate=48000:duration=30,volume=0.25' -ac 2 -c:a pcm_f32le $toneWav660
    & $ffmpeg -y -hide_banner -loglevel error -f lavfi -i 'sine=frequency=880:sample_rate=48000:duration=30,volume=0.25' -ac 2 -c:a pcm_f32le $toneWav880

    # 660 应用必须在 app 启动前播放：UI 音频源快照（进程 PID）才能列出它。
    Get-Process screen-rec -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 900
    $tone660Process = Start-Process -FilePath $ffplay -ArgumentList @('-nodisp', '-loop', '0', '-loglevel', 'quiet', $toneWav660) -PassThru
    Start-Sleep -Milliseconds 900
    $appOut = Join-Path $work 'app-stdout.log'
    $appErr = Join-Path $work 'app-stderr.log'
    $appProcess = Start-Process -FilePath $AppExe -PassThru -RedirectStandardOutput $appOut -RedirectStandardError $appErr
    Start-Sleep -Seconds 6
    $appWin = Find-AppWindow -ProcessId $appProcess.Id
    Move-Window -Handle $appWin.Handle -X ($dragX + $dragW + 40) -Y $dragY
    # 顶部置顶：避免用户桌面其它窗口覆盖主窗后拦截后续真实点击。
    [void][SrE2E.Native]::SetWindowPos($appWin.Handle, [IntPtr](-1), ($dragX + $dragW + 40), $dragY, 0, 0, 0x0001 -bor 0x0004 -bor 0x0010)

    $open = Open-RegionRecordFromUi -ProcessId $appProcess.Id -WorkDir 'm18r' -TolerancePx 6
    Start-Sleep -Milliseconds 1200
    Add-Result -Name 'd0. 区域录制覆盖层已打开' -Pass ($null -ne $open.Overlay) -Detail "overlay=$($open.Overlay.Rect.Width)x$($open.Overlay.Rect.Height)"

    $outDir = Get-OutputDir
    $beforeMedia = @(Get-MediaFiles -Dir $outDir)
    Send-Drag -X1 $recX -Y1 $recY -X2 ($recX + $recW) -Y2 ($recY + $recH) -Steps 14 -StepDelayMs 30 -TolerancePx 6
    $closed = Wait-OverlayGone -TimeoutMs 8000
    Add-Result -Name 'd1. 确认后覆盖层隐藏（DWM 合成窗口 ≥150ms）' -Pass $closed -Detail "gone=$closed"
    Start-Sleep -Milliseconds 1200
    $confirmed = Get-UiRegionSize -ProcessId $appProcess.Id
    Add-Result -Name 'd1b. 覆盖层确认的最终尺寸可读（精确校验基准）' -Pass ($null -ne $confirmed) `
      -Detail $(if ($confirmed) { "$($confirmed.Width)x$($confirmed.Height)" } else { '未读到区域尺寸文本' })

    # 启动（开始按钮为底部红色 .btn-rec；「全部停止」用 UIA 检测，避免页面滚动影响）。
    $started = $false
    for ($attempt = 1; $attempt -le 2 -and -not $started; $attempt++) {
      [void](Set-ForegroundWindow -Handle $open.AppWindow.Handle)
      [void](Set-UiScrollTop -ProcessId $appProcess.Id)
      Start-Sleep -Milliseconds 250
      $shot = Join-Path $work "record-main-$attempt.png"
      Save-ScreenBitmap -Path $shot -X $open.Client.X -Y $open.Client.Y -Width $open.Client.Width -Height $open.Client.Height | Out-Null
      $startBtn = Find-RedButton -ScreenshotPath $shot -MinY ([int]($open.Client.Height - 220))
      if (-not $startBtn) { continue }
      Write-Host "开始按钮坐标: $($open.Client.X + $startBtn.X),$($open.Client.Y + $startBtn.Y)"
      Send-Click -X ($open.Client.X + $startBtn.X) -Y ($open.Client.Y + $startBtn.Y) -TolerancePx 6
      Start-Sleep -Seconds 2
      $stopEl = Wait-UiButtonByName -ProcessId $appProcess.Id -Name '全部停止' -TimeoutMs 8000
      if ($stopEl) { $started = $true }
    }
    Add-Result -Name 'd2. 录制已启动（UIA 可见「全部停止」）' -Pass $started -Detail "started=$started"
    if ($started) {
      Start-Sleep -Seconds 7
      $stopInvoked = Invoke-UiButtonByName -ProcessId $appProcess.Id -Name '全部停止'
      if (-not $stopInvoked) {
        [void](Set-UiScrollTop -ProcessId $appProcess.Id)
        Start-Sleep -Milliseconds 250
        $shot3 = Join-Path $work 'record-stop.png'
        Save-ScreenBitmap -Path $shot3 -X $open.Client.X -Y $open.Client.Y -Width $open.Client.Width -Height $open.Client.Height | Out-Null
        $stopBtn = Find-RedButton -ScreenshotPath $shot3 -MaxY ([int]($open.Client.Height - 250))
        if ($stopBtn) { Send-Click -X ($open.Client.X + $stopBtn.X) -Y ($open.Client.Y + $stopBtn.Y) -TolerancePx 6 }
      }
      $final = Wait-NewMediaFile -Dir $outDir -Before $beforeMedia -TimeoutSec 45
      Add-Result -Name 'd3. 停止后产出成品' -Pass ($null -ne $final) -Detail "final=$final"
    } else {
      $final = $null
      Add-Result -Name 'd3. 停止后产出成品' -Pass $false -Detail '录制未启动'
    }
    if ($final) {
      $copy = Join-Path $EvidenceDir 'region-record.mkv'
      Copy-Item $final $copy -Force
      $dur = [double](& $ffprobe -v error -show_entries format=duration -of csv=p=0 $final)
      $fmt = (& $ffprobe -v error -show_entries format=format_name -of csv=p=0 $final) -join ''
      $vw = [int](& $ffprobe -v error -select_streams v:0 -show_entries stream=width -of csv=p=0 $final)
      $vh = [int](& $ffprobe -v error -select_streams v:0 -show_entries stream=height -of csv=p=0 $final)
      $aCount = @(& $ffprobe -v error -select_streams a -show_entries stream=index -of csv=p=0 $final | Where-Object { $_ -ne '' }).Count
      $sizeOk = ($null -ne $confirmed -and $vw -eq $confirmed.Width -and $vh -eq $confirmed.Height)
      Add-Result -Name 'd4. 成品尺寸 = 覆盖层确认的最终尺寸（精确）' -Pass $sizeOk `
        -Detail "${vw}x${vh}（确认 $($confirmed.Width)x$($confirmed.Height)；非整屏）"
      Add-Result -Name 'd5. MKV 容器 + 时长覆盖录制段（含 UIA 启动等待）' -Pass ($fmt -match 'matroska' -and $dur -ge 6.0 -and $dur -le 25.0) -Detail "format=$fmt dur=$([math]::Round($dur,2))s"
      Add-Result -Name 'd6. 全音轨（默认系统）恰 1 轨' -Pass ($aCount -eq 1) -Detail "audioTracks=$aCount"

      # 帧无覆盖层：真正解码第一帧 + 5s 后续帧；遮罩 alpha=0.55 会把绿色压暗（<170）。
      $frame1 = Join-Path $EvidenceDir 'record-first-frame.png'
      $frame2 = Join-Path $EvidenceDir 'record-frame-5s.png'
      & $ffmpeg -y -hide_banner -loglevel error -i $final -frames:v 1 $frame1
      & $ffmpeg -y -hide_banner -loglevel error -ss 5 -i $final -frames:v 1 $frame2
      Add-Type -AssemblyName System.Drawing
      $frameOk = $true
      $samples = @()
      foreach ($fp in @($frame1, $frame2)) {
        $bmp = [System.Drawing.Bitmap]::FromFile($fp)
        try {
          foreach ($pt in @(@([int]($bmp.Width/2), [int]($bmp.Height/2)), @([int]($bmp.Width/4), [int]($bmp.Height/4)))) {
            $c = $bmp.GetPixel($pt[0], $pt[1])
            $samples += "RGB($($c.R),$($c.G),$($c.B))"
            if (-not ($c.G -ge 170 -and $c.R -le 70 -and $c.B -le 70)) { $frameOk = $false }
          }
        } finally { $bmp.Dispose() }
      }
      Add-Result -Name 'd7. 真正第一帧与后续帧无覆盖层（纯绿目标未被遮罩压暗）' -Pass $frameOk -Detail ($samples -join ' ') -Evidence $frame1

      # 系统正例：只断言 660 存在；880 未播放，不再冒充负例。
      $db660 = Get-BandMeanDb -Ffmpeg $ffmpeg -File $final -Hz 660
      $db880Ref = Get-BandMeanDb -Ffmpeg $ffmpeg -File $final -Hz 880
      Add-Result -Name 'd8. 系统音轨含 660Hz 正例' -Pass ($db660 -gt -55) `
        -Detail "660Hz=$([math]::Round($db660,1))dB；880Hz=$([math]::Round($db880Ref,1))dB（未播放，仅参考，不作负例）"

      # ── d9+: 真实隔离双应用（660 应用被 UI 勾选；880 为另一应用进程，不勾选） ──
      $isolatedSelected = Select-UiRadioByPrefix -ProcessId $appProcess.Id -NamePrefix '隔离'
      Add-Result -Name 'd9. 真实 UI 切到「隔离」音轨模式' -Pass $isolatedSelected -Detail "selected=$isolatedSelected"
      if ($isolatedSelected) {
        $open2 = Open-RegionRecordFromUi -ProcessId $appProcess.Id -WorkDir 'm18r-iso' -TolerancePx 6
        Start-Sleep -Milliseconds 1200
        $confirmed2 = $null
        if ($open2.Overlay) {
          Send-Drag -X1 $recX -Y1 $recY -X2 ($recX + $recW) -Y2 ($recY + $recH) -Steps 14 -StepDelayMs 30 -TolerancePx 6
          $closed2 = Wait-OverlayGone -TimeoutMs 8000
          Add-Result -Name 'd9b. 隔离场景覆盖层确认/隐藏' -Pass $closed2 -Detail "gone=$closed2"
          Start-Sleep -Milliseconds 1200
          $confirmed2 = Get-UiRegionSize -ProcessId $appProcess.Id
        }
        $checkboxId = "cfg-region-app-$($tone660Process.Id)"
        $toggleOk = Invoke-UiToggle -ProcessId $appProcess.Id -AutomationId $checkboxId
        Add-Result -Name 'd10. UI 勾选 660 应用（UIA AutomationId = PID）' -Pass $toggleOk -Detail $checkboxId
        if ($toggleOk -and $open2.Overlay) {
          # 另一独立应用进程（880）在选中 660 之后开始播放，且不被勾选。
          $tone880Process = Start-Process -FilePath $ffplay -ArgumentList @('-nodisp', '-loop', '0', '-loglevel', 'quiet', $toneWav880) -PassThru
          Start-Sleep -Milliseconds 900
          $beforeMedia2 = @(Get-MediaFiles -Dir $outDir)
          $started2 = $false
          for ($attempt = 1; $attempt -le 2 -and -not $started2; $attempt++) {
            [void](Set-ForegroundWindow -Handle $open2.AppWindow.Handle)
            [void](Set-UiScrollTop -ProcessId $appProcess.Id)
            Start-Sleep -Milliseconds 250
            $shotIso = Join-Path $work "record-iso-main-$attempt.png"
            Save-ScreenBitmap -Path $shotIso -X $open2.Client.X -Y $open2.Client.Y -Width $open2.Client.Width -Height $open2.Client.Height | Out-Null
            $startBtn2 = Find-RedButton -ScreenshotPath $shotIso -MinY ([int]($open2.Client.Height - 220))
            if (-not $startBtn2) { continue }
            Send-Click -X ($open2.Client.X + $startBtn2.X) -Y ($open2.Client.Y + $startBtn2.Y) -TolerancePx 6
            Start-Sleep -Seconds 2
            $stopEl2 = Wait-UiButtonByName -ProcessId $appProcess.Id -Name '全部停止' -TimeoutMs 8000
            if ($stopEl2) { $started2 = $true }
          }
          Add-Result -Name 'd11. 隔离场景录制已启动（UIA 可见「全部停止」）' -Pass $started2 -Detail "started=$started2"
          if ($started2) {
            Start-Sleep -Seconds 7
            $stopInvoked = Invoke-UiButtonByName -ProcessId $appProcess.Id -Name '全部停止'
            if (-not $stopInvoked) {
              [void](Set-UiScrollTop -ProcessId $appProcess.Id)
              Start-Sleep -Milliseconds 250
              $shotIso3 = Join-Path $work 'record-iso-stop.png'
              Save-ScreenBitmap -Path $shotIso3 -X $open2.Client.X -Y $open2.Client.Y -Width $open2.Client.Width -Height $open2.Client.Height | Out-Null
              $stopBtn2 = Find-RedButton -ScreenshotPath $shotIso3 -MaxY ([int]($open2.Client.Height - 250))
              if ($stopBtn2) { Send-Click -X ($open2.Client.X + $stopBtn2.X) -Y ($open2.Client.Y + $stopBtn2.Y) -TolerancePx 6 }
            }
            $final2 = Wait-NewMediaFile -Dir $outDir -Before $beforeMedia2 -TimeoutSec 45
          if ($tone880Process -and -not $tone880Process.HasExited) { $tone880Process.Kill() }
          Add-Result -Name 'd12. 隔离场景停止后产出成品' -Pass ($null -ne $final2) -Detail "final=$final2"
          if ($final2) {
            $copy2 = Join-Path $EvidenceDir 'region-record-isolated.mkv'
            Copy-Item $final2 $copy2 -Force
            $aCount2 = @(& $ffprobe -v error -select_streams a -show_entries stream=index -of csv=p=0 $final2 | Where-Object { $_ -ne '' }).Count
            $vw2 = [int](& $ffprobe -v error -select_streams v:0 -show_entries stream=width -of csv=p=0 $final2)
            $vh2 = [int](& $ffprobe -v error -select_streams v:0 -show_entries stream=height -of csv=p=0 $final2)
            Add-Result -Name 'd13. 隔离成品恰 1 条应用音轨（无系统混音）' -Pass ($aCount2 -eq 1) -Detail "audioTracks=$aCount2"
            $sizeOk2 = ($null -ne $confirmed2 -and $vw2 -eq $confirmed2.Width -and $vh2 -eq $confirmed2.Height)
            Add-Result -Name 'd14. 隔离成品尺寸 = 确认矩形（精确）' -Pass $sizeOk2 `
              -Detail "${vw2}x${vh2}（确认 $($confirmed2.Width)x$($confirmed2.Height)）"
            $g660 = Get-GoertzelDb -Ffmpeg $ffmpeg -File $final2 -Hz 660
            $g880 = Get-GoertzelDb -Ffmpeg $ffmpeg -File $final2 -Hz 880
            $isoOk = ($g660 -gt -60) -and (($g660 - $g880) -ge 15)
            Add-Result -Name 'd15. 隔离音轨 FFT 单频点：660 存在、未选 880 至少低 15dB' -Pass $isoOk `
              -Detail "660Hz=$([math]::Round($g660,1))dB 880Hz=$([math]::Round($g880,1))dB diff=$([math]::Round($g660-$g880,1))dB（Goertzel DFT bin）" -Evidence $copy2
          }
        }
      }
    }
  }
  }

  # ── 汇总 ────────────────────────────────────────────────────────────────
  $failed = @($results | Where-Object { $_.Result -eq 'FAIL' })
  $lines = @(
    '# 区域截图 E2E 结果（自动生成）',
    '',
    "- 时间：$((Get-Date).ToString('yyyy-MM-dd HH:mm:ss'))",
    "- 应用：``$AppExe``",
    "- 输入通道：$(Get-InputMode)",
    "- 虚拟桌面：$($vs.Width)x$($vs.Height) @ ($($vs.X),$($vs.Y))",
    "- 拖拽矩形（物理像素）：$dragX,$dragY $($dragW)x$($dragH)",
    '',
    '| 场景 | 结果 | 明细 | 证据 |',
    '|---|---|---|---|'
  )
  foreach ($r in $results) {
    $lines += "| $($r.Scenario) | $($r.Result) | $($r.Detail -replace '\|', '/') | $($r.Evidence) |"
  }
  $lines += ''
  $lines += "结论：$(if ($failed.Count -eq 0) { '全部 PASS' } else { "$($failed.Count) 项 FAIL" })"
  Set-Content -Path $summaryPath -Value ($lines -join "`r`n") -Encoding UTF8
  Write-Host "汇总：$summaryPath"
  if ($failed.Count -gt 0) { exit 1 }
}
finally {
  Restore-Settings
  if ($toneProcess -and -not $toneProcess.HasExited) { $toneProcess.Kill() }
  if ($tone660Process -and -not $tone660Process.HasExited) { $tone660Process.Kill() }
  if ($tone880Process -and -not $tone880Process.HasExited) { $tone880Process.Kill() }
  if ($targetProcess) { Stop-TargetWindow -Process $targetProcess }
  if (-not $KeepAppRunning) {
    Get-Process screen-rec -ErrorAction SilentlyContinue | Stop-Process -Force
  }
}
