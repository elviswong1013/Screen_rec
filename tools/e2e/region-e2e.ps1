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
  [ValidateSet('all', 'draw', 'esc', 'failure')] [string] $Scenario = 'all',
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

$script:settingsBackup = Join-Path $work 'settings.backup.json'
$targetProcess = $null
$appProcess = $null
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
  if ($targetProcess) { Stop-TargetWindow -Process $targetProcess }
  if (-not $KeepAppRunning) {
    Get-Process screen-rec -ErrorAction SilentlyContinue | Stop-Process -Force
  }
}
