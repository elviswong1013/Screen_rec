//! 固定输出画布（REQ-13/D6）：任意源帧 → 固定 {canvas_w×canvas_h} BGRA 画布 → NV12。
//!
//! 缩放/转换 = S1 定稿 CPU 路径（docs/spikes/s1/s1-pipeline-throughput.md §6）：
//! `fast_image_resize`（Bilinear 卷积，U8x4）等比缩放 + 整行 memcpy 黑边 compose
//! （studio black RGB16,16,16）+ 手工 BGRA→NV12（BT.601 limited，2×2 均值色度）。
//! 画布/缩放缓冲跨帧复用；源尺寸变化时缩放目标缓冲按需重建（REQ-13 的资源重建语义）。

use fast_image_resize::images::Image;
use fast_image_resize::{PixelType, ResizeAlg, ResizeOptions, Resizer};

/// 固定画布转换器（非线程安全；须在帧回调线程内使用）。
pub struct CanvasConverter {
    canvas_w: u32,
    canvas_h: u32,
    /// 画布 BGRA（黑边常驻；内容区每帧覆写）。
    canvas_bgra: Vec<u8>,
    /// 输出 NV12（W*H*3/2）。
    nv12: Vec<u8>,
    resizer: Resizer,
    /// 当前缩放目标缓冲（含其 fit 尺寸；源尺寸变化时重建）。
    fitted: Option<Image<'static>>,
}

impl CanvasConverter {
    pub fn new(canvas_w: u32, canvas_h: u32) -> Self {
        // NV12 编码要求偶数尺寸；防御性向下取偶（配置错误在此显式化）。
        let (canvas_w, canvas_h) = (canvas_w & !1, canvas_h & !1);
        assert!(canvas_w > 0 && canvas_h > 0, "画布尺寸必须为正");
        let mut canvas_bgra = vec![0u8; canvas_w as usize * canvas_h as usize * 4];
        // 黑边 = studio black RGB16,16,16（S1 §3d 信号实测基线）。
        for px in canvas_bgra.chunks_exact_mut(4) {
            px.copy_from_slice(&[16, 16, 16, 255]);
        }
        let mut nv12 = vec![0u8; canvas_w as usize * canvas_h as usize * 3 / 2];
        let (y_len, uv_len) = (
            canvas_w as usize * canvas_h as usize,
            canvas_w as usize * canvas_h as usize / 4,
        );
        nv12[..y_len].fill(16);
        nv12[y_len..y_len + uv_len * 2].fill(128);
        Self {
            canvas_w,
            canvas_h,
            canvas_bgra,
            nv12,
            resizer: Resizer::new(),
            fitted: None,
        }
    }

    pub fn canvas_size(&self) -> (u32, u32) {
        (self.canvas_w, self.canvas_h)
    }

    /// 将一帧紧凑 BGRA（`src_w×src_h`）缩放合成到固定画布并转为 NV12。
    /// 返回画布 NV12 帧（借用至下一次调用）。
    pub fn convert(&mut self, bgra: &[u8], src_w: u32, src_h: u32) -> &[u8] {
        let (canvas_w, canvas_h) = (self.canvas_w, self.canvas_h);
        if src_w == 0 || src_h == 0 {
            // 零尺寸帧（如最小化窗口占位）：跳过内容，画布保持黑（不产生撕裂）。
            self.nv12[..canvas_w as usize * canvas_h as usize].fill(16);
            let y_len = canvas_w as usize * canvas_h as usize;
            self.nv12[y_len..].fill(128);
            return &self.nv12;
        }

        // 等比缩放至画布内（偶数对齐；NV12 2×2 采样约束）。
        let s = ((canvas_w as f64 / src_w as f64).min(canvas_h as f64 / src_h as f64) * 2.0)
            .floor()
            / 2.0;
        let (fit_w, fit_h) = if s <= 0.0 {
            (canvas_w.min(src_w) & !1, canvas_h.min(src_h) & !1)
        } else {
            (
                ((src_w as f64 * s) as u32) & !1,
                ((src_h as f64 * s) as u32) & !1,
            )
        };
        let (fit_w, fit_h) = (fit_w.max(2).min(canvas_w), fit_h.max(2).min(canvas_h));
        let (off_x, off_y) = ((canvas_w - fit_w) / 2, (canvas_h - fit_h) / 2);

        let src_img = Image::from_vec_u8(src_w, src_h, bgra.to_vec(), PixelType::U8x4)
            .expect("源帧尺寸与字节长度不符");

        // 缩放目标缓冲复用：fit 尺寸未变则沿用（REQ-13 启动后恒定路径）。
        let rebuilt = match &self.fitted {
            Some(img) => img.width() != fit_w || img.height() != fit_h,
            None => true,
        };
        if rebuilt {
            self.fitted = Some(Image::new(fit_w, fit_h, PixelType::U8x4));
        }
        let fitted = self.fitted.as_mut().expect("缩放目标缓冲必须存在");
        let opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(
            fast_image_resize::FilterType::Bilinear,
        ));
        self.resizer
            .resize(&src_img, fitted, &opts)
            .expect("fast_image_resize 缩放失败");

        // 黑边 compose：内容区整行拷入画布（黑边常驻，未被覆盖处保持 RGB16）。
        if fit_w != canvas_w || fit_h != canvas_h {
            let fitted_buf = fitted.buffer();
            let fit_row = fit_w as usize * 4;
            let canvas_row = canvas_w as usize * 4;
            for row in 0..fit_h as usize {
                let d = (off_y as usize + row) * canvas_row + off_x as usize * 4;
                let sr = row * fit_row;
                self.canvas_bgra[d..d + fit_row].copy_from_slice(&fitted_buf[sr..sr + fit_row]);
            }
        } else {
            self.canvas_bgra.copy_from_slice(fitted.buffer());
        }

        bgra_to_nv12(&self.canvas_bgra, canvas_w, canvas_h, &mut self.nv12);
        &self.nv12
    }
}

/// BGRA → NV12（BT.601 limited；2×2 均值色度；S1/S1b 实测同款实现）。
fn bgra_to_nv12(bgra: &[u8], w: u32, h: u32, nv12: &mut [u8]) {
    let (w, h) = (w as usize, h as usize);
    debug_assert_eq!(nv12.len(), w * h * 3 / 2);
    let (y_plane, uv_plane) = nv12.split_at_mut(w * h);
    for y in (0..h).step_by(2) {
        let r0 = y * w;
        let r1 = r0 + w;
        for x in (0..w).step_by(2) {
            let i00 = (r0 + x) * 4;
            let i01 = i00 + 4;
            let i10 = (r1 + x) * 4;
            let i11 = i10 + 4;
            let (b0, g0, r0c) = (bgra[i00] as i32, bgra[i00 + 1] as i32, bgra[i00 + 2] as i32);
            let (b1, g1, r1c) = (bgra[i01] as i32, bgra[i01 + 1] as i32, bgra[i01 + 2] as i32);
            let (b2, g2, r2c) = (bgra[i10] as i32, bgra[i10 + 1] as i32, bgra[i10 + 2] as i32);
            let (b3, g3, r3c) = (bgra[i11] as i32, bgra[i11 + 1] as i32, bgra[i11 + 2] as i32);
            y_plane[r0 + x] = clamp_y((77 * r0c + 150 * g0 + 29 * b0) >> 8);
            y_plane[r0 + x + 1] = clamp_y((77 * r1c + 150 * g1 + 29 * b1) >> 8);
            y_plane[r1 + x] = clamp_y((77 * r2c + 150 * g2 + 29 * b2) >> 8);
            y_plane[r1 + x + 1] = clamp_y((77 * r3c + 150 * g3 + 29 * b3) >> 8);
            let bavg = (b0 + b1 + b2 + b3) >> 2;
            let gavg = (g0 + g1 + g2 + g3) >> 2;
            let ravg = (r0c + r1c + r2c + r3c) >> 2;
            // NV12 色度交错布局：每 2×2 块一个 (Cb,Cr) 对写入同一行（行宽 = w 字节）；
            // 平面式（Cb 全平面+Cr 全平面 = I420 形状）写入会致解码侧色度错位（M5 红报告 §8-1）。
            let uv_i = (y / 2) * w + (x / 2) * 2;
            uv_plane[uv_i] = clamp_uv(((-43 * ravg - 85 * gavg + 128 * bavg) >> 8) + 128);
            uv_plane[uv_i + 1] = clamp_uv(((128 * ravg - 107 * gavg - 21 * bavg) >> 8) + 128);
        }
    }
}

#[inline]
fn clamp_y(v: i32) -> u8 {
    v.clamp(16, 235) as u8
}

#[inline]
fn clamp_uv(v: i32) -> u8 {
    v.clamp(16, 240) as u8
}
