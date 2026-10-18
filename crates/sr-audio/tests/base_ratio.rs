//! M15/REQ-6：`DriftResampler` 基础比率（捕获率 → 输出率）单测。
//!
//! 契约（specs/002-recording-modes.md §3 REQ-6）：
//! - `new(channels, in_rate, out_rate)`；`base = out/in`；
//! - 漂移叠加 `ratio = base / (1 + ppm/1e6)`（ppm 限幅 ±1000 不变，是相对基础比率的微调）；
//! - `base != 1` 时 0ppm **不得直通**；补齐静音、延迟记账、停止 flush 覆盖转换路径；
//! - `base == 1` 且 0ppm 保持逐样本恒等直通（48k 基线行为不变）。

use sr_audio::resample::DriftResampler;

/// 立体声正弦（`rate` 采样率，`hz` 音调）。
fn sine_stereo(frames: usize, hz: f64, rate: u32) -> Vec<f32> {
    let mut v = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let s = (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin() as f32 * 0.5;
        v.push(s);
        v.push(s);
    }
    v
}

fn total_frames(v: &[f32]) -> usize {
    v.len() / 2
}

/// process + flush 的总输出帧数（冷启动延迟已由 flush 补齐）。
fn process_and_flush(r: &mut DriftResampler, input: &[f32]) -> Vec<f32> {
    let mut out = r.process(input);
    out.extend_from_slice(&r.flush());
    out
}

fn assert_close(got: f64, want: f64, tol: f64, label: &str) {
    assert!(
        (got - want).abs() <= tol,
        "{label}：应≈{want:.2}（得到 {got:.2}，容差 {tol}）"
    );
}

#[test]
fn unit_resample_base_ratio_96000_to_48000() {
    let in_rate = 96_000u32;
    let out_rate = 48_000u32;
    let base = out_rate as f64 / in_rate as f64;
    let input = sine_stereo(in_rate as usize, 440.0, in_rate); // 1s
    let mut r = DriftResampler::new(2, in_rate, out_rate);
    let delay = r.delay_frames();
    assert!(
        (60.0 * base..=68.0 * base).contains(&delay),
        "96k→48k 冷启动内容延迟应≈63×base 输出帧（得到 {delay}）"
    );
    let out = process_and_flush(&mut r, &input);
    let frames = total_frames(&out);
    assert_close(
        frames as f64,
        in_rate as f64 * base + delay,
        2.0,
        "96k→48k 输出帧数",
    );
    assert!(out.iter().all(|s| s.is_finite()), "输出不得含 NaN/Inf");
}

#[test]
fn unit_resample_base_ratio_48000_to_96000() {
    let in_rate = 48_000u32;
    let out_rate = 96_000u32;
    let base = out_rate as f64 / in_rate as f64;
    let input = sine_stereo(in_rate as usize, 440.0, in_rate);
    let mut r = DriftResampler::new(2, in_rate, out_rate);
    let delay = r.delay_frames();
    let out = process_and_flush(&mut r, &input);
    let frames = total_frames(&out);
    assert_close(
        frames as f64,
        in_rate as f64 * base + delay,
        2.0,
        "48k→96k 输出帧数",
    );
    assert!(
        frames > in_rate as usize + 1000,
        "48k→96k 应升采样（得到 {frames} 帧）"
    );
    assert!(out.iter().all(|s| s.is_finite()));
}

#[test]
fn unit_resample_base_ratio_48000_to_44100() {
    let in_rate = 48_000u32;
    let out_rate = 44_100u32;
    let base = out_rate as f64 / in_rate as f64;
    let input = sine_stereo(in_rate as usize, 440.0, in_rate);
    let mut r = DriftResampler::new(2, in_rate, out_rate);
    let delay = r.delay_frames();
    let out = process_and_flush(&mut r, &input);
    let frames = total_frames(&out);
    assert_close(
        frames as f64,
        in_rate as f64 * base + delay,
        2.0,
        "48k→44.1k 输出帧数",
    );
    assert!(
        frames < in_rate as usize,
        "48k→44.1k 应降采样（得到 {frames}）"
    );
    assert!(out.iter().all(|s| s.is_finite()));
}

#[test]
fn unit_resample_base_ratio_96000_to_44100() {
    let in_rate = 96_000u32;
    let out_rate = 44_100u32;
    let base = out_rate as f64 / in_rate as f64;
    let input = sine_stereo(in_rate as usize, 440.0, in_rate);
    let mut r = DriftResampler::new(2, in_rate, out_rate);
    let delay = r.delay_frames();
    let out = process_and_flush(&mut r, &input);
    let frames = total_frames(&out);
    assert_close(
        frames as f64,
        in_rate as f64 * base + delay,
        2.0,
        "96k→44.1k 输出帧数",
    );
    assert!(out.iter().all(|s| s.is_finite()));
}

/// base != 1 时 0ppm 不得直通（不得按 1:1 输出）。
#[test]
fn unit_resample_base_ratio_zero_ppm_is_not_passthrough() {
    let input = sine_stereo(4_800, 440.0, 48_000);

    let mut down = DriftResampler::new(2, 48_000, 44_100);
    let down_delay = down.delay_frames();
    let down_out = process_and_flush(&mut down, &input);
    let expect_down = 4_800f64 * (44_100.0 / 48_000.0) + down_delay;
    assert_close(
        total_frames(&down_out) as f64,
        expect_down,
        2.0,
        "base=0.91875 且 0ppm 应转换（不是 4800 帧直通）",
    );

    let mut up = DriftResampler::new(2, 48_000, 96_000);
    let up_out = process_and_flush(&mut up, &input);
    assert!(
        total_frames(&up_out) > 8_000,
        "base=2 且 0ppm 应输出约 2 倍帧数（得到 {}；直通会≈4800）",
        total_frames(&up_out)
    );
}

/// ±ppm 是在基础比率上的微调；回到 Hold（0ppm）恢复基础比率。
#[test]
fn unit_resample_base_ratio_ppm_adjusts_around_base_and_hold_recovers() {
    let in_rate = 96_000u32;
    let out_rate = 48_000u32;
    let base = out_rate as f64 / in_rate as f64;
    let input = sine_stereo(in_rate as usize, 440.0, in_rate);

    for ppm in [1_000.0f64, -1_000.0] {
        let ratio = base / (1.0 + ppm / 1e6);
        let delay = 63.0 * ratio;
        let mut r = DriftResampler::new(2, in_rate, out_rate);
        r.set_rate_ppm(ppm);
        let out = process_and_flush(&mut r, &input);
        assert_close(
            total_frames(&out) as f64,
            in_rate as f64 * ratio + delay,
            2.0,
            &format!("ppm={ppm} 输出帧数"),
        );
    }

    // 恢复 Hold：先进入 ±ppm，再回 0；0ppm 仍按 base 转换（不退回直通）。
    let mut r = DriftResampler::new(2, in_rate, out_rate);
    r.set_rate_ppm(1_000.0);
    r.set_rate_ppm(0.0);
    let delay = r.delay_frames();
    let out = process_and_flush(&mut r, &input);
    assert_close(
        total_frames(&out) as f64,
        in_rate as f64 * base + delay,
        2.0,
        "Hold 恢复基础比率",
    );
}

/// 分块处理与整块处理等价（转换路径跨块相位连续）。
#[test]
fn unit_resample_base_ratio_chunked_equals_oneshot() {
    let in_rate = 96_000u32;
    let out_rate = 48_000u32;
    let input = sine_stereo(in_rate as usize / 2, 440.0, in_rate);

    let mut one = DriftResampler::new(2, in_rate, out_rate);
    let a = process_and_flush(&mut one, &input);

    let mut chunked = DriftResampler::new(2, in_rate, out_rate);
    let mut b = Vec::new();
    for chunk in input.chunks(2 * 317) {
        b.extend_from_slice(&chunked.process(chunk));
    }
    b.extend_from_slice(&chunked.flush());

    assert_eq!(a.len(), b.len(), "分块不应改变输出帧数");
    let max = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);
    assert!(max < 1e-5, "分块输出应逐样本一致（最大差 {max}）");
}

/// base == 1 且 0ppm：既有恒等直通契约保持不变（48k 基线）。
#[test]
fn unit_resample_base_ratio_identity_only_when_base_is_one() {
    let input = sine_stereo(4_800, 440.0, 48_000);
    let mut r = DriftResampler::new(2, 48_000, 48_000);
    assert_eq!(r.delay_frames(), 1.0, "base=1 直通仅尾部 1 帧延迟");
    let mut out = r.process(&input);
    assert_eq!(total_frames(&out), 4_799, "0ppm 应逐帧恒等（尾部 1 帧）");
    for (i, (a, b)) in out.iter().zip(input.iter()).enumerate() {
        assert!((a - b).abs() < 1e-6, "样本 {i} 应恒等（{a} vs {b}）");
    }
    out.extend_from_slice(&r.flush());
    assert_eq!(total_frames(&out), 4_800, "flush 应补出保留帧");
}
