//! 区域截图的物理坐标拼合（REQ-6）：区域跨显示器时按各显示器物理帧在桌面物理
//! 坐标系裁剪拼合后输出；负坐标合法。每台显示器取各自原生物理分辨率帧——混合
//! DPI（如 150%/125%）下不会因按单屏逻辑坐标缩放而得错尺寸。

use crate::capture::{capture_monitor, list_physical_monitors, BgraFrame, PhysicalMonitor};
use crate::screenshot::{ScreenRect, ShotError};

/// 请求矩形 → 拼合画布（紧凑 BGRA）：对每台与矩形相交的显示器取物理帧，
/// 以「桌面物理坐标」求交（交点同时 expressed 为帧内偏移与画布内偏移）后逐行拷贝。
pub(crate) fn compose_region(rect: &ScreenRect) -> Result<BgraFrame, ShotError> {
    if rect.width_px == 0 || rect.height_px == 0 {
        return Err(ShotError::EmptyRegion { rect: rect.clone() });
    }
    let mons = list_physical_monitors();

    // 与每台显示器的物理交集（i64 中间量防 i32 溢出；显示器矩形互不重叠）
    let (rx0, ry0) = (rect.x as i64, rect.y as i64);
    let (rx1, ry1) = (rx0 + rect.width_px as i64, ry0 + rect.height_px as i64);
    let mut hits: Vec<RegionHit<'_>> = Vec::new();
    for mon in &mons {
        let (mx0, my0) = (mon.x as i64, mon.y as i64);
        let (mx1, my1) = (mx0 + mon.width_px as i64, my0 + mon.height_px as i64);
        let ix0 = rx0.max(mx0);
        let iy0 = ry0.max(my0);
        let ix1 = rx1.min(mx1);
        let iy1 = ry1.min(my1);
        if ix1 > ix0 && iy1 > iy0 {
            hits.push(RegionHit {
                monitor: mon,
                frame_offset: ((ix0 - mx0) as u32, (iy0 - my0) as u32),
                canvas_offset: ((ix0 - rx0) as u32, (iy0 - ry0) as u32),
                size: ((ix1 - ix0) as u32, (iy1 - iy0) as u32),
            });
        }
    }
    if hits.is_empty() {
        return Err(ShotError::EmptyRegion { rect: rect.clone() });
    }

    let mut canvas = BgraFrame {
        width: rect.width_px,
        height: rect.height_px,
        data: vec![0u8; rect.width_px as usize * rect.height_px as usize * 4],
    };
    for hit in &hits {
        let frame = capture_monitor(hit.monitor)?; // 帧尺寸已校验 = 显示器物理尺寸
        blit(&mut canvas, &frame, hit);
    }
    Ok(canvas)
}

struct RegionHit<'a> {
    monitor: &'a PhysicalMonitor,
    /// 交集在该显示器帧内的物理偏移。
    frame_offset: (u32, u32),
    /// 交集在输出画布内的偏移（相对请求矩形原点）。
    canvas_offset: (u32, u32),
    /// 交集物理尺寸。
    size: (u32, u32),
}

/// 逐行拷贝交集区域：帧（源）→ 画布（目标），均按紧凑 BGRA 行。
fn blit(canvas: &mut BgraFrame, frame: &BgraFrame, hit: &RegionHit<'_>) {
    let (cw, ch) = hit.size;
    let src_stride = frame.width as usize * 4;
    let dst_stride = canvas.width as usize * 4;
    let copy_len = cw as usize * 4;
    let (sx, sy) = (hit.frame_offset.0 as usize, hit.frame_offset.1 as usize);
    let (dx, dy) = (hit.canvas_offset.0 as usize, hit.canvas_offset.1 as usize);
    for row in 0..ch as usize {
        let src_off = (sy + row) * src_stride + sx * 4;
        let dst_off = (dy + row) * dst_stride + dx * 4;
        canvas.data[dst_off..dst_off + copy_len]
            .copy_from_slice(&frame.data[src_off..src_off + copy_len]);
    }
}
