//! P1-2：变比重采样器（REQ-26/D7 录制管线路径）单测。
//!
//! 契约：
//! - 0ppm 恒等（分块处理与整块一致；流末 ≤1 帧历史延迟）；
//! - ±500ppm 输出帧数 ≈ 输入/step（±2 帧）；跨块相位连续（无 NaN/无爆音）；
//! - 速率限幅（±1000ppm 外不改变 step）。

use sr_audio::resample::DriftResampler;

fn sine_stereo(frames: usize, hz: f64) -> Vec<f32> {
    let mut v = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let s = (2.0 * std::f64::consts::PI * hz * i as f64 / 48_000.0).sin() as f32 * 0.5;
        v.push(s);
        v.push(s);
    }
    v
}

#[test]
fn zero_ppm_is_identity_across_chunks() {
    let input = sine_stereo(4800, 440.0);
    let mut r = DriftResampler::new(2);
    let mut out = Vec::new();
    for chunk in input.chunks(2 * 317) {
        out.extend(r.process(chunk));
    }
    // 流末保留 1 帧作为下一块左端点（≤1 帧延迟）。
    assert_eq!(
        out.len() / 2,
        input.len() / 2 - 1,
        "0ppm 应逐帧恒等（仅尾部 1 帧延迟）"
    );
    for (i, (a, b)) in out.iter().zip(input.iter()).enumerate() {
        assert!((a - b).abs() < 1e-6, "样本 {i} 应恒等（{a} vs {b}）");
    }
}

#[test]
fn positive_500ppm_shortens_and_negative_lengthens() {
    let input = sine_stereo(48_000, 440.0); // 1s
    let mut fast = DriftResampler::new(2);
    fast.set_rate_ppm(500.0);
    let out_fast = fast.process(&input);
    let mut slow = DriftResampler::new(2);
    slow.set_rate_ppm(-500.0);
    let out_slow = slow.process(&input);

    let frames = |v: &[f32]| v.len() / 2;
    let expect_fast = (48_000f64 / 1.0005) as usize;
    let expect_slow = (48_000f64 / 0.9995) as usize;
    assert!(
        (frames(&out_fast) as i64 - expect_fast as i64).abs() <= 2,
        "+500ppm 输出帧数应≈{expect_fast}（得到 {}）",
        frames(&out_fast)
    );
    assert!(
        (frames(&out_slow) as i64 - expect_slow as i64).abs() <= 2,
        "-500ppm 输出帧数应≈{expect_slow}（得到 {}）",
        frames(&out_slow)
    );
}

#[test]
fn chunked_and_oneshot_are_equivalent() {
    let input = sine_stereo(20_000, 1000.0);
    let mut one = DriftResampler::new(2);
    one.set_rate_ppm(300.0);
    let a = one.process(&input);
    let mut chunked = DriftResampler::new(2);
    chunked.set_rate_ppm(300.0);
    let mut b = Vec::new();
    for chunk in input.chunks(2 * 511) {
        b.extend(chunked.process(chunk));
    }
    assert_eq!(a.len(), b.len(), "分块不应改变输出帧数");
    let max = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);
    assert!(max < 1e-5, "分块相位应连续（max Δ={max}）");
    assert!(a.iter().all(|s| s.is_finite()));
}

#[test]
fn rate_is_clamped_to_1000ppm() {
    let mut r = DriftResampler::new(2);
    r.set_rate_ppm(1e9);
    let out = r.process(&sine_stereo(48_000, 440.0));
    let expect = (48_000f64 / 1.001) as usize;
    assert!(
        (out.len() / 2).abs_diff(expect) <= 2,
        "超限输入应夹到 1000ppm（期望≈{expect}，得到 {}）",
        out.len() / 2
    );
}

// ── M12-B：rubato sinc 路径的比率/延迟正确性证据（已知漂移输入）──

/// REQ-26：脉冲在输出中的位置 = 输入位置 × 比率 + 已记账的固定延迟；
/// 延迟为常量（两枚脉冲间距只随比率缩放，不额外累积），且等于 `delay_frames()`。
#[test]
fn impulse_delay_matches_accounted_filter_delay() {
    let ppm = 500.0;
    let ratio = 1.0 / (1.0 + ppm / 1e6);
    let mut r = DriftResampler::new(2);
    r.set_rate_ppm(ppm);
    let delay = r.delay_frames();
    assert!(
        (60.0..=68.0).contains(&delay),
        "rubato sinc 冷启动延迟应≈sinc_len/2 输出帧（得到 {delay}）"
    );

    let n = 24_000usize;
    let at = [1000usize, 20_000usize];
    let mut input = vec![0.0f32; n * 2];
    for p in at {
        input[p * 2] = 1.0;
        input[p * 2 + 1] = 1.0;
    }
    let out = r.process(&input);
    let frames = out.len() / 2;

    let peak_near = |center: usize, span: usize| -> (usize, f32) {
        let lo = center.saturating_sub(span);
        let hi = (center + span).min(frames - 1);
        let mut best = (lo, 0.0f32);
        for i in lo..=hi {
            let v = out[i * 2].abs();
            if v > best.1 {
                best = (i, v);
            }
        }
        best
    };

    let mut peaks = Vec::new();
    for &p in &at {
        let expected = p as f64 * ratio + delay;
        let (got, amp) = peak_near(expected as usize, 80);
        assert!(
            (got as f64 - expected).abs() <= 1.5,
            "脉冲 {p} 应出现在输出 {expected:.1} 帧（得到 {got}；延迟 {delay}）"
        );
        assert!(amp > 0.5, "脉冲 {p} 峰值幅度应接近 1（得到 {amp}）");
        peaks.push(got);
    }
    let spacing = (peaks[1] - peaks[0]) as f64;
    let expected_spacing = (at[1] - at[0]) as f64 * ratio;
    assert!(
        (spacing - expected_spacing).abs() <= 1.5,
        "延迟不得累积：两脉冲间距应≈{expected_spacing:.1}（得到 {spacing}）"
    );
}

/// REQ-26：已知漂移下输出内容按补偿比率缩放（相位/幅度对齐理想重采样）。
#[test]
fn sine_content_matches_compensated_ratio() {
    for ppm in [500.0f64, -500.0] {
        let step = 1.0 + ppm / 1e6;
        let ratio = 1.0 / step;
        let input = sine_stereo(48_000, 440.0);
        let mut r = DriftResampler::new(2);
        r.set_rate_ppm(ppm);
        let delay = r.delay_frames();
        let out = r.process(&input);
        let frames = out.len() / 2;
        let expect_frames = (48_000f64 * ratio) as usize;
        assert!(
            frames.abs_diff(expect_frames) <= 2,
            "ppm={ppm}：输出帧数应≈{expect_frames}（得到 {frames}）"
        );

        // 输出第 i 帧对应输入时间 (i - delay) × step；跳过启动斜坡与尾部
        // 零前瞻（滤波器群延迟量级）后逐样本比对。
        let guard = delay.ceil() as usize;
        let start = guard + 400;
        let end = frames.saturating_sub(guard + 20);
        assert!(start < end, "比对区间应非空（start={start}, end={end}）");
        let mut max_err = 0.0f64;
        for i in start..end {
            let t = (i as f64 - delay) * step;
            let expect = (2.0 * std::f64::consts::PI * 440.0 * t / 48_000.0).sin() * 0.5;
            max_err = max_err.max((out[i * 2] as f64 - expect).abs());
        }
        assert!(
            max_err < 1e-3,
            "ppm={ppm}：正弦应满足补偿比率重采样（最大误差 {max_err}）"
        );
    }
}

/// REQ-26：直通后首次进入校正时完成延迟补偿——输出无重放/无跳帧、长度不漂移。
#[test]
fn midstream_engagement_compensates_delay_without_replay() {
    let input = sine_stereo(4800, 440.0);
    let mut r = DriftResampler::new(2);
    let mut out = Vec::new();
    for chunk in input[..2400 * 2].chunks(480 * 2) {
        out.extend(r.process(chunk));
    }
    let seam = out.len() / 2;
    assert_eq!(r.delay_frames(), 1.0, "直通路径仅保留 1 帧尾部");
    r.set_rate_ppm(500.0);
    assert_eq!(r.delay_frames(), 0.0, "中途接入应补偿 rubato 固定延迟");
    for chunk in input[2400 * 2..].chunks(480 * 2) {
        out.extend(r.process(chunk));
    }

    // 中途补偿把 rubato 的群延迟从“内容滞后”转为“流末固定短 1 个群延迟”
    // （冷启动同量延迟表现为开头斜坡）；其余帧数仍按比率累积。
    let mut probe = DriftResampler::new(2);
    probe.set_rate_ppm(500.0);
    let filter_delay = probe.delay_frames();
    let ratio = 1.0 / 1.0005;
    let expect_frames = 4800f64 * ratio - filter_delay;
    assert!(
        ((out.len() / 2) as f64 - expect_frames).abs() <= 4.0,
        "接入前后总帧数应≈{expect_frames:.1}（得到 {}）",
        out.len() / 2
    );
    // 接缝后内容应从直通保留的下一帧接续，并按 step 累积（无重放/跳帧）。
    let step = 1.0005;
    let mut max_dev = 0.0f64;
    for i in 300..4600 {
        let t = if i < seam {
            i as f64
        } else {
            seam as f64 + (i - seam) as f64 * step
        };
        let expect = (2.0 * std::f64::consts::PI * 440.0 * t / 48_000.0).sin() * 0.5;
        max_dev = max_dev.max((out[i * 2] as f64 - expect).abs());
    }
    // 补偿按整数输出帧丢弃，允许 <0.1 帧的舍入残差（440Hz 斜率 0.029/帧）。
    assert!(
        max_dev < 3e-3,
        "校正起点应与直通流无缝衔接（最大偏差 {max_dev}）"
    );
    assert!(out.iter().all(|s| s.is_finite()));
}

/// REQ-26：运行中切换速率（Hold↔Correct）保持有限输出与比率累积正确。
#[test]
fn rate_changes_keep_stream_finite_and_ratio_correct() {
    let input = sine_stereo(4800, 440.0);
    let mut r = DriftResampler::new(2);
    let mut emitted = 0usize;
    let mut all = Vec::new();
    for ppm in [500.0f64, 0.0, -500.0] {
        r.set_rate_ppm(ppm);
        let out = r.process(&input);
        emitted += out.len() / 2;
        all.extend_from_slice(&out);
    }
    let expect = 4800.0 / 1.0005 + 4800.0 + 4800.0 / 0.9995;
    assert!(
        (emitted as f64 - expect).abs() <= 4.0,
        "分段速率输出帧数应≈{expect:.1}（得到 {emitted}）"
    );
    assert!(all.iter().all(|s| s.is_finite()));
}

/// REQ-26 停止冲刷：冷启动路径把滤波延迟对应的尾部补齐，总帧数符合理论。
#[test]
fn flush_recovers_filter_tail_and_matches_theory() {
    let ppm = 500.0;
    let ratio = 1.0 / (1.0 + ppm / 1e6);
    let mut r = DriftResampler::new(2);
    r.set_rate_ppm(ppm);
    let delay = r.delay_frames();
    let input = sine_stereo(48_000, 440.0);
    let mut out = r.process(&input);
    let before = out.len() / 2;
    let tail = r.flush();
    out.extend_from_slice(&tail);
    let total = out.len() / 2;
    let theory = 48_000f64 * ratio + delay;
    assert!(
        (total as f64 - theory).abs() <= 2.0,
        "flush 后总帧数应≈{theory:.1}（process {before} + flush {} = {total}）",
        tail.len() / 2
    );
    assert!(!tail.is_empty(), "冷启动应有滤波器尾部待冲刷");
    assert!(out.iter().all(|s| s.is_finite()));
    assert!(r.flush().is_empty(), "flush 应幂等（终态）");
}

/// REQ-26 停止冲刷：中途接入（已补偿延迟）后总帧数 = 直通段 + 变比段理论值。
#[test]
fn flush_after_midstream_engage_matches_mixed_rate_theory() {
    let input = sine_stereo(4800, 440.0);
    let mut r = DriftResampler::new(2);
    let mut out = Vec::new();
    for chunk in input[..2400 * 2].chunks(480 * 2) {
        out.extend(r.process(chunk));
    }
    r.set_rate_ppm(500.0);
    for chunk in input[2400 * 2..].chunks(480 * 2) {
        out.extend(r.process(chunk));
    }
    let tail = r.flush();
    out.extend_from_slice(&tail);
    // 直通段 2400 帧（1:1）+ 变比段 2400 帧 × ratio。
    let theory = 2400.0 + 2400.0 / 1.0005;
    assert!(
        ((out.len() / 2) as f64 - theory).abs() <= 2.0,
        "中途接入 flush 后总帧数应≈{theory:.1}（得到 {}）",
        out.len() / 2
    );
    assert!(!tail.is_empty(), "中途接入也应有尾部待冲刷");
    assert!(out.iter().all(|s| s.is_finite()));
}

/// REQ-26 停止冲刷：未进入校正的直通路径补出保留的 1 帧，样本仍逐帧恒等。
#[test]
fn flush_passthrough_emits_held_tail_frame() {
    let input = sine_stereo(100, 440.0);
    let mut r = DriftResampler::new(2);
    let mut out = r.process(&input);
    assert_eq!(out.len() / 2, 99, "直通 process 仍保留 1 帧（既有契约）");
    out.extend(r.flush());
    assert_eq!(out.len() / 2, 100, "停止冲刷应补出保留帧");
    for (i, (a, b)) in out.iter().zip(input.iter()).enumerate() {
        assert!((a - b).abs() < 1e-6, "样本 {i} 应逐帧恒等（{a} vs {b}）");
    }
    assert!(r.flush().is_empty(), "flush 应幂等（终态）");
}
