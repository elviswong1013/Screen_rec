//! 区域视频捕获基础（M18/REQ-21/22/24）：帧内裁剪坐标、失效闩锁与同尺寸 NV12。
//!
//! 关键约束（specs/002-recording-modes.md §3.4）：
//! - 裁剪走 `windows-capture` 的 `Frame::buffer_crop`（锁定的 2.0.1 **不检查终点越界**，
//!   调用前必须由 [`RegionCrop::validate_for_frame`] 校验 `0 ≤ start < end ≤ frame`）；
//! - 裁剪后直接按**实际区域尺寸**转 NV12（禁止画布缩放/letterbox）；
//! - 生产者（WGC 回调线程）在 capture_ended **之前**同步闩锁首因；worker 结束后重读，
//!   杜绝「检查为空 → 事件入队 → 结束」竞态。

use std::sync::{Arc, Mutex};

use crate::canvas::bgra_to_nv12;

/// 单显示器帧内裁剪矩形（半开区间；相对显示器帧左上角）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionCrop {
    pub start_x: u32,
    pub start_y: u32,
    pub end_x: u32,
    pub end_y: u32,
}

impl RegionCrop {
    /// 构造并校验非空区间（`start < end`；不检查帧终点——见 [`Self::validate_for_frame`]）。
    pub fn new(start_x: u32, start_y: u32, end_x: u32, end_y: u32) -> Result<Self, String> {
        if start_x >= end_x || start_y >= end_y {
            return Err(format!(
                "区域裁剪矩形无效（start=({start_x},{start_y}) end=({end_x},{end_y})）"
            ));
        }
        Ok(Self {
            start_x,
            start_y,
            end_x,
            end_y,
        })
    }

    pub fn width(&self) -> u32 {
        self.end_x - self.start_x
    }

    pub fn height(&self) -> u32 {
        self.end_y - self.start_y
    }

    /// 调用 `Frame::buffer_crop` **之前**的边界校验（锁定库只检查 start<end）。
    pub fn validate_for_frame(&self, frame_width: u32, frame_height: u32) -> Result<(), String> {
        if self.start_x >= self.end_x
            || self.start_y >= self.end_y
            || self.end_x > frame_width
            || self.end_y > frame_height
        {
            return Err(format!(
                "区域裁剪越界（crop=({},{})..({},{}) frame={frame_width}x{frame_height}）",
                self.start_x, self.start_y, self.end_x, self.end_y
            ));
        }
        if self.width() < 2 || self.height() < 2 {
            return Err(format!(
                "区域裁剪尺寸过小（{}x{}；NV12 要求 ≥2x2）",
                self.width(),
                self.height()
            ));
        }
        Ok(())
    }
}

/// 区域帧校验（生产者回调；M18/REQ-24 分辨率变化/裁剪越界 → 停路保留原因）。
///
/// `frame_*` = WGC 实际帧尺寸；启动时 `full_*` = 显示器物理尺寸。任何不一致都返回
/// 首因字符串（调用方同步闩锁后停采集），不得缩放/重复旧帧。
pub fn validate_region_frame(
    crop: &RegionCrop,
    full_width: u32,
    full_height: u32,
    frame_width: u32,
    frame_height: u32,
) -> Result<(), String> {
    if frame_width != full_width || frame_height != full_height {
        return Err(format!(
            "显示器分辨率变化（启动 {full_width}x{full_height}，当前 {frame_width}x{frame_height}）"
        ));
    }
    crop.validate_for_frame(frame_width, frame_height)
        .map_err(|e| format!("区域裁剪越界：{e}"))
}

/// 区域捕获失效首因闩锁（生产者写、worker 读；首因生效）。
#[derive(Clone, Debug, Default)]
pub struct CaptureLossLatch {
    inner: Arc<Mutex<Option<String>>>,
}

impl CaptureLossLatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// 同步闩锁首因（生产者：WGC `on_closed` / 尺寸变化 / crop 越界）。
    pub fn latch(&self, reason: impl Into<String>) {
        let mut guard = self.inner.lock().expect("region loss 锁");
        if guard.is_none() {
            *guard = Some(reason.into());
        }
    }

    pub fn get(&self) -> Option<String> {
        self.inner.lock().expect("region loss 锁").clone()
    }

    pub fn is_set(&self) -> bool {
        self.inner.lock().expect("region loss 锁").is_some()
    }
}

/// 裁剪后同尺寸 BGRA → NV12 转换器（无缩放、无黑边；区域尺寸＝唯一编码尺寸）。
pub struct RegionConverter {
    width: u32,
    height: u32,
    nv12: Vec<u8>,
}

impl RegionConverter {
    pub fn new(width: u32, height: u32) -> Self {
        assert!(
            width >= 2 && height >= 2 && width % 2 == 0 && height % 2 == 0,
            "区域 NV12 尺寸必须为 ≥2 的偶数（得到 {width}x{height}）"
        );
        Self {
            width,
            height,
            nv12: vec![0u8; width as usize * height as usize * 3 / 2],
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 将裁剪后的紧凑 BGRA 转为同尺寸 NV12（尺寸不符 → 明确 panic，属实现错误）。
    pub fn convert_exact(&mut self, bgra: &[u8], width: u32, height: u32) -> &[u8] {
        assert_eq!(
            (width, height),
            (self.width, self.height),
            "区域 BGRA 尺寸必须与转换器一致"
        );
        assert_eq!(
            bgra.len(),
            width as usize * height as usize * 4,
            "区域 BGRA 字节长度不符"
        );
        bgra_to_nv12(bgra, width, height, &mut self.nv12);
        &self.nv12
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_region_crop_bounds_rejects_end_past_frame() {
        let crop = RegionCrop::new(10, 20, 110, 80).expect("有效矩形");
        assert_eq!((crop.width(), crop.height()), (100, 60));
        crop.validate_for_frame(200, 100).expect("边界内");
        let err = crop
            .validate_for_frame(100, 100)
            .expect_err("end_x 越界必须拒绝（锁定库不检查终点）");
        assert!(err.contains("越界"), "得到 {err}");
        let err = RegionCrop::new(5, 5, 5, 9).expect_err("零宽必须拒绝");
        assert!(err.contains("无效"), "得到 {err}");
    }

    #[test]
    fn unit_region_crop_requires_even_nv12_minimum() {
        let crop = RegionCrop::new(0, 0, 1, 10).expect("构造允许");
        assert!(crop.validate_for_frame(100, 100).is_err(), "1px 宽不可编码");
    }

    #[test]
    fn unit_region_frame_resolution_change_and_crop_bounds_latch_reason() {
        let crop = RegionCrop::new(100, 100, 502, 400).expect("crop");
        // 分辨率变化（帧尺寸 != 启动显示器尺寸）→ 明确原因。
        let err =
            validate_region_frame(&crop, 1920, 1080, 1280, 720).expect_err("分辨率变化必须停路");
        assert!(err.contains("分辨率变化"), "得到 {err}");
        // crop 终点越界（锁定库不检查）→ 明确原因（尺寸与启动一致时仍越界）。
        let err = validate_region_frame(&crop, 300, 300, 300, 300).expect_err("crop 越界必须停路");
        assert!(err.contains("越界"), "得到 {err}");
        validate_region_frame(&crop, 1920, 1080, 1920, 1080).expect("有效帧");
    }

    #[test]
    fn unit_region_loss_latch_first_cause_wins() {
        let latch = CaptureLossLatch::new();
        assert!(!latch.is_set());
        latch.latch("显示器移除");
        latch.latch("几何变化");
        assert_eq!(latch.get().as_deref(), Some("显示器移除"));
    }

    #[test]
    fn unit_region_converter_is_same_size_nv12() {
        let mut conv = RegionConverter::new(4, 2);
        let bgra = vec![0u8; 4 * 2 * 4];
        let out = conv.convert_exact(&bgra, 4, 2);
        assert_eq!(out.len(), 4 * 2 * 3 / 2);
        assert_eq!(conv.size(), (4, 2));
    }
}
