# ============================================================================
# tools/e2e/New-SolidTarget.ps1 — E2E 底层目标窗口（已知纯色 + 色块图案）
#
# 用途：区域截图 E2E 需要一个内容已知、静止、不被覆盖层之外的窗口遮挡的底层目标，
#       用于「成品像素 == 底层像素」比对。窗口 TopMost，用户关掉即可（或删除 StopFile）。
#
# 用法：
#   powershell -NoProfile -File tools/e2e/New-SolidTarget.ps1 `
#     -X 600 -Y 300 -Width 900 -Height 600 -StopFile "$env:TEMP\screen-rec-e2e\target.stop"
# ============================================================================
param(
  [int] $X = 600,
  [int] $Y = 300,
  [int] $Width = 900,
  [int] $Height = 600,
  [string] $StopFile = '',
  [string] $Color = '#00FF00'
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing

if (-not ('SrE2E.Native' -as [type])) {
  Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace SrE2E {
  public static class Native {
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] public static extern int GetAwarenessFromDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] public static extern IntPtr GetThreadDpiAwarenessContext();
  }
}
'@
}
[void][SrE2E.Native]::SetProcessDpiAwarenessContext([IntPtr](-4))

$form = New-Object System.Windows.Forms.Form
$form.FormBorderStyle = [System.Windows.Forms.FormBorderStyle]::None
$form.StartPosition = [System.Windows.Forms.FormStartPosition]::Manual
$form.TopMost = $true
$form.ShowInTaskbar = $false
$form.BackColor = [System.Drawing.ColorTranslator]::FromHtml($Color)
$form.Bounds = New-Object System.Drawing.Rectangle($X, $Y, $Width, $Height)

# 图案：四角 + 中心色块（便于人工辨认，同时不破坏整块纯色背景判据）
$marks = @(
  @{ X = 40; Y = 40; W = 160; H = 120; C = '#FF0000' },
  @{ X = ($Width - 200); Y = 40; W = 160; H = 120; C = '#0000FF' },
  @{ X = 40; Y = ($Height - 160); W = 160; H = 120; C = '#000000' },
  @{ X = ($Width - 200); Y = ($Height - 160); W = 160; H = 120; C = '#FFFFFF' },
  @{ X = [int](($Width - 120) / 2); Y = [int](($Height - 120) / 2); W = 120; H = 120; C = '#FFFF00' }
)
foreach ($m in $marks) {
  $p = New-Object System.Windows.Forms.Panel
  $p.Bounds = New-Object System.Drawing.Rectangle($m.X, $m.Y, $m.W, $m.H)
  $p.BackColor = [System.Drawing.ColorTranslator]::FromHtml($m.C)
  $form.Controls.Add($p)
}

$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = 200
$timer.Add_Tick({
    if ($script:StopFile -and (Test-Path -LiteralPath $script:StopFile)) {
      $timer.Stop()
      $form.Close()
    }
  })
$script:StopFile = $StopFile
$timer.Start()

[void]$form.Show()
# 记录实际物理矩形（供 E2E 断言）
if ($StopFile) {
  $rectFile = "$StopFile.rect"
  "$($form.Left),$($form.Top),$($form.Width),$($form.Height)" | Out-File -LiteralPath $rectFile -Encoding ascii
}
[System.Windows.Forms.Application]::Run($form)
