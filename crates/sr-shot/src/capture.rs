//! 捕获基础设施（内部）：WGC 单帧抓取（显示器/窗口）、窗口 PrintWindow 备选、
//! 物理显示器枚举、worker 线程环境（WinRT apartment + per-monitor-v2 DPI 上下文）。
//!
//! 物理像素语义（REQ-6）：所有 Win32 坐标查询都发生在 `CaptureThread` 持有的
//! per-monitor-v2 上下文内；显示器物理矩形以 DEVMODE（EnumDisplaySettingsW，
//! 与 sr-enum 同源、不受 DPI 感知影响）为准。

use std::mem::size_of;
use std::time::{Duration, Instant};

use windows::core::{factory, Interface, PCWSTR};
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, EnumDisplayMonitors,
    EnumDisplaySettingsW, GetDIBits, GetMonitorInfoW, GetWindowDC, ReleaseDC, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DEVMODEW, DIB_RGB_COLORS, ENUM_CURRENT_SETTINGS, HDC,
    HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::System::Com::{CoDecrementMTAUsage, CoIncrementMTAUsage, CO_MTA_USAGE_COOKIE};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, IsIconic, IsWindow, PW_RENDERFULLCONTENT,
};

use crate::screenshot::ShotError;

/// 取帧超时（首帧依赖 DWM 合成节拍，4s 已远超正常延迟；超时视为捕获不可用）。
const FRAME_TIMEOUT: Duration = Duration::from_secs(4);
/// 取帧轮询间隔（WGC 无同步等待原语；TryGetNextFrame 在无帧时返回 Err）。
const FRAME_POLL: Duration = Duration::from_millis(8);

// ─────────────────────────── worker 线程环境 ───────────────────────────

/// capture worker 线程环境：WinRT apartment（RoInitialize MTA + CoIncrementMTAUsage）+
/// per-monitor-v2 DPI 上下文。Drop 时按 RoUninitialize → CoDecrementMTAUsage 收尾并
/// 恢复 DPI 上下文。
///
/// 初始化协议与 windows-capture（MIT，长期实测的 WGC 库）一致：必须走 `RoInitialize`
/// 注册 WinRT apartment——裸 `CoInitializeEx` 绕过 WinRT 的延迟卸载记账，捕获对象全部
/// 释放后 `GraphicsCapture.dll` 过早卸载、DWM/线程池遗留回调命中已卸载模块
/// （实测崩溃签名： faulting module `GraphicsCapture.dll_unloaded`，AV 0xc0000005）。
pub(crate) struct CaptureThread {
    mta_cookie: Option<CO_MTA_USAGE_COOKIE>,
    prev_dpi_ctx: Option<DPI_AWARENESS_CONTEXT>,
}

impl CaptureThread {
    pub(crate) fn init() -> Self {
        unsafe {
            // CoIncrementMTAUsage：确保进程存在 MTA（现代替代 CoInitializeEx(MTA)，
            // 无需配对 CoUninitialize、可安全配对 CoDecrementMTAUsage）。
            let mta_cookie = CoIncrementMTAUsage().ok();
            // RoInitialize：注册 WinRT apartment（S_FALSE=已初始化，容忍）。
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
            // per-monitor v2：本线程内的 Win32 坐标查询（EnumDisplayMonitors/GetWindowRect
            // 等）返回物理像素，避免 DPI 虚拟化（REQ-6）。仅影响本线程，不改进程清单——
            // 调用方（含测试进程的 DPI-unaware 约定）不受影响。
            let prev = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            CaptureThread {
                mta_cookie,
                prev_dpi_ctx: (!prev.is_invalid()).then_some(prev),
            }
        }
    }
}

impl Drop for CaptureThread {
    fn drop(&mut self) {
        unsafe {
            // 先结束 WinRT apartment（内部按安全序处理 WinRT DLL 的延迟卸载），
            // 再撤销 MTA 引用，最后恢复 DPI 上下文。
            RoUninitialize();
            if let Some(cookie) = self.mta_cookie {
                let _ = CoDecrementMTAUsage(cookie);
            }
        }
        if let Some(ctx) = self.prev_dpi_ctx {
            unsafe { SetThreadDpiAwarenessContext(ctx) };
        }
    }
}

// ─────────────────────────── 帧与显示器 ───────────────────────────

/// 紧凑 BGRA 帧（行无对齐 padding，长度 = w*h*4）。
pub(crate) struct BgraFrame {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) data: Vec<u8>,
}

impl BgraFrame {
    /// BGRA → RGBA（PNG 编码输入；不透明 Alpha 置 255）。
    pub(crate) fn to_rgba(&self) -> Vec<u8> {
        let mut out = self.data.clone();
        for px in out.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 0xff;
        }
        out
    }
}

/// 物理显示器条目：DEVMODE 物理矩形 + HMONITOR（WGC 目标）。
/// 枚举序 = EnumDisplayMonitors 回调序（与 sr_enum::monitors 同法，index 空间一致）。
pub(crate) struct PhysicalMonitor {
    pub(crate) hmonitor: HMONITOR,
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width_px: u32,
    pub(crate) height_px: u32,
}

/// 枚举当前活动显示器（物理矩形；须在 per-monitor-v2 上下文内调用）。
pub(crate) fn list_physical_monitors() -> Vec<PhysicalMonitor> {
    let mut out: Vec<PhysicalMonitor> = Vec::new();
    unsafe {
        // 同线程同步回调；EnumDisplayMonitors 失败时返回已收集条目（为空由调用方处置）。
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(monitor_proc),
            LPARAM(&mut out as *mut Vec<PhysicalMonitor> as isize),
        );
    }
    out
}

/// 回调契约：extern "system" 内禁止 unwinding——collect_monitor 全程无 panic 路径。
unsafe extern "system" fn monitor_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> windows::core::BOOL {
    let out = &mut *(lparam.0 as *mut Vec<PhysicalMonitor>);
    collect_monitor(hmonitor, out);
    windows::core::BOOL::from(true)
}

fn collect_monitor(hmonitor: HMONITOR, out: &mut Vec<PhysicalMonitor>) {
    unsafe {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if !GetMonitorInfoW(
            hmonitor,
            &mut info.monitorInfo as *mut _ as *mut MONITORINFO,
        )
        .as_bool()
        {
            return;
        }
        // 物理矩形：DEVMODE（与 sr-enum 同源，不受 DPI 感知影响）；失败兜底 rcMonitor
        //（本线程为 per-monitor-v2，rcMonitor 亦为物理坐标）。
        let device = windows::core::HSTRING::from(wide_to_string(&info.szDevice));
        let mut dm = DEVMODEW::default();
        dm.dmSize = size_of::<DEVMODEW>() as u16;
        let (x, y, width_px, height_px) = if EnumDisplaySettingsW(
            PCWSTR::from_raw(device.as_ptr()),
            ENUM_CURRENT_SETTINGS,
            &mut dm,
        )
        .as_bool()
        {
            let pos = dm.Anonymous1.Anonymous2.dmPosition;
            (pos.x, pos.y, dm.dmPelsWidth, dm.dmPelsHeight)
        } else {
            let r = &info.monitorInfo.rcMonitor;
            (
                r.left,
                r.top,
                (r.right - r.left) as u32,
                (r.bottom - r.top) as u32,
            )
        };
        out.push(PhysicalMonitor {
            hmonitor,
            x,
            y,
            width_px,
            height_px,
        });
    }
}

fn wide_to_string(wide: &[u16]) -> String {
    let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end])
}

// ─────────────────────────── 目标捕获入口 ───────────────────────────

/// 捕获一台显示器的完整物理帧（WGC 单帧；尺寸必须与 DEVMODE 一致，宁失败不出错图）。
pub(crate) fn capture_monitor(mon: &PhysicalMonitor) -> Result<BgraFrame, ShotError> {
    let item = monitor_capture_item(mon)?;
    let frame = wgc_grab_item(&item, &format!("显示器 ({},{})", mon.x, mon.y))?;
    if (frame.width, frame.height) != (mon.width_px, mon.height_px) {
        return Err(ShotError::CaptureUnavailable {
            detail: format!(
                "显示器 ({},{}) WGC 帧 {}x{} ≠ DEVMODE 物理分辨率 {}x{}（物理像素契约破坏，拒绝输出）",
                mon.x, mon.y, frame.width, frame.height, mon.width_px, mon.height_px
            ),
        });
    }
    Ok(frame)
}

/// 捕获窗口自身表面（REQ-7：被遮挡仍正确）。
/// 前置校验：无效窗口 → TargetInvalid；最小化（IsIconic）→ TargetMinimized。
/// WGC 主路径失败时回退 PrintWindow(PW_RENDERFULLCONTENT)（研究报告 §4 备选）。
pub(crate) fn capture_window(hwnd_u64: u64) -> Result<BgraFrame, ShotError> {
    let hwnd = HWND(hwnd_u64 as usize as *mut core::ffi::c_void);
    if hwnd_u64 == 0 || !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return Err(ShotError::TargetInvalid {
            detail: format!(
                "hwnd={hwnd_u64:#x} 不是有效窗口（IsWindow=false，E_TARGET_LOST 语义）"
            ),
        });
    }
    if unsafe { IsIconic(hwnd) }.as_bool() {
        return Err(ShotError::TargetMinimized);
    }
    match wgc_capture_window(hwnd) {
        Ok(frame) => Ok(frame),
        Err(wgc_err) => match print_window_capture(hwnd) {
            Ok(frame) => Ok(frame),
            Err(pw_err) => Err(ShotError::CaptureUnavailable {
                detail: format!(
                    "WGC 窗口捕获失败（{wgc_err}）且 PrintWindow 备选亦失败（{pw_err}）"
                ),
            }),
        },
    }
}

fn wgc_capture_window(hwnd: HWND) -> Result<BgraFrame, ShotError> {
    let interop: IGraphicsCaptureItemInterop =
        factory::<GraphicsCaptureItem, _>().map_err(|e| ShotError::CaptureUnavailable {
            detail: format!("激活 GraphicsCaptureItem 工厂: {e}"),
        })?;
    let item: GraphicsCaptureItem =
        unsafe { interop.CreateForWindow(hwnd) }.map_err(|e| ShotError::CaptureUnavailable {
            detail: format!("CreateForWindow: {e}"),
        })?;
    wgc_grab_item(&item, "窗口")
}

fn monitor_capture_item(mon: &PhysicalMonitor) -> Result<GraphicsCaptureItem, ShotError> {
    let interop: IGraphicsCaptureItemInterop =
        factory::<GraphicsCaptureItem, _>().map_err(|e| ShotError::CaptureUnavailable {
            detail: format!("激活 GraphicsCaptureItem 工厂: {e}"),
        })?;
    let item: GraphicsCaptureItem =
        unsafe { interop.CreateForMonitor(mon.hmonitor) }.map_err(|e| {
            ShotError::CaptureUnavailable {
                detail: format!("CreateForMonitor(monitor at {},{}): {e}", mon.x, mon.y),
            }
        })?;
    Ok(item)
}

// ─────────────────────────── WGC 单帧抓取 ───────────────────────────

/// WGC 单帧抓取：free-threaded frame pool + 轮询 TryGetNextFrame（无事件泵依赖）。
/// 输出为帧实际内容尺寸的紧凑 BGRA。
///
/// D3D11 释放序：`ID3D11DeviceContext` 与纹理等子对象引用必须先于 `ID3D11Device`
/// 释放（MSDN 约定；部分驱动在 device 先释放时于 context 释放路径上访问越界）。
/// 本函数把 device 声明在最前（Rust 逆序 drop ⇒ 最后释放），其余 D3D 引用随后声明。
fn wgc_grab_item(item: &GraphicsCaptureItem, what: &str) -> Result<BgraFrame, ShotError> {
    let d3d = create_d3d_device()?;
    let ctx = unsafe { d3d.GetImmediateContext() }.map_err(|e| ShotError::CaptureUnavailable {
        detail: format!("{what}: GetImmediateContext: {e}"),
    })?;
    let item_size = item.Size().map_err(|e| ShotError::CaptureUnavailable {
        detail: format!("{what}: 读取捕获项尺寸失败: {e}"),
    })?;
    if item_size.Width <= 0 || item_size.Height <= 0 {
        return Err(ShotError::CaptureUnavailable {
            detail: format!(
                "{what}: 捕获项尺寸无效 {}x{}",
                item_size.Width, item_size.Height
            ),
        });
    }

    let dxgi: IDXGIDevice = d3d.cast().map_err(|e| ShotError::CaptureUnavailable {
        detail: format!("{what}: QI IDXGIDevice 失败: {e}"),
    })?;
    let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }.map_err(|e| {
        ShotError::CaptureUnavailable {
            detail: format!("{what}: CreateDirect3D11DeviceFromDXGIDevice: {e}"),
        }
    })?;
    let rt_device: IDirect3DDevice =
        inspectable
            .cast()
            .map_err(|e| ShotError::CaptureUnavailable {
                detail: format!("{what}: 转换 WinRT IDirect3DDevice 失败: {e}"),
            })?;

    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &rt_device,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        2,
        item_size,
    )
    .map_err(|e| ShotError::CaptureUnavailable {
        detail: format!("{what}: CreateFreeThreaded: {e}"),
    })?;
    let session = pool
        .CreateCaptureSession(item)
        .map_err(|e| ShotError::CaptureUnavailable {
            detail: format!("{what}: CreateCaptureSession: {e}"),
        })?;
    // 光标不入帧（可选能力；失败不阻塞，REQ-11 光标开关属视频路径契约）
    let _ = session.SetIsCursorCaptureEnabled(false);
    session
        .StartCapture()
        .map_err(|e| ShotError::CaptureUnavailable {
            detail: format!("{what}: StartCapture: {e}"),
        })?;

    let deadline = Instant::now() + FRAME_TIMEOUT;
    let out = loop {
        match pool.TryGetNextFrame() {
            Ok(f) => {
                let content = f
                    .ContentSize()
                    .map(|s| (s.Width, s.Height))
                    .unwrap_or((0, 0));
                if content.0 > 0 && content.1 > 0 {
                    // 帧读取在 pool/session 仍存活的内层作用域完成；frame→surface→
                    // texture 全部在 Close 之前释放（windows-capture 同序）。
                    let out = (|| -> Result<BgraFrame, ShotError> {
                        let surface = f.Surface().map_err(|e| ShotError::CaptureUnavailable {
                            detail: format!("{what}: 读取帧 surface 失败: {e}"),
                        })?;
                        let access: IDirect3DDxgiInterfaceAccess =
                            surface.cast().map_err(|e| ShotError::CaptureUnavailable {
                                detail: format!("{what}: QI IDirect3DDxgiInterfaceAccess: {e}"),
                            })?;
                        let texture: ID3D11Texture2D =
                            unsafe { access.GetInterface() }.map_err(|e| {
                                ShotError::CaptureUnavailable {
                                    detail: format!("{what}: 获取 D3D11 纹理失败: {e}"),
                                }
                            })?;
                        read_texture_to_bgra(&d3d, &ctx, &texture, what)
                    })();
                    break out;
                }
                if Instant::now() >= deadline {
                    let _ = pool.Close();
                    let _ = session.Close();
                    return Err(ShotError::CaptureTimeout {
                        detail: format!("{what}: {FRAME_TIMEOUT:?} 内未取得非空 WGC 帧"),
                    });
                }
                std::thread::sleep(FRAME_POLL);
            }
            Err(_) if Instant::now() < deadline => std::thread::sleep(FRAME_POLL),
            Err(e) => {
                let _ = pool.Close();
                let _ = session.Close();
                return Err(ShotError::CaptureTimeout {
                    detail: format!("{what}: {FRAME_TIMEOUT:?} 内未取得 WGC 帧（{e}）"),
                });
            }
        }
    };
    // 收尾序（windows-capture cleanup 同序）：frame pool 先 Close，session 后 Close；
    // item 及设备引用在作用域收尾逆序 drop（device 最后）。
    let _ = pool.Close();
    let _ = session.Close();
    out
}

fn create_d3d_device() -> Result<ID3D11Device, ShotError> {
    unsafe {
        let mut device = None;
        let mut context = None;
        D3D11CreateDevice(
            None, // 默认适配器
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None, // 特性级别缺省
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .map_err(|e| ShotError::CaptureUnavailable {
            detail: format!("D3D11CreateDevice: {e}"),
        })?;
        drop(context); // immediate context 由调用方按序经 GetImmediateContext 另行获取
        device.ok_or_else(|| ShotError::CaptureUnavailable {
            detail: "D3D11CreateDevice 返回空设备".into(),
        })
    }
}

/// 暂存纹理 + CPU 回读：GPU 纹理 → 紧凑 BGRA 行。
fn read_texture_to_bgra(
    device: &ID3D11Device,
    ctx: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
    what: &str,
) -> Result<BgraFrame, ShotError> {
    unsafe {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        texture.GetDesc(&mut desc);
        if desc.Width == 0 || desc.Height == 0 {
            return Err(ShotError::CaptureUnavailable {
                detail: format!("{what}: 帧纹理尺寸为 0"),
            });
        }
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: desc.Width,
            Height: desc.Height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut staging = None;
        device
            .CreateTexture2D(&staging_desc, None, Some(&mut staging))
            .map_err(|e| ShotError::CaptureUnavailable {
                detail: format!("{what}: 创建暂存纹理失败: {e}"),
            })?;
        let staging = staging.ok_or_else(|| ShotError::CaptureUnavailable {
            detail: format!("{what}: 暂存纹理创建返回空"),
        })?;
        ctx.CopyResource(&staging, texture);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .map_err(|e| ShotError::CaptureUnavailable {
                detail: format!("{what}: Map 暂存纹理失败: {e}"),
            })?;
        let mut data = vec![0u8; desc.Width as usize * desc.Height as usize * 4];
        let row_len = desc.Width as usize * 4;
        for row in 0..desc.Height as usize {
            let src = (mapped.pData as *const u8).add(row * mapped.RowPitch as usize);
            std::ptr::copy_nonoverlapping(src, data.as_mut_ptr().add(row * row_len), row_len);
        }
        ctx.Unmap(&staging, 0);
        Ok(BgraFrame {
            width: desc.Width,
            height: desc.Height,
            data,
        })
    }
}

// ─────────────────────────── 窗口备选：PrintWindow ───────────────────────────

/// PrintWindow(PW_RENDERFULLCONTENT) 捕获窗口自身表面（含被遮挡；红报告 §6.2 已独立
/// 验证被遮挡目标的 DWM 表面保有目标内容）。须在 per-monitor-v2 上下文内调用
/// （GetWindowRect 返回物理像素）。
fn print_window_capture(hwnd: HWND) -> Result<BgraFrame, ShotError> {
    unsafe {
        let mut rc = RECT::default();
        GetWindowRect(hwnd, &mut rc).map_err(|e| ShotError::TargetInvalid {
            detail: format!("GetWindowRect: {e}"),
        })?;
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        if w <= 0 || h <= 0 {
            return Err(ShotError::TargetInvalid {
                detail: format!("窗口矩形为空 ({w}x{h})"),
            });
        }
        let hdc_window = GetWindowDC(Some(hwnd));
        if hdc_window.is_invalid() {
            return Err(ShotError::CaptureUnavailable {
                detail: "GetWindowDC 失败".into(),
            });
        }
        let hdc_mem = CreateCompatibleDC(Some(hdc_window));
        let hbmp = CreateCompatibleBitmap(hdc_window, w, h);
        if hdc_mem.is_invalid() || hbmp.is_invalid() {
            if !hbmp.is_invalid() {
                let _ = DeleteObject(hbmp.into());
            }
            if !hdc_mem.is_invalid() {
                let _ = DeleteDC(hdc_mem);
            }
            let _ = ReleaseDC(Some(hwnd), hdc_window);
            return Err(ShotError::CaptureUnavailable {
                detail: "创建兼容 DC/位图失败".into(),
            });
        }
        let old = SelectObject(hdc_mem, hbmp.into());
        let drawn = PrintWindow(hwnd, hdc_mem, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT));
        // GetDIBits 要求位图不在任何 DC 中被选中
        SelectObject(hdc_mem, old);
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h; // 负值 = 顶行在前（与帧缓冲行序一致）
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB.0;
        let mut data = vec![0u8; w as usize * h as usize * 4];
        let lines = GetDIBits(
            hdc_mem,
            hbmp,
            0,
            h as u32,
            Some(data.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        let _ = DeleteObject(hbmp.into());
        let _ = DeleteDC(hdc_mem);
        let _ = ReleaseDC(Some(hwnd), hdc_window);

        if !drawn.as_bool() || lines == 0 {
            return Err(ShotError::CaptureUnavailable {
                detail: format!(
                    "PrintWindow(PW_RENDERFULLCONTENT) 失败（drawn={}，GetDIBits lines={lines}）",
                    drawn.as_bool()
                ),
            });
        }
        Ok(BgraFrame {
            width: w as u32,
            height: h as u32,
            data,
        })
    }
}
