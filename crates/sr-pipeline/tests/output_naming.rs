//! L1 单元测试：输出命名（REQ-24，specs/001-screen-recorder.md §2.5 / §3.4；D12）。
//!
//! 运行方式（L1：随默认 `cargo test` 执行）：
//! ```text
//! cargo test -p sr-pipeline --test output_naming -- --nocapture
//! ```
//!
//! 接口契约（测试即契约，详见 docs/milestones/m3-red-report.md）：
//! - `sr_pipeline::naming::sanitize_target_name(raw: &str) -> String`
//!   （非法字符 `\/:*?"<>|` 与控制字符替换、结尾点/空格修剪、保留名转义）
//! - `sr_pipeline::naming::format_timestamp(unix_ms: u64, utc_offset_minutes: i32) -> String`
//!   （`yyyyMMdd_HHmmss`；**显式偏移**注入——本地时区确定性与可测性，避免测试脆于宿主 TZ）
//! - `sr_pipeline::naming::next_available(dir: &Path, stem: &str, ext: &str) -> PathBuf`
//!   （碰撞加 `_1`/`_2` 序号后缀——测试假定的后缀形状；**纯分配器**：不创建文件、
//!   返回时目标不存在（禁止覆盖））

use std::fs;
use std::path::PathBuf;

use sr_pipeline::naming::{format_timestamp, next_available, sanitize_target_name};

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn is_reserved(s: &str) -> bool {
    RESERVED.iter().any(|r| s.eq_ignore_ascii_case(r))
}

fn temp_dir_unique(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m3_red_naming_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("创建临时目录");
    d
}

#[test]
fn unit_output_naming() {
    // ── 1. sanitize：安全字符原样保留（可辨识性） ──
    assert_eq!(
        sanitize_target_name("Monitor 1"),
        "Monitor 1",
        "安全名应原样保留"
    );
    assert_eq!(
        sanitize_target_name("记事本 - Notepad"),
        "记事本 - Notepad",
        "Unicode 安全名应原样保留"
    );
    assert_eq!(sanitize_target_name("x"), "x", "单字符安全名不变");

    // ── 2. sanitize：Windows 非法字符与控制字符不得残留 ──
    let dirty = "a/b\\c:d*e?f\"g<h>i|j\tk";
    let clean = sanitize_target_name(dirty);
    assert!(!clean.is_empty(), "sanitize 结果不应为空");
    for bad in ['\\', '/', ':', '*', '?', '"', '<', '>', '|', '\t', '\0'] {
        assert!(
            !clean.contains(bad),
            "非法字符 {bad:?} 不得残留在 {clean:?}"
        );
    }
    for keep in ['a', 'e', 'j', 'k'] {
        assert!(
            clean.contains(keep),
            "安全片段应保留（{keep} 不在 {clean:?}）"
        );
    }

    // ── 3. sanitize：结尾点/空格修剪（Windows 路径元素约束） ──
    let dotted = sanitize_target_name("name.");
    assert!(
        !dotted.ends_with('.') && !dotted.is_empty(),
        "结尾点应处理：{dotted:?}"
    );
    let spaced = sanitize_target_name("name ");
    assert!(
        !spaced.ends_with(' ') && !spaced.is_empty(),
        "结尾空格应处理：{spaced:?}"
    );

    // ── 4. sanitize：保留名转义 ──
    for rsv in ["CON", "con", "NUL", "Com1", "LPT9"] {
        let s = sanitize_target_name(rsv);
        assert!(!is_reserved(&s), "保留名 {rsv:?} 应被转义（得到 {s:?}）");
        assert!(!s.is_empty(), "转义结果不应为空");
    }

    // ── 5. format_timestamp：yyyyMMdd_HHmmss（显式偏移，确定性锚点） ──
    // 2025-01-01T00:00:00Z = 1_735_689_600_000 ms（2024 闰年 366 天之后，覆盖闰年路径）
    const T2025: u64 = 1_735_689_600_000;
    assert_eq!(format_timestamp(0, 0), "19700101_000000", "unix 0 @UTC");
    assert_eq!(format_timestamp(0, 480), "19700101_080000", "unix 0 @UTC+8");
    assert_eq!(format_timestamp(86_400_000, 0), "19700102_000000", "+1 天");
    assert_eq!(
        format_timestamp(T2025, 0),
        "20250101_000000",
        "2025-01-01 @UTC"
    );
    assert_eq!(
        format_timestamp(T2025, 330),
        "20250101_053000",
        "UTC+5:30（非整时偏移）"
    );
    assert_eq!(
        format_timestamp(T2025, -60),
        "20241231_230000",
        "UTC-1（跨日回退）"
    );
    for (ms, off) in [(0i64 * 1000, 0i32), (T2025 as i64, 480)] {
        let s = format_timestamp(ms as u64, off);
        assert_eq!(s.len(), 15, "形状 yyyyMMdd_HHmmss（得到 {s:?}）");
        let bytes = s.as_bytes();
        assert!(
            bytes[..8].iter().all(u8::is_ascii_digit)
                && bytes[8] == b'_'
                && bytes[9..].iter().all(u8::is_ascii_digit),
            "形状：8 数字 + 下划线 + 6 数字（{s:?}）"
        );
    }

    // ── 6. next_available：空目录返回原名；碰撞逐级加序号；永不覆盖 ──
    let dir = temp_dir_unique("collide");
    let first = next_available(&dir, "20261016_120000_SRTarget", "mkv");
    assert_eq!(
        first,
        dir.join("20261016_120000_SRTarget.mkv"),
        "无碰撞时应返回原名（纯分配器，不创建文件）"
    );
    assert!(!first.exists(), "next_available 不得自行创建文件");

    fs::write(&first, b"occupied").expect("占用首个名字");
    let second = next_available(&dir, "20261016_120000_SRTarget", "mkv");
    assert_eq!(
        second,
        dir.join("20261016_120000_SRTarget_1.mkv"),
        "同秒同目标冲突应加序号 _1（测试假定的后缀形状）"
    );
    assert!(!second.exists(), "分配结果在返回时不得已存在（禁止覆盖）");

    fs::write(&second, b"occupied").expect("占用 _1");
    let third = next_available(&dir, "20261016_120000_SRTarget", "mkv");
    assert_eq!(
        third,
        dir.join("20261016_120000_SRTarget_2.mkv"),
        "再次冲突应加序号 _2"
    );
    assert!(!third.exists(), "禁止覆盖（_2）");

    // 幂等性：目录状态不变时重复分配结果一致
    let third_again = next_available(&dir, "20261016_120000_SRTarget", "mkv");
    assert_eq!(third_again, third, "分配应是目录状态的纯函数");

    let _ = fs::remove_dir_all(&dir);
}
