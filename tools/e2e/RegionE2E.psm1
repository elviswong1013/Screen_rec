# ============================================================================
# tools/e2e/RegionE2E.psm1 — 区域截图端到端测试基座（PowerShell + Win32 SendInput）
#
# 用途：驱动真实 screen-rec.exe（优先 target/release，其次 target/debug）：
#   - 定位主窗口并按真实鼠标点击「截图 → 框选区域截图」；
#   - 用 SendInput 做拖拽/Esc（真实输入，非 webview 内部脚本调用）；
#   - 桌面 Bitmap 截图 + 像素统计（覆盖层是否绘制、成品是否被遮罩/边框污染）。
#
# 约束：进程必须显式声明 per-monitor-v2 DPI 感知，否则 GetWindowRect /
# SetCursorPos / CopyFromScreen 在 150% 缩放下会被 DPI 虚拟化（坐标整体偏移）。
# ============================================================================
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not ('SrE2E.Native' -as [type])) {
  Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

namespace SrE2E {
  [StructLayout(LayoutKind.Sequential)]
  public struct RECT { public int Left, Top, Right, Bottom; }

  [StructLayout(LayoutKind.Sequential)]
  public struct POINT { public int X, Y; }

  [StructLayout(LayoutKind.Sequential)]
  public struct MOUSEINPUT {
    public int dx, dy;
    public uint mouseData, dwFlags, time;
    public IntPtr dwExtraInfo;
  }

  [StructLayout(LayoutKind.Sequential)]
  public struct KEYBDINPUT {
    public ushort wVk, wScan;
    public uint dwFlags, time;
    public IntPtr dwExtraInfo;
  }

  [StructLayout(LayoutKind.Explicit)]
  public struct INPUT {
    [FieldOffset(0)] public uint type;
    [FieldOffset(8)] public MOUSEINPUT mi;
    [FieldOffset(8)] public KEYBDINPUT ki;
  }

  public delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);

  public static class Native {
    public const uint INPUT_MOUSE = 0;
    public const uint INPUT_KEYBOARD = 1;
    public const uint MOUSEEVENTF_MOVE = 0x0001;
    public const uint MOUSEEVENTF_LEFTDOWN = 0x0002;
    public const uint MOUSEEVENTF_LEFTUP = 0x0004;
    public const uint MOUSEEVENTF_ABSOLUTE = 0x8000;
    public const uint MOUSEEVENTF_VIRTUALDESK = 0x4000;
    public const uint KEYEVENTF_KEYUP = 0x0002;

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx);

    [DllImport("user32.dll")] public static extern IntPtr GetThreadDpiAwarenessContext();
    [DllImport("user32.dll")] public static extern int GetAwarenessFromDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);

    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hwnd, ref POINT pt);

    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr hwnd, EnumProc cb, IntPtr lparam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd, StringBuilder sb, int max);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hwnd, StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr hwnd, int index);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);

    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT pt);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, IntPtr extra);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, IntPtr extra);
    [DllImport("user32.dll")] public static extern uint SendInput(uint count, INPUT[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint attach, uint attachTo, bool fAttach);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int cmd);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);

    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
  }
}
'@
}

function Get-CursorPosition {
  $pt = New-Object SrE2E.POINT
  [void][SrE2E.Native]::GetCursorPos([ref]$pt)
  return [pscustomobject]@{ X = $pt.X; Y = $pt.Y }
}

function Test-SendInputInjection {
  <#
    .SYNOPSIS 探测本会话是否接受 SendInput 注入（某些会话/钩子会静默丢弃）。
    .DESCRIPTION SendInput 返回 1 也可能完全没有效果；必须用光标位移验证。
  #>
  $before = Get-CursorPosition
  $inputs = New-Object 'SrE2E.INPUT[]' 1
  $inputs[0].type = [SrE2E.Native]::INPUT_MOUSE
  $inputs[0].mi.dx = 6
  $inputs[0].mi.dy = 0
  $inputs[0].mi.dwFlags = [SrE2E.Native]::MOUSEEVENTF_MOVE
  $sent = [SrE2E.Native]::SendInput(1, $inputs, [System.Runtime.InteropServices.Marshal]::SizeOf([type][SrE2E.INPUT]))
  Start-Sleep -Milliseconds 60
  $after = Get-CursorPosition
  [void][SrE2E.Native]::SetCursorPos($before.X, $before.Y)
  return ($sent -eq 1 -and ($after.X -ne $before.X -or $after.Y -ne $before.Y))
}

$script:Initialized = $false
$script:UseSendInput = $false

function Initialize-E2E {
  <#
    .SYNOPSIS 启用 per-monitor-v2 DPI 感知并探测输入注入通道（SendInput / legacy mouse_event）。
  #>
  if ($script:Initialized) { return }
  $ctx = [IntPtr](-4) # DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
  if (-not [SrE2E.Native]::SetProcessDpiAwarenessContext($ctx)) {
    $awareness = [SrE2E.Native]::GetAwarenessFromDpiAwarenessContext([SrE2E.Native]::GetThreadDpiAwarenessContext())
    if ($awareness -ne 2) { throw "无法启用 per-monitor-v2 DPI 感知（awareness=$awareness）" }
  }
  $script:UseSendInput = Test-SendInputInjection
  $script:Initialized = $true
}

function Get-InputMode {
  Initialize-E2E
  if ($script:UseSendInput) { return 'SendInput' }
  return 'mouse_event(legacy)'
}

function Get-VirtualScreen {
  Initialize-E2E
  # SM_XVIRTUALSCREEN=76, SM_YVIRTUALSCREEN=77, SM_CXVIRTUALSCREEN=78, SM_CYVIRTUALSCREEN=79
  [pscustomobject]@{
    X = [SrE2E.Native]::GetSystemMetrics(76)
    Y = [SrE2E.Native]::GetSystemMetrics(77)
    Width = [SrE2E.Native]::GetSystemMetrics(78)
    Height = [SrE2E.Native]::GetSystemMetrics(79)
  }
}

function Get-TopLevelWindows {
  Initialize-E2E
  $list = New-Object System.Collections.ArrayList
  $cb = [SrE2E.EnumProc] {
    param($hwnd, $lparam)
    $cls = New-Object System.Text.StringBuilder 256
    [void][SrE2E.Native]::GetClassName($hwnd, $cls, 256)
    $title = New-Object System.Text.StringBuilder 256
    [void][SrE2E.Native]::GetWindowText($hwnd, $title, 256)
    $rect = New-Object SrE2E.RECT
    [void][SrE2E.Native]::GetWindowRect($hwnd, [ref]$rect)
    $procId = [uint32]0
    [void][SrE2E.Native]::GetWindowThreadProcessId($hwnd, [ref]$procId)
    [void]$list.Add([pscustomobject]@{
      Handle = $hwnd
      Class = $cls.ToString()
      Title = $title.ToString()
      Visible = [SrE2E.Native]::IsWindowVisible($hwnd)
      Rect = [pscustomobject]@{
        X = $rect.Left; Y = $rect.Top
        Width = $rect.Right - $rect.Left; Height = $rect.Bottom - $rect.Top
      }
      Style = [SrE2E.Native]::GetWindowLong($hwnd, -16)
      ExStyle = [SrE2E.Native]::GetWindowLong($hwnd, -20)
      ProcessId = [int]$procId
    })
    return $true
  }
  [void][SrE2E.Native]::EnumWindows($cb, [IntPtr]::Zero)
  return $list.ToArray()
}

function Get-ChildWindows {
  param([Parameter(Mandatory)] [IntPtr] $Handle)
  Initialize-E2E
  $list = New-Object System.Collections.ArrayList
  $cb = [SrE2E.EnumProc] {
    param($hwnd, $lparam)
    $cls = New-Object System.Text.StringBuilder 256
    [void][SrE2E.Native]::GetClassName($hwnd, $cls, 256)
    $rect = New-Object SrE2E.RECT
    [void][SrE2E.Native]::GetWindowRect($hwnd, [ref]$rect)
    $client = New-Object SrE2E.RECT
    [void][SrE2E.Native]::GetClientRect($hwnd, [ref]$client)
    [void]$list.Add([pscustomobject]@{
      Handle = $hwnd
      Class = $cls.ToString()
      Visible = [SrE2E.Native]::IsWindowVisible($hwnd)
      Width = $rect.Right - $rect.Left
      Height = $rect.Bottom - $rect.Top
      ClientWidth = $client.Right
      ClientHeight = $client.Bottom
    })
    return $true
  }
  [void][SrE2E.Native]::EnumChildWindows($Handle, $cb, [IntPtr]::Zero)
  return $list.ToArray()
}

function Wait-TopLevelWindow {
  param(
    [Parameter(Mandatory)] [string] $Title,
    [int] $TimeoutMs = 8000,
    [int] $ProcessId = 0
  )
  $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
  do {
    $hit = @(Get-TopLevelWindows | Where-Object {
        $_.Title -eq $Title -and ($ProcessId -eq 0 -or $_.ProcessId -eq $ProcessId)
      })
    if ($hit.Count -gt 0) { return $hit[0] }
    Start-Sleep -Milliseconds 120
  } while ([DateTime]::UtcNow -lt $deadline)
  return $null
}

function Wait-NoTopLevelWindow {
  param(
    [Parameter(Mandatory)] [string] $Title,
    [int] $TimeoutMs = 8000
  )
  $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
  do {
    $hit = @(Get-TopLevelWindows | Where-Object { $_.Title -eq $Title })
    if ($hit.Count -eq 0) { return $true }
    Start-Sleep -Milliseconds 120
  } while ([DateTime]::UtcNow -lt $deadline)
  return $false
}

function Move-Window {
  param(
    [Parameter(Mandatory)] [IntPtr] $Handle,
    [Parameter(Mandatory)] [int] $X,
    [Parameter(Mandatory)] [int] $Y,
    [int] $Width = 0,
    [int] $Height = 0
  )
  Initialize-E2E
  $flags = 0x0004 -bor 0x0010 # SWP_NOZORDER | SWP_NOACTIVATE
  if ($Width -le 0 -or $Height -le 0) { $flags = $flags -bor 0x0001 } # SWP_NOSIZE
  [void][SrE2E.Native]::SetWindowPos($Handle, [IntPtr]::Zero, $X, $Y, $Width, $Height, $flags)
  Start-Sleep -Milliseconds 250
}

function Get-ClientArea {
  param([Parameter(Mandatory)] [IntPtr] $Handle)
  Initialize-E2E
  $rect = New-Object SrE2E.RECT
  [void][SrE2E.Native]::GetClientRect($Handle, [ref]$rect)
  $origin = New-Object SrE2E.POINT
  $origin.X = 0; $origin.Y = 0
  [void][SrE2E.Native]::ClientToScreen($Handle, [ref]$origin)
  return [pscustomobject]@{
    X = $origin.X
    Y = $origin.Y
    Width = $rect.Right - $rect.Left
    Height = $rect.Bottom - $rect.Top
  }
}

function Get-MonitorUnion {
  <#
    .SYNOPSIS 显示器物理矩形并集（排除“虚拟屏幕被异常窗口撑大”的干扰）。
  #>
  Initialize-E2E
  Add-Type -AssemblyName System.Windows.Forms
  $minX = [int]::MaxValue; $minY = [int]::MaxValue
  $maxX = [int]::MinValue; $maxY = [int]::MinValue
  foreach ($s in [System.Windows.Forms.Screen]::AllScreens) {
    $b = $s.Bounds
    if ($b.X -lt $minX) { $minX = $b.X }
    if ($b.Y -lt $minY) { $minY = $b.Y }
    if (($b.X + $b.Width) -gt $maxX) { $maxX = $b.X + $b.Width }
    if (($b.Y + $b.Height) -gt $maxY) { $maxY = $b.Y + $b.Height }
  }
  return [pscustomobject]@{ X = $minX; Y = $minY; Width = $maxX - $minX; Height = $maxY - $minY }
}

function Save-ScreenBitmap {
  param(
    [Parameter(Mandatory)] [string] $Path,
    [int] $X = [int]::MinValue,
    [int] $Y = [int]::MinValue,
    [int] $Width = 0,
    [int] $Height = 0
  )
  Initialize-E2E
  Add-Type -AssemblyName System.Drawing
  if ($X -eq [int]::MinValue) {
    $vs = Get-VirtualScreen
    $X = $vs.X; $Y = $vs.Y; $Width = $vs.Width; $Height = $vs.Height
  }
  $bmp = New-Object System.Drawing.Bitmap($Width, $Height)
  try {
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    try {
      $g.CopyFromScreen($X, $Y, 0, 0, (New-Object System.Drawing.Size($Width, $Height)))
    } finally { $g.Dispose() }
    $dir = Split-Path -Parent $Path
    if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
  } finally { $bmp.Dispose() }
  return $Path
}

function Get-ImageStats {
  param(
    [Parameter(Mandatory)] [string] $Path,
    [int] $Step = 9,
    [int] $X = 0, [int] $Y = 0, [int] $Width = 0, [int] $Height = 0
  )
  Add-Type -AssemblyName System.Drawing
  $bmp = [System.Drawing.Bitmap]::FromFile($Path)
  try {
    if ($Width -le 0) { $Width = $bmp.Width - $X }
    if ($Height -le 0) { $Height = $bmp.Height - $Y }
    $sum = 0.0; $sum2 = 0.0; $n = 0; $min = 255.0; $max = 0.0
    for ($yy = $Y; $yy -lt ($Y + $Height); $yy += $Step) {
      for ($xx = $X; $xx -lt ($X + $Width); $xx += $Step) {
        $c = $bmp.GetPixel($xx, $yy)
        $l = 0.299 * $c.R + 0.587 * $c.G + 0.114 * $c.B
        $sum += $l; $sum2 += $l * $l; $n++
        if ($l -lt $min) { $min = $l }
        if ($l -gt $max) { $max = $l }
      }
    }
    $mean = $sum / $n
    [pscustomobject]@{
      Width = $Width; Height = $Height; Samples = $n
      Mean = [math]::Round($mean, 3)
      Variance = [math]::Round($sum2 / $n - $mean * $mean, 3)
      Min = [math]::Round($min, 2); Max = [math]::Round($max, 2)
    }
  } finally { $bmp.Dispose() }
}

function Send-MouseMove {
  param([Parameter(Mandatory)] [int] $X, [Parameter(Mandatory)] [int] $Y)
  Initialize-E2E
  $vs = Get-VirtualScreen
  $nx = [int][math]::Round(($X - $vs.X) * 65535.0 / [math]::Max(1, $vs.Width - 1))
  $ny = [int][math]::Round(($Y - $vs.Y) * 65535.0 / [math]::Max(1, $vs.Height - 1))

  if ($script:UseSendInput) {
    $inputs = New-Object 'SrE2E.INPUT[]' 1
    $inputs[0].type = [SrE2E.Native]::INPUT_MOUSE
    $inputs[0].mi.dx = $nx
    $inputs[0].mi.dy = $ny
    $inputs[0].mi.dwFlags = [SrE2E.Native]::MOUSEEVENTF_MOVE -bor [SrE2E.Native]::MOUSEEVENTF_ABSOLUTE -bor [SrE2E.Native]::MOUSEEVENTF_VIRTUALDESK
    $sent = [SrE2E.Native]::SendInput(1, $inputs, [System.Runtime.InteropServices.Marshal]::SizeOf([type][SrE2E.INPUT]))
    if ($sent -ne 1) { throw "SendInput(move) 失败（sent=$sent）" }
  } else {
    [SrE2E.Native]::mouse_event(0x0001 -bor 0x8000 -bor 0x4000, [uint32]$nx, [uint32]$ny, 0, [IntPtr]::Zero)
  }

  $pos = Get-CursorPosition
  if ([math]::Abs($pos.X - $X) -gt 2 -or [math]::Abs($pos.Y - $Y) -gt 2) {
    [void][SrE2E.Native]::SetCursorPos($X, $Y)
    Start-Sleep -Milliseconds 30
    $pos = Get-CursorPosition
    if ([math]::Abs($pos.X - $X) -gt 2 -or [math]::Abs($pos.Y - $Y) -gt 2) {
      throw "光标定位失败（目标 $X,$Y，实际 $($pos.X),$($pos.Y)）"
    }
  }
}

function Send-MouseButton {
  param([Parameter(Mandatory)] [ValidateSet('down', 'up')] [string] $Action)
  Initialize-E2E
  $flag = if ($Action -eq 'down') { [SrE2E.Native]::MOUSEEVENTF_LEFTDOWN } else { [SrE2E.Native]::MOUSEEVENTF_LEFTUP }
  if ($script:UseSendInput) {
    $inputs = New-Object 'SrE2E.INPUT[]' 1
    $inputs[0].type = [SrE2E.Native]::INPUT_MOUSE
    $inputs[0].mi.dwFlags = $flag
    $sent = [SrE2E.Native]::SendInput(1, $inputs, [System.Runtime.InteropServices.Marshal]::SizeOf([type][SrE2E.INPUT]))
    if ($sent -ne 1) { throw "SendInput($Action) 失败（sent=$sent）" }
  } else {
    [SrE2E.Native]::mouse_event($flag, 0, 0, 0, [IntPtr]::Zero)
  }
}

function Send-Click {
  param(
    [Parameter(Mandatory)] [int] $X,
    [Parameter(Mandatory)] [int] $Y,
    [int] $SettleMs = 120
  )
  Send-MouseMove -X $X -Y $Y
  Start-Sleep -Milliseconds $SettleMs
  Send-MouseButton -Action down
  Start-Sleep -Milliseconds 60
  Send-MouseButton -Action up
}

function Send-Drag {
  param(
    [Parameter(Mandatory)] [int] $X1,
    [Parameter(Mandatory)] [int] $Y1,
    [Parameter(Mandatory)] [int] $X2,
    [Parameter(Mandatory)] [int] $Y2,
    [int] $Steps = 12,
    [int] $StepDelayMs = 30
  )
  Send-MouseMove -X $X1 -Y $Y1
  Start-Sleep -Milliseconds 180
  Send-MouseButton -Action down
  Start-Sleep -Milliseconds 120
  for ($i = 1; $i -le $Steps; $i++) {
    $x = [int][math]::Round($X1 + ($X2 - $X1) * $i / $Steps)
    $y = [int][math]::Round($Y1 + ($Y2 - $Y1) * $i / $Steps)
    Send-MouseMove -X $x -Y $y
    Start-Sleep -Milliseconds $StepDelayMs
  }
  Start-Sleep -Milliseconds 120
  Send-MouseButton -Action up
}

function Send-Key {
  param([Parameter(Mandatory)] [int] $VirtualKey)
  Initialize-E2E
  if ($script:UseSendInput) {
    $down = New-Object 'SrE2E.INPUT[]' 1
    $down[0].type = [SrE2E.Native]::INPUT_KEYBOARD
    $down[0].ki.wVk = [uint16]$VirtualKey
    $up = New-Object 'SrE2E.INPUT[]' 1
    $up[0].type = [SrE2E.Native]::INPUT_KEYBOARD
    $up[0].ki.wVk = [uint16]$VirtualKey
    $up[0].ki.dwFlags = [SrE2E.Native]::KEYEVENTF_KEYUP
    $size = [System.Runtime.InteropServices.Marshal]::SizeOf([type][SrE2E.INPUT])
    if ([SrE2E.Native]::SendInput(1, $down, $size) -ne 1) { throw 'SendInput(key down) 失败' }
    Start-Sleep -Milliseconds 60
    if ([SrE2E.Native]::SendInput(1, $up, $size) -ne 1) { throw 'SendInput(key up) 失败' }
  } else {
    [SrE2E.Native]::keybd_event([byte]$VirtualKey, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 60
    [SrE2E.Native]::keybd_event([byte]$VirtualKey, 0, [SrE2E.Native]::KEYEVENTF_KEYUP, [IntPtr]::Zero)
  }
}

function Set-ForegroundWindow {
  param([Parameter(Mandatory)] [IntPtr] $Handle)
  Initialize-E2E
  [void][SrE2E.Native]::ShowWindow($Handle, 9) # SW_RESTORE
  [void][SrE2E.Native]::BringWindowToTop($Handle)
  [void][SrE2E.Native]::SetForegroundWindow($Handle)
  Start-Sleep -Milliseconds 250
  if ([SrE2E.Native]::GetForegroundWindow() -eq $Handle) { return $true }

  # 前台锁：把本线程输入队列附加到当前前台线程后重试
  $fg = [SrE2E.Native]::GetForegroundWindow()
  $fgThread = [uint32]0
  [void][SrE2E.Native]::GetWindowThreadProcessId($fg, [ref]$fgThread)
  $self = [SrE2E.Native]::GetCurrentThreadId()
  if ($fgThread -ne 0 -and $fgThread -ne $self) {
    [void][SrE2E.Native]::AttachThreadInput($self, $fgThread, $true)
    [void][SrE2E.Native]::SetForegroundWindow($Handle)
    [void][SrE2E.Native]::AttachThreadInput($self, $fgThread, $false)
  }
  Start-Sleep -Milliseconds 250
  if ([SrE2E.Native]::GetForegroundWindow() -eq $Handle) { return $true }

  # 最后手段：点一下标题栏（真实用户首次点击会激活窗口）
  $rect = New-Object SrE2E.RECT
  [void][SrE2E.Native]::GetWindowRect($Handle, [ref]$rect)
  Send-Click -X ($rect.Left + 200) -Y ($rect.Top + 18)
  Start-Sleep -Milliseconds 300
  return ([SrE2E.Native]::GetForegroundWindow() -eq $Handle)
}

function Find-AppWindow {
  param([Parameter(Mandatory)] [int] $ProcessId)
  return Wait-TopLevelWindow -Title 'Screen_rec' -ProcessId $ProcessId -TimeoutMs 15000
}

function Find-AccentButton {
  <#
    .SYNOPSIS 在窗口截图中定位填充色为强调蓝（--accent）的按钮，返回窗口内坐标。
    .DESCRIPTION 只在截图坐标系（窗口左上角为原点）内扫描；逐行取最长连续色块，
                 再取「最宽行」的 x 区间与纵向范围求中心（避免导航高亮等零散像素干扰）。
  #>
  param(
    [Parameter(Mandatory)] [string] $ScreenshotPath,
    [int] $MinRunPx = 120
  )
  Add-Type -AssemblyName System.Drawing
  $bmp = [System.Drawing.Bitmap]::FromFile($ScreenshotPath)
  try {
    $rows = New-Object System.Collections.ArrayList
    for ($y = 0; $y -lt $bmp.Height; $y += 2) {
      $runStart = -1; $runEnd = -1; $bestStart = -1; $bestEnd = -1
      for ($x = 0; $x -lt $bmp.Width; $x += 2) {
        $c = $bmp.GetPixel($x, $y)
        # --accent: #4f7cff 附近（B 高、R 低、G 中）
        $hit = $c.B -ge 200 -and $c.R -le 130 -and $c.G -ge 100 -and $c.G -le 170
        if ($hit) {
          if ($runStart -lt 0) { $runStart = $x }
          $runEnd = $x
        } else {
          if ($runStart -ge 0 -and ($runEnd - $runStart) -gt ($bestEnd - $bestStart)) {
            $bestStart = $runStart; $bestEnd = $runEnd
          }
          $runStart = -1
        }
      }
      if ($runStart -ge 0 -and ($runEnd - $runStart) -gt ($bestEnd - $bestStart)) {
        $bestStart = $runStart; $bestEnd = $runEnd
      }
      if ($bestStart -ge 0 -and ($bestEnd - $bestStart) -ge $MinRunPx) {
        [void]$rows.Add([pscustomobject]@{ Y = $y; Left = $bestStart; Right = $bestEnd })
      }
    }
    if ($rows.Count -eq 0) { return $null }
    $widest = $rows | Sort-Object { $_.Right - $_.Left } -Descending | Select-Object -First 1
    # 纵向范围：与最宽行水平重叠 >50% 的行
    $band = @($rows | Where-Object {
        $overlap = [math]::Min($_.Right, $widest.Right) - [math]::Max($_.Left, $widest.Left)
        $overlap -gt (($widest.Right - $widest.Left) * 0.5)
      })
    $top = ($band | Measure-Object -Property Y -Minimum).Minimum
    $bottom = ($band | Measure-Object -Property Y -Maximum).Maximum
    return [pscustomobject]@{
      X = [int](($widest.Left + $widest.Right) / 2)
      Y = [int](($top + $bottom) / 2)
      Left = $widest.Left; Top = $top; Right = $widest.Right; Bottom = $bottom
    }
  } finally { $bmp.Dispose() }
}

function Send-Wheel {
  param(
    [Parameter(Mandatory)] [int] $Delta,
    [int] $Repeat = 1
  )
  Initialize-E2E
  for ($i = 0; $i -lt $Repeat; $i++) {
    if ($script:UseSendInput) {
      $inputs = New-Object 'SrE2E.INPUT[]' 1
      $inputs[0].type = [SrE2E.Native]::INPUT_MOUSE
      $inputs[0].mi.dwFlags = 0x0800 # MOUSEEVENTF_WHEEL
      $wheelData = [int64]$Delta -band 0xFFFFFFFFL
      $inputs[0].mi.mouseData = [uint32]$wheelData
      $sent = [SrE2E.Native]::SendInput(1, $inputs, [System.Runtime.InteropServices.Marshal]::SizeOf([type][SrE2E.INPUT]))
      if ($sent -ne 1) { throw "SendInput(wheel) 失败（sent=$sent）" }
    } else {
      $wheelData = [int64]$Delta -band 0xFFFFFFFFL
      [SrE2E.Native]::mouse_event(0x0800, 0, 0, [uint32]$wheelData, [IntPtr]::Zero)
    }
    Start-Sleep -Milliseconds 50
  }
}

function Open-RegionOverlayFromUi {
  <#
    .SYNOPSIS 以真实鼠标操作打开区域框选覆盖层（截图视图 → 滚动 → 「框选区域截图」）。
    .DESCRIPTION 坐标基于主窗口左上角（物理像素）。150% DPI 下「截图」导航项在
                 (100, 256)；区域面板在截图视图最下方，需先在 main 空白处滚轮下滚。
                 每一步都做结果校验（导航后面板必须出现强调蓝按钮），失败重试。
    .OUTPUTS 覆盖层窗口信息（Wait-TopLevelWindow 结果）；失败返回 $null。
  #>
  param(
    [Parameter(Mandatory)] [int] $ProcessId,
    [Parameter(Mandatory)] [string] $WorkDir,
    [int] $ScrollNotches = 20,
    [int] $OverlayTimeoutMs = 8000,
    [int] $MaxAttempts = 3
  )
  $win = Find-AppWindow -ProcessId $ProcessId
  if (-not $win) { throw "主窗口未出现（pid=$ProcessId）" }
  $shotDir = Join-Path $env:TEMP 'screen-rec-e2e'
  New-Item -ItemType Directory -Force -Path $shotDir | Out-Null

  $lastShot = $null
  for ($attempt = 1; $attempt -le $MaxAttempts; $attempt++) {
    [void](Set-ForegroundWindow -Handle $win.Handle)
    $client = Get-ClientArea -Handle $win.Handle
    # UI 设计基准宽度 1024 CSS px（tauri.conf.json 主窗口）；缩放 = 物理客户区宽 / 1024。
    # 导航项「截图」在 CSS (45, 139) → 物理 (client.X + 45*scale, client.Y + 139*scale)。
    $uiScale = $client.Width / 1024.0
    Send-Click -X ([int]($client.X + 45 * $uiScale)) -Y ([int]($client.Y + 139 * $uiScale))
    Start-Sleep -Milliseconds 900

    # 主内容区滚动到底部（落点在 main 的 padding 空白，避免落进列表内部滚动区）
    Send-MouseMove -X ($client.X + [int](214 * $uiScale)) -Y ($client.Y + [int](467 * $uiScale))
    Send-Wheel -Delta -120 -Repeat $ScrollNotches
    Start-Sleep -Milliseconds 700

    $shot = Join-Path $shotDir 'main-window-region-panel.png'
    Save-ScreenBitmap -Path $shot -X $client.X -Y $client.Y -Width $client.Width -Height $client.Height | Out-Null
    $lastShot = $shot
    $btn = Find-AccentButton -ScreenshotPath $shot
    if (-not $btn) {
      Send-Wheel -Delta -120 -Repeat 8
      Start-Sleep -Milliseconds 500
      Save-ScreenBitmap -Path $shot -X $client.X -Y $client.Y -Width $client.Width -Height $client.Height | Out-Null
      $btn = Find-AccentButton -ScreenshotPath $shot
      if (-not $btn) { continue }
    }
    Send-Click -X ($client.X + $btn.X) -Y ($client.Y + $btn.Y)
    $overlay = Wait-TopLevelWindow -Title 'region-overlay' -ProcessId $ProcessId -TimeoutMs $OverlayTimeoutMs
    if ($overlay) {
      return [pscustomobject]@{
        AppWindow = $win
        Client = $client
        UiScale = $uiScale
        Button = $btn
        Screenshot = $shot
        Overlay = $overlay
        Attempts = $attempt
      }
    }
  }
  throw "区域面板按钮点击后未出现 region-overlay 窗口（最后截图：$lastShot）"
}

Export-ModuleMember -Function *
