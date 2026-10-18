//! L2 集成测试：截图（REQ-5..7，specs/001-screen-recorder.md §2.2 / §4）。
//!
//! 运行方式（L2 门：`#[ignore]`，默认 `cargo test` 不执行本文件用例）：
//! ```text
//! cargo test -p sr-itests --test shot -- --ignored --nocapture
//! cargo test -p sr-itests --test shot it_screenshot_three_targets -- --ignored --nocapture   # 单用例
//! ```
//! 前置条件：交互式用户会话（Session 0 不合格）+ tools/ffmpeg 锁定构建（遮挡用例做
//! PNG 内容级判定）；缺失 → BLOCKED（fail），不静默 skip（§4）。
//! 本机环境（红测试编写时）：Windows 11 25H2，build 26200.9457，SESSIONNAME=Console；
//! 双显示器混合 DPI：主 3840x2160@(0,0) 150%（DPI 144）+ 副 1440x2560@(3840,442)
//! 125%（DPI 120），水平贴合；无负坐标显示器（见 m2-red-report §10）。
//!
//! 接口契约（测试即契约，详见 docs/milestones/m2-red-report.md §3/§10）：
//! - `sr_shot::screenshot::take_screenshot(ScreenshotRequest) -> Result<SavedScreenshot, ShotError>`
//! - `ScreenshotRequest { target: ShotTarget, output_dir: PathBuf }`
//! - `ShotTarget::Monitor { index: u32 }`（与 `sr_enum::monitors::MonitorEntry.index`
//!   同一索引空间，RouteConfig::Monitor(idx) 兼容）
//!   `ShotTarget::Window { hwnd: u64 }`（HWND 数值化）
//!   `ShotTarget::Region { rect: ScreenRect }`（物理桌面坐标，REQ-6）
//! - `ScreenRect { x: i32, y: i32, width_px: u32, height_px: u32 }`（物理像素；负坐标合法）
//! - `SavedScreenshot { path: PathBuf, width_px: u32, height_px: u32, source: ShotTarget }`
//!   （输出目录内的 PNG；尺寸=物理像素；source=实际捕获来源回执；derive(Debug)）
//! - `ShotError`：公开错误枚举，derive(Debug)。测试仅钉 REQ-7 最小化语义所需的一个
//!   变体 `ShotError::TargetMinimized`（`matches!(.., ShotError::TargetMinimized { .. })`，
//!   容忍变体带字段）；其余变体/形状由实现自定（M1「错误枚举未钉」约定的最小收紧）。
//! ShotTarget/ScreenRect derive(Clone/Debug/PartialEq)。
//!
//! 手法：交叉验证用测试侧 Win32 地面真值（DEVMODE 物理分辨率；WindowFromPoint 遮挡判定）
//! 与 sr_enum（M1 已对 DEVMODE 验证过的显示器索引空间）；内容级判定用锁定 ffmpeg 把
//! PNG 解码为 rgb24 后统计颜色占比（不新增 Rust 依赖）。测试进程保持默认 DPI-unaware
//! （无清单）：若实现输出被 DPI 虚拟化的尺寸，与 DEVMODE 真值比对即失败（REQ-6 意图）。

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Once;
use std::time::{Duration, Instant};

use common::*;
use sr_enum::monitors::list_monitors;
use sr_shot::screenshot::{
    take_screenshot, SavedScreenshot, ScreenRect, ScreenshotRequest, ShotError, ShotTarget,
};

// ─────────────────────────── 内容级判定参数（scratch 实测校准） ───────────────────────────
// 测试窗口内容：目标=红、覆盖=青（通道差 ≥112，容错 30 不会互相混淆）。
// scratch 实测（仓库外，2026-10-16）：被遮挡目标窗口 DWM 内容红占比 0.871、青 0.000
// （PrintWindow(PW_RENDERFULLCONTENT) 464x281）；WGC 窗口帧另含标题栏/边框/圆角，
// 红占比阈值取保守下限 0.30（空白帧≈0；屏幕 Blt 式误实现≈0.9 青）。
const TARGET_RGB: (u8, u8, u8) = (196, 32, 44);
const COVER_RGB: (u8, u8, u8) = (24, 168, 156);
const COLOR_TOL: i32 = 30;
const OCCLUDED_TARGET_RED_MIN: f64 = 0.30;
const OCCLUDED_COVER_LEAK_MAX: f64 = 0.01;

/// REQ-5：截图三目标——显示器 / 窗口 / 矩形区域（程序化矩形，无交互）各出一张 PNG。
///
/// 断言：三次调用各产出一个位于输出目录内的 PNG 文件（路径互不相同）；显示器与区域
/// 的元数据尺寸=物理像素地面真值（显示器=DEVMODE；区域=请求矩形的物理像素宽高）；
/// PNG 实际编码尺寸（IHDR）与元数据一致；source 回执与请求目标一致。
#[test]
#[ignore = "L2：需交互式会话/真实显示器与窗口（specs §4）。显式运行：cargo test -p sr-itests --test shot -- --ignored --nocapture"]
fn it_screenshot_three_targets() {
    require_interactive_session();
    let _os = record_os_build();

    let mons = list_monitors().expect("前置：sr_enum::monitors::list_monitors 应成功（M1 已验证）");
    assert!(!mons.is_empty(), "交叉验证前提：至少一台显示器");
    let primary = mons
        .iter()
        .find(|m| m.is_primary)
        .unwrap_or_else(|| panic!("前置：应有且仅有一台主显示器（得到 {mons:?}）"));
    let out_dir = fresh_out_dir("three");

    // ── 目标 1：显示器（REQ-5；尺寸=DEVMODE 物理分辨率，REQ-6 物理像素契约） ──
    let mon_target = ShotTarget::Monitor {
        index: primary.index,
    };
    let mon_shot = take_screenshot(ScreenshotRequest {
        target: mon_target.clone(),
        output_dir: out_dir.clone(),
    })
    .expect("REQ-5：显示器截图应成功");
    assert_png_shape(&mon_shot, &out_dir, "显示器");
    assert_eq!(
        (mon_shot.width_px, mon_shot.height_px),
        (primary.width_px, primary.height_px),
        "REQ-5/REQ-6：显示器截图尺寸应为主显示器物理分辨率（DEVMODE 真值 {}x{}）",
        primary.width_px,
        primary.height_px
    );
    assert_eq!(
        mon_shot.source, mon_target,
        "REQ-5：source 回执应与请求目标一致（显示器）"
    );

    // ── 目标 2：窗口（自建可见顶层窗口；内容级判定在遮挡用例，此处验形状） ──
    let target_win =
        ColoredWindow::new_target(&format!("SR-M2-RED-shot-target-{}", std::process::id()));
    pump_messages(600);
    target_win.repaint_now();
    pump_messages(200);
    let win_target = ShotTarget::Window {
        hwnd: target_win.hwnd_u64,
    };
    let win_shot = take_screenshot(ScreenshotRequest {
        target: win_target.clone(),
        output_dir: out_dir.clone(),
    })
    .expect("REQ-5：窗口截图应成功");
    assert_png_shape(&win_shot, &out_dir, "窗口");
    assert!(
        win_shot.width_px > 0 && win_shot.height_px > 0,
        "REQ-5：窗口截图尺寸应为正（得到 {}x{}）",
        win_shot.width_px,
        win_shot.height_px
    );
    assert_eq!(
        win_shot.source, win_target,
        "REQ-5：source 回执应与请求目标一致（窗口）"
    );

    // ── 目标 3：矩形区域（程序化矩形，位于主显示器内；不涉及 REQ-8 交互） ──
    let rect = ScreenRect {
        x: primary.x + (primary.width_px / 4) as i32,
        y: primary.y + (primary.height_px / 4) as i32,
        width_px: 320.min(primary.width_px / 2).max(16),
        height_px: 240.min(primary.height_px / 2).max(16),
    };
    let reg_target = ShotTarget::Region { rect: rect.clone() };
    let reg_shot = take_screenshot(ScreenshotRequest {
        target: reg_target.clone(),
        output_dir: out_dir.clone(),
    })
    .expect("REQ-5：区域截图应成功");
    assert_png_shape(&reg_shot, &out_dir, "区域");
    assert_eq!(
        (reg_shot.width_px, reg_shot.height_px),
        (rect.width_px, rect.height_px),
        "REQ-5/REQ-6：区域截图尺寸应等于请求矩形的物理像素宽高（{}x{}）",
        rect.width_px,
        rect.height_px
    );
    assert_eq!(
        reg_shot.source, reg_target,
        "REQ-5：source 回执应与请求目标一致（区域）"
    );

    // ── 三目标各出一张 PNG：三个互不相同的结果路径 ──
    let mut paths = vec![
        mon_shot.path.clone(),
        win_shot.path.clone(),
        reg_shot.path.clone(),
    ];
    let before = paths.len();
    paths.sort();
    paths.dedup();
    assert_eq!(
        paths.len(),
        before,
        "REQ-5：三目标的输出文件应互不相同（{:?}）",
        paths
    );

    drop(target_win);
    let _ = std::fs::remove_dir_all(&out_dir);
}

/// REQ-6（M2 范围：单显示器维度）：输出尺寸=物理像素——结果元数据与 PNG 实际编码尺寸
/// 都必须与直接 API 查询（EnumDisplaySettingsW 的 DEVMODE，不受进程 DPI 感知影响）一致。
///
/// 交叉验证链：sr_shot 元数据 ↔ PNG IHDR ↔ sr_enum 显示器条目（M1 已对 DEVMODE 验证的
/// 索引空间）↔ 测试侧 DEVMODE 直接查询（按 x/y/w/h 匹配同一台显示器）。
/// 测试进程默认 DPI-unaware：实现若返回 DPI 虚拟化尺寸（如 150% 缩放下 2560x1440），
/// 与 DEVMODE 真值（3840x2160）不一致即失败。
/// （跨显示器拼合/负坐标区域属 §4 的 `it_screenshot_mixed_dpi_negative_coords`，
/// 不在本用例范围。）
#[test]
#[ignore = "L2：需交互式会话/真实显示器（specs §4）。显式运行：cargo test -p sr-itests --test shot -- --ignored --nocapture"]
fn it_screenshot_monitor_physical_size() {
    require_interactive_session();
    let _os = record_os_build();

    let mons = list_monitors().expect("前置：sr_enum::monitors::list_monitors 应成功（M1 已验证）");
    let sys = enum_sys_monitors();
    assert_eq!(
        mons.len(),
        sys.len(),
        "交叉验证前提：sr-enum 与 EnumDisplayMonitors 数量一致"
    );
    let out_dir = fresh_out_dir("dpi");

    let mut seen_paths: Vec<PathBuf> = Vec::new();
    for mon in &mons {
        // 直接 API 真值：按 (x,y,w,h) 在 DEVMODE 快照中定位同一台显示器
        let sys_m = sys
            .iter()
            .find(|s| {
                (s.x, s.y, s.width_px, s.height_px) == (mon.x, mon.y, mon.width_px, mon.height_px)
            })
            .unwrap_or_else(|| {
                panic!("前置：sr-enum 显示器条目（{mon:?}）在 DEVMODE 真值中无对应条目")
            });

        let shot = take_screenshot(ScreenshotRequest {
            target: ShotTarget::Monitor { index: mon.index },
            output_dir: out_dir.clone(),
        })
        .unwrap_or_else(|e| panic!("REQ-6：显示器 index={} 截图应成功：{e:?}", mon.index));

        assert_png_shape(&shot, &out_dir, &format!("显示器 index={}", mon.index));
        assert_eq!(
            (shot.width_px, shot.height_px),
            (sys_m.width_px, sys_m.height_px),
            "REQ-6：显示器 index={} 截图尺寸应为物理像素（DEVMODE 直接查询 {}x{}；得到 {}x{}）",
            mon.index,
            sys_m.width_px,
            sys_m.height_px,
            shot.width_px,
            shot.height_px
        );
        assert_eq!(
            shot.source,
            ShotTarget::Monitor { index: mon.index },
            "REQ-6：source 回执应为该显示器"
        );
        println!(
            "[dpi] monitor index={} at ({},{}) -> {}x{} ({})",
            mon.index,
            mon.x,
            mon.y,
            shot.width_px,
            shot.height_px,
            shot.path.display()
        );
        seen_paths.push(shot.path);
    }

    // 各显示器输出互不相同
    let mut sorted = seen_paths.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        mons.len(),
        "REQ-6：各显示器的输出文件应互不相同"
    );

    let _ = std::fs::remove_dir_all(&out_dir);
}

/// REQ-6：混合 DPI 多显示器下的区域截图——区域跨两台（不同缩放）显示器时按各显示器
/// 帧在物理坐标系拼合后裁剪；负坐标区域合法。
///
/// 拓扑探测：期望物理矩形全部由 DEVMODE（EnumDisplaySettingsW，物理像素/物理坐标）
/// 动态拼接计算，不硬编码坐标。两个子用例：
/// 1. **跨屏区域**：找水平方向最接近的一对显示器（贴合或间隙最小），区域以两屏交界
///    （或间隙中点）为中心横向跨两屏——输出尺寸必须=请求物理尺寸（捕获单屏再缩放、
///    或 DPI 虚拟化都会得错尺寸）；需 ≥2 台显示器，缺失 → BLOCKED（fail，§4）。
/// 2. **负坐标区域**：矩形位于原点为负方向的显示器内（x 或 y 为负）；本机拓扑无负
///    方向显示器时 → BLOCKED 并注明（跨屏断言已在上方执行，语义不削弱；供多屏
///    （负坐标布局）机器实测）。
#[test]
#[ignore = "L2：需交互式会话/多显示器拓扑（specs §4）。显式运行：cargo test -p sr-itests --test shot -- --ignored --nocapture"]
fn it_screenshot_mixed_dpi_negative_coords() {
    require_interactive_session();
    let _os = record_os_build();

    let mons = list_monitors().expect("前置：sr_enum::monitors::list_monitors 应成功（M1 已验证）");
    let sys = enum_sys_monitors();
    assert_eq!(
        mons.len(),
        sys.len(),
        "交叉验证前提：sr-enum 与 EnumDisplayMonitors 数量一致"
    );
    if sys.len() < 2 {
        blocked(&format!(
            "REQ-6 跨屏区域子用例需要 ≥2 台显示器，当前拓扑仅 {} 台（物理坐标无法构造同时覆盖两屏的矩形）",
            sys.len()
        ));
    }
    println!(
        "[mixdpi] 拓扑（DEVMODE 物理坐标）：{}",
        sys.iter()
            .map(|m| format!(
                "({},{}) {}x{}{}",
                m.x,
                m.y,
                m.width_px,
                m.height_px,
                if m.primary { " 主" } else { "" }
            ))
            .collect::<Vec<_>>()
            .join("；")
    );
    let out_dir = fresh_out_dir("mixdpi");

    // ── 子用例 1：跨两台显示器的区域（水平方向；期望矩形由 DEVMODE 拼接） ──
    // 找水平方向间隙最小的一对（A 左 B 右；Windows 不允许显示器矩形重叠，gap ≥ 0）
    let mut best: Option<(usize, usize)> = None;
    let mut best_gap = i32::MAX;
    for (i, a) in sys.iter().enumerate() {
        for (j, b) in sys.iter().enumerate() {
            if i == j {
                continue;
            }
            let a_right = a.x + a.width_px as i32;
            let gap = b.x - a_right;
            if gap < 0 {
                continue; // 重叠（异常拓扑），跳过该方向
            }
            let y0 = a.y.max(b.y);
            let y1 = (a.y + a.height_px as i32).min(b.y + b.height_px as i32);
            if y1 - y0 < 64 {
                continue; // 垂直重叠不足以放置区域
            }
            if gap < best_gap {
                best_gap = gap;
                best = Some((i, j));
            }
        }
    }
    let (a, b) = match best {
        Some((i, j)) => (&sys[i], &sys[j]),
        None => blocked("拓扑中不存在可构造水平跨屏区域的显示器对（垂直重叠 <64px 或全部重叠）"),
    };

    let a_right = a.x + a.width_px as i32;
    let boundary_x = (a_right + b.x) / 2; // 贴合时=物理交界；有间隙时=间隙中点
    let y0 = a.y.max(b.y);
    let y1 = (a.y + a.height_px as i32).min(b.y + b.height_px as i32);
    // 宽度须跨过交界（含间隙时覆盖两侧屏面）：w ≥ gap+64，且不超两屏宽度
    let w = 400u32
        .max((best_gap + 64) as u32)
        .min(a.width_px)
        .min(b.width_px)
        .max(32);
    let h = 300u32.min((y1 - y0) as u32).max(32);
    let x = (boundary_x - (w / 2) as i32).max(a.x); // 左缘留在 A 内
    let y = ((y0 + y1) / 2 - (h / 2) as i32).clamp(y0, y1 - h as i32);

    let rect = ScreenRect {
        x,
        y,
        width_px: w,
        height_px: h,
    };
    assert!(
        x < a_right && (x + w as i32) > b.x,
        "前置自检：跨屏矩形 [{x},{}+) 应同时覆盖 A（右缘 {a_right}）与 B（左缘 {}）",
        x + w as i32,
        b.x
    );
    let span_target = ShotTarget::Region { rect: rect.clone() };
    let span_shot = take_screenshot(ScreenshotRequest {
        target: span_target.clone(),
        output_dir: out_dir.clone(),
    })
    .expect("REQ-6：跨两台显示器的区域截图应成功");
    assert_png_shape(&span_shot, &out_dir, "跨屏区域");
    assert_eq!(
        (span_shot.width_px, span_shot.height_px),
        (w, h),
        "REQ-6：跨屏区域输出尺寸应=请求的物理像素宽高（按各显示器帧在物理坐标系拼合后裁剪；两屏缩放不同时按任一屏逻辑/虚拟坐标计算都会得错尺寸）"
    );
    assert_eq!(
        span_shot.source, span_target,
        "REQ-6：source 回执应与请求的跨屏矩形一致"
    );
    println!(
        "[mixdpi] 跨屏区域 A=({},{}) {}x{} × B=({},{}) {}x{} → rect=({x},{y}) {w}x{h} → {}",
        a.x,
        a.y,
        a.width_px,
        a.height_px,
        b.x,
        b.y,
        b.width_px,
        b.height_px,
        span_shot.path.display()
    );

    // ── 子用例 2：负坐标区域（需原点为负方向的显示器；无则 BLOCKED 记录本机限制） ──
    match sys.iter().find(|m| m.x < 0 || m.y < 0) {
        Some(m) => {
            let w = 320u32.min(m.width_px / 2).max(16);
            let h = 240u32.min(m.height_px / 2).max(16);
            // 负方向维度取负坐标（保证落在该显示器内）；正方向维度取 1/4 处
            let x = if m.x < 0 { (m.x + (m.width_px / 8) as i32).min(-1) } else { m.x + (m.width_px / 4) as i32 };
            let y = if m.y < 0 { (m.y + (m.height_px / 8) as i32).min(-1) } else { m.y + (m.height_px / 4) as i32 };
            let rect = ScreenRect { x, y, width_px: w, height_px: h };
            let neg_target = ShotTarget::Region { rect: rect.clone() };
            let neg_shot = take_screenshot(ScreenshotRequest {
                target: neg_target.clone(),
                output_dir: out_dir.clone(),
            })
            .expect("REQ-6：负坐标区域截图应成功（REQ-6：支持负坐标）");
            assert_png_shape(&neg_shot, &out_dir, "负坐标区域");
            assert_eq!(
                (neg_shot.width_px, neg_shot.height_px),
                (w, h),
                "REQ-6：负坐标区域输出尺寸应=请求的物理像素宽高"
            );
            assert_eq!(neg_shot.source, neg_target, "REQ-6：source 回执应与请求的负坐标矩形一致");
            println!("[mixdpi] 负坐标区域 rect=({x},{y}) {w}x{h} → {}", neg_shot.path.display());
        }
        None => blocked(
            "本机拓扑无负方向显示器（全部显示器位于桌面原点 (0,0) 的正方向）：REQ-6 负坐标区域子用例无法在此环境实测（上方跨屏断言已执行；语义不削弱，供负坐标布局的多屏机器验证）",
        ),
    }

    let _ = std::fs::remove_dir_all(&out_dir);
}

/// REQ-7：被遮挡窗口仍能截到内容而非空白（WGC 单帧语义）+ 最小化窗口返回明确错误。
///
/// 遮挡部分策略：自建红底目标窗口（先完整绘制）→ 自建青底覆盖窗口完全盖住目标客户
/// 区并置于其上（SetWindowPos(HWND_TOP)）→ 以 WindowFromPoint 确认遮挡地面真值 →
/// 截图 → 锁定 ffmpeg 解码 PNG 为 rgb24 做内容级判定：红占比 ≥0.30（空白帧≈0）且
/// 青占比 ≤0.01（屏幕 Blt 式误实现会把覆盖窗口拍进去）。避免仅看尺寸/文件存在的
/// 空洞通过。
/// 最小化部分策略：ShowWindow(SW_MINIMIZE) + IsIconic 地面真值确认后，截图必须返回
/// Err 且错误为契约钉死的 `ShotError::TargetMinimized`（明确错误，不得成功、不得
/// 含糊失败的其它错误变体）。
#[test]
#[ignore = "L2：需交互式会话/锁定 ffmpeg（specs §4）。显式运行：cargo test -p sr-itests --test shot -- --ignored --nocapture"]
fn it_screenshot_window_occluded_and_minimized_error() {
    require_interactive_session();
    let _os = record_os_build();

    let out_dir = fresh_out_dir("occl");

    // 目标窗口：创建并完整绘制（覆盖前必须先画，否则被盖住后可能永远收不到绘制）
    let target = ColoredWindow::new_target(&format!("SR-M2-RED-shot-occl-{}", std::process::id()));
    pump_messages(600);
    target.repaint_now();
    pump_messages(200);
    assert!(target.is_visible(), "前置：目标窗口应可见");

    // 覆盖窗口：环绕目标窗口外扩 20px，创建后置顶于目标之上
    let cover = ColoredWindow::new_cover_over(
        &target,
        &format!("SR-M2-RED-shot-cover-{}", std::process::id()),
    );
    pump_messages(600);

    // 遮挡地面真值：目标客户区中心的命中窗口必须是覆盖窗口
    let center = target.client_center_screen();
    let at_point = window_from_point(center);
    if at_point.0 != cover.hwnd.0 {
        blocked(&format!(
            "无法构造遮挡前置条件：目标客户区中心 ({},{}) 命中 hwnd={:#x}，期望覆盖窗口 {:#x}（环境有其它窗口置顶）",
            center.x, center.y, at_point.0 as usize, cover.hwnd.0 as usize
        ));
    }

    // ── 被测 API（REQ-7）──
    let win_target = ShotTarget::Window {
        hwnd: target.hwnd_u64,
    };
    let shot = take_screenshot(ScreenshotRequest {
        target: win_target.clone(),
        output_dir: out_dir.clone(),
    })
    .expect("REQ-7：被遮挡窗口截图应成功（WGC 单帧，无需窗口可见于最顶层）");
    assert_png_shape(&shot, &out_dir, "被遮挡窗口");
    assert!(
        shot.width_px > 0 && shot.height_px > 0,
        "REQ-7：截图尺寸应为正"
    );
    assert_eq!(
        shot.source, win_target,
        "REQ-7：source 回执应为被遮挡的目标窗口"
    );

    // ── 内容级判定（锁定 ffmpeg 解码 → rgb24 → 颜色占比） ──
    let rgb = decode_png_rgb24(&shot.path, shot.width_px, shot.height_px);
    let red = color_fraction_rgb(&rgb, shot.width_px, shot.height_px, TARGET_RGB, COLOR_TOL);
    let teal = color_fraction_rgb(&rgb, shot.width_px, shot.height_px, COVER_RGB, COLOR_TOL);
    println!(
        "[occluded] {w}x{h}: target-red={red:.3} (≥{OCCLUDED_TARGET_RED_MIN}), cover-teal={teal:.3} (≤{OCCLUDED_COVER_LEAK_MAX})",
        w = shot.width_px,
        h = shot.height_px
    );
    assert!(
        red >= OCCLUDED_TARGET_RED_MIN,
        "REQ-7：被遮挡窗口截图应含目标窗口内容（红占比 {red:.3} < {OCCLUDED_TARGET_RED_MIN}：疑似空白帧或未捕获窗口表面）"
    );
    assert!(
        teal <= OCCLUDED_COVER_LEAK_MAX,
        "REQ-7：被遮挡窗口截图不得包含覆盖窗口内容（青占比 {teal:.3} > {OCCLUDED_COVER_LEAK_MAX}：疑似按屏幕而非按窗口捕获）"
    );

    drop(cover);

    // ── 最小化窗口 → 明确错误（REQ-7 后半；契约：ShotError::TargetMinimized） ──
    minimize_window(target.hwnd);
    let iconic = wait_until(Duration::from_secs(5), Duration::from_millis(100), || {
        if is_iconic(target.hwnd) {
            Some(())
        } else {
            None
        }
    });
    assert!(
        iconic.is_some(),
        "前置：ShowWindow(SW_MINIMIZE) 后 5s 内 IsIconic 应为 true（地面真值）"
    );
    let err = take_screenshot(ScreenshotRequest {
        target: win_target.clone(),
        output_dir: out_dir.clone(),
    })
    .expect_err("REQ-7：最小化窗口截图应返回错误（不得成功；WGC 无法从最小化窗口取帧）");
    assert!(
        matches!(err, ShotError::TargetMinimized { .. }),
        "REQ-7：最小化窗口的错误应为明确的 ShotError::TargetMinimized（得到 {err:?}；不得含糊失败或借道其它错误变体）"
    );

    drop(target);
    pump_messages(200);
    let _ = std::fs::remove_dir_all(&out_dir);
}

// ─────────────────────────── 本文件内部工具 ───────────────────────────

/// 唯一输出目录（先清后建，避免上次运行残留干扰「不覆盖」类判断）。
fn fresh_out_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m2_red_shot_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("创建输出目录失败");
    d
}

/// REQ-5 形状断言：文件存在、扩展名 png、位于输出目录内、PNG 实际编码尺寸（IHDR）
/// 与结果元数据一致（钉死「元数据不是空头支票」）。
fn assert_png_shape(shot: &SavedScreenshot, out_dir: &Path, label: &str) {
    assert!(
        shot.path.exists(),
        "{label}：结果文件应存在（{}）",
        shot.path.display()
    );
    assert!(
        shot.path
            .extension()
            .map_or(false, |e| e.to_string_lossy().to_lowercase() == "png"),
        "{label}：输出应为 PNG 文件（{}）",
        shot.path.display()
    );
    let p = shot.path.canonicalize().unwrap_or_else(|e| {
        panic!(
            "{label}：结果路径应可 canonicalize（{}）：{e}",
            shot.path.display()
        )
    });
    let d = out_dir
        .canonicalize()
        .unwrap_or_else(|e| panic!("{label}：输出目录应可 canonicalize：{e}"));
    assert!(
        p.starts_with(&d),
        "{label}：PNG 应位于输出目录内（{} 不在 {} 下）",
        p.display(),
        d.display()
    );
    let (w, h) = png_ihdr_dims(&shot.path);
    assert_eq!(
        (w, h),
        (shot.width_px, shot.height_px),
        "{label}：PNG 实际编码尺寸（IHDR）应与结果元数据一致"
    );
}

/// 解析 PNG IHDR（无解码依赖）：返回 (width, height)。校验 8 字节签名 + 首块为 IHDR。
fn png_ihdr_dims(path: &Path) -> (u32, u32) {
    let bytes =
        std::fs::read(path).unwrap_or_else(|e| panic!("读取 PNG 失败 {}: {e}", path.display()));
    const SIG: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    assert!(
        bytes.len() >= 24,
        "PNG 过短（{} 字节）：{}",
        bytes.len(),
        path.display()
    );
    assert_eq!(&bytes[..8], &SIG, "PNG 签名不符：{}", path.display());
    let ihdr_len = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    assert_eq!(
        ihdr_len,
        13,
        "首个 chunk 长度应为 IHDR 的 13：{}",
        path.display()
    );
    assert_eq!(
        &bytes[12..16],
        b"IHDR",
        "首个 chunk 应为 IHDR：{}",
        path.display()
    );
    (
        u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]),
        u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]),
    )
}

/// ffplay/ffmpeg 位置：任务锁定的仓库内构建（与 enum_audio.rs 同一来源）。
fn ffmpeg_bin(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/ffmpeg/ffmpeg-master-latest-win64-lgpl/bin")
        .join(name)
        .canonicalize()
        .unwrap_or_else(|e| {
            blocked(&format!(
                "无法定位 tools/ffmpeg/.../{name}.exe（canonicalize 失败：{e}）"
            ))
        });
    if !p.exists() {
        blocked(&format!("锁定 ffmpeg 构建缺失：{}", p.display()));
    }
    p
}

/// 锁定 ffmpeg 把 PNG 解码为 rgb24 紧凑字节流（内容级判定；不新增 Rust 依赖）。
fn decode_png_rgb24(path: &Path, w: u32, h: u32) -> Vec<u8> {
    let out = Command::new(ffmpeg_bin("ffmpeg.exe"))
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn 锁定 ffmpeg 解码 PNG：{e}")));
    assert!(
        out.status.success(),
        "ffmpeg 解码 PNG 失败（exit={}）",
        out.status
    );
    let expect = w as usize * h as usize * 3;
    assert_eq!(
        out.stdout.len(),
        expect,
        "rgb24 解码字节数应为 {w}x{h}x3={expect}（得到 {}）",
        out.stdout.len()
    );
    out.stdout
}

/// 颜色占比：与目标 RGB 每通道差 ≤tol 的像素比例。
fn color_fraction_rgb(data: &[u8], w: u32, h: u32, rgb: (u8, u8, u8), tol: i32) -> f64 {
    let n = (w as usize) * (h as usize);
    let mut hit = 0usize;
    for i in 0..n {
        let (r, g, b) = (
            data[i * 3] as i32,
            data[i * 3 + 1] as i32,
            data[i * 3 + 2] as i32,
        );
        if (r - rgb.0 as i32).abs() <= tol
            && (g - rgb.1 as i32).abs() <= tol
            && (b - rgb.2 as i32).abs() <= tol
        {
            hit += 1;
        }
    }
    hit as f64 / n as f64
}

// ─────────────────────────── 彩色测试窗口（内容级判定载体） ───────────────────────────

static TARGET_CLASS: Once = Once::new();
static COVER_CLASS: Once = Once::new();
const TARGET_CLASS_NAME: &str = "SR_M2_RED_SHOT_TARGET";
const COVER_CLASS_NAME: &str = "SR_M2_RED_SHOT_COVER";

fn colorref(rgb: (u8, u8, u8)) -> windows::Win32::Foundation::COLORREF {
    windows::Win32::Foundation::COLORREF(
        rgb.0 as u32 | ((rgb.1 as u32) << 8) | ((rgb.2 as u32) << 16),
    )
}

/// WM_PAINT 走 BeginPaint/EndPaint：BeginPaint 触发 WM_ERASEBKGND（DefWindowProc 用
/// 类画刷填充纯色背景），EndPaint 验证更新区——纯色内容由系统绘制，无需应用自绘。
unsafe extern "system" fn paint_wndproc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wp: windows::Win32::Foundation::WPARAM,
    lp: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
    use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, WM_PAINT};
    if msg == WM_PAINT {
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let _dc = BeginPaint(hwnd, &mut ps);
            let _ = EndPaint(hwnd, &ps);
            return windows::Win32::Foundation::LRESULT(0);
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn register_colored_class(name: &str, rgb: (u8, u8, u8), once: &'static Once) {
    use windows::Win32::Graphics::Gdi::CreateSolidBrush;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::RegisterClassW;
    once.call_once(|| unsafe {
        let hinst = GetModuleHandleW(None).ok().expect("GetModuleHandleW");
        // 具名局部变量存活覆盖 RegisterClassW（M1 §9.1 悬垂教训）
        let class_name = windows::core::HSTRING::from(name);
        let brush = CreateSolidBrush(colorref(rgb));
        assert!(!brush.0.is_null(), "CreateSolidBrush 失败");
        let wc = windows::Win32::UI::WindowsAndMessaging::WNDCLASSW {
            lpfnWndProc: Some(paint_wndproc),
            hInstance: hinst.into(),
            hbrBackground: brush,
            lpszClassName: windows::core::PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        let atom = RegisterClassW(&wc);
        assert_ne!(atom, 0, "RegisterClassW({name}) 失败");
        // 类画刷归类所有，测试进程存活期内不删除（避免破坏仍存活的窗口类）
    });
}

/// 纯色背景的可见顶层窗口（红=被截目标 / 青=覆盖），在 Drop 时销毁。
pub struct ColoredWindow {
    pub hwnd: windows::Win32::Foundation::HWND,
    pub hwnd_u64: u64,
}

impl ColoredWindow {
    /// 红底目标窗口（CW_USEDEFAULT 定位，由系统保证落在可见桌面内）。
    fn new_target(title: &str) -> ColoredWindow {
        Self::create(TARGET_CLASS_NAME, title, None, 480, 320)
    }

    /// 青底覆盖窗口：环绕 `over` 的外扩 20px 矩形（同坐标空间，任意 DPI 缩放下保持覆盖），
    /// 创建后置顶于其上。
    fn new_cover_over(over: &ColoredWindow, title: &str) -> ColoredWindow {
        use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
        let mut rc = windows::Win32::Foundation::RECT::default();
        unsafe { GetWindowRect(over.hwnd, &mut rc).expect("GetWindowRect（覆盖定位）") };
        let (x, y, w, h) = (
            rc.left - 20,
            rc.top - 20,
            rc.right - rc.left + 40,
            rc.bottom - rc.top + 40,
        );
        let cover = Self::create(COVER_CLASS_NAME, title, Some((x, y)), w, h);
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        };
        unsafe {
            SetWindowPos(
                cover.hwnd,
                Some(HWND_TOP),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .expect("SetWindowPos(HWND_TOP) 置顶覆盖窗口");
        }
        cover
    }

    fn create(class: &str, title: &str, pos: Option<(i32, i32)>, w: i32, h: i32) -> ColoredWindow {
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
        };
        register_colored_class_for(class);
        unsafe {
            let hinst = GetModuleHandleW(None).ok().expect("GetModuleHandleW");
            let class_name = windows::core::HSTRING::from(class);
            let title_w = windows::core::HSTRING::from(title);
            let (x, y) = pos.unwrap_or((
                windows::Win32::UI::WindowsAndMessaging::CW_USEDEFAULT,
                windows::Win32::UI::WindowsAndMessaging::CW_USEDEFAULT,
            ));
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::PCWSTR::from_raw(class_name.as_ptr()),
                windows::core::PCWSTR::from_raw(title_w.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                x,
                y,
                w,
                h,
                None,
                None,
                Some(hinst.into()),
                None,
            )
            .expect("CreateWindowExW 失败：无法构造彩色测试窗口");
            ColoredWindow {
                hwnd,
                hwnd_u64: hwnd.0 as usize as u64,
            }
        }
    }

    /// 立即触发一次同步重绘（UpdateWindow 直发 WM_PAINT，不经过队列）。
    fn repaint_now(&self) {
        use windows::Win32::Graphics::Gdi::UpdateWindow;
        let _ = unsafe { UpdateWindow(self.hwnd) };
    }

    fn is_visible(&self) -> bool {
        use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    /// 客户区中心的屏幕坐标（遮挡判定点；与本进程 CreateWindow 同一坐标空间）。
    fn client_center_screen(&self) -> windows::Win32::Foundation::POINT {
        use windows::Win32::Graphics::Gdi::ClientToScreen;
        use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
        unsafe {
            let mut rc = windows::Win32::Foundation::RECT::default();
            GetClientRect(self.hwnd, &mut rc).expect("GetClientRect");
            let mut pt = windows::Win32::Foundation::POINT {
                x: (rc.left + rc.right) / 2,
                y: (rc.top + rc.bottom) / 2,
            };
            let _ = ClientToScreen(self.hwnd, &mut pt);
            pt
        }
    }

    fn destroy(&self) {
        use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

impl Drop for ColoredWindow {
    fn drop(&mut self) {
        self.destroy();
    }
}

/// 按类名选择对应的 Once/颜色注册（两个类各自只注册一次）。
fn register_colored_class_for(class: &str) {
    match class {
        TARGET_CLASS_NAME => register_colored_class(TARGET_CLASS_NAME, TARGET_RGB, &TARGET_CLASS),
        COVER_CLASS_NAME => register_colored_class(COVER_CLASS_NAME, COVER_RGB, &COVER_CLASS),
        other => panic!("未知测试窗口类：{other}"),
    }
}

/// 泵消息 duration 毫秒（窗口的绘制/位置生效都依赖消息循环分发）。
fn pump_messages(ms: u64) {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };
    let deadline = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < deadline {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                let _ = DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// 遮挡地面真值：给定屏幕点命中的顶层窗口 HWND（0=无）。
fn window_from_point(pt: windows::Win32::Foundation::POINT) -> windows::Win32::Foundation::HWND {
    use windows::Win32::UI::WindowsAndMessaging::WindowFromPoint;
    unsafe { WindowFromPoint(pt) }
}

/// 最小化窗口（ShowWindow(SW_MINIMIZE)）。
fn minimize_window(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_MINIMIZE};
    let _ = unsafe { ShowWindow(hwnd, SW_MINIMIZE) };
}

/// 最小化状态地面真值（IsIconic）。
fn is_iconic(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::IsIconic;
    unsafe { IsIconic(hwnd).as_bool() }
}
