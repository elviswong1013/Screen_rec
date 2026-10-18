//! REQ-30 设置持久化（specs/001-screen-recorder.md §2.8 REQ-30、§3.3 commands、§3.4 Settings）。
//!
//! 存储选型（M1-D 决策）：**自管 JSON + 原子写**（写临时文件 → rename 覆盖），
//! 不引入 tauri-plugin-store——依赖面最小、写入原子性可控（与规格对 Manifest
//! 「临时文件+rename」的原子写约定同风格）。文件位置：
//! `app_local_data_dir()` = `%LOCALAPPDATA%\com.screenrec.desktop\settings.json`
//! （LocalAppData：设置含本机路径 output_dir，不宜随漫游配置同步）。
//!
//! 归属说明：§3.2 将 Settings 规划于 sr-core::types；本轮任务约束不改 crates/*，
//! 结构按 §3.4 形状先行落在本模块，后续里程碑迁移时保持 JSON 字段名与形状不变。

use std::path::Path;

use serde::{Deserialize, Serialize};

/// 设置 JSON 文件名。
const SETTINGS_FILE: &str = "settings.json";

/// 默认音频轨组织模式（REQ-17：隔离为默认）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioMode {
    /// 隔离（默认）：目标应用轨（+ 麦克风轨，若启用）。
    Isolated,
    /// 全音轨：目标应用 + 系统 + 麦克风 + 追加应用，各独立轨。
    AllTracks,
}

impl Default for AudioMode {
    fn default() -> Self {
        Self::Isolated
    }
}

/// 编码器设置（§3.4：`Auto|String`）。
///
/// 线格式（与 UI/`set_settings` DTO 一致）：`"auto"` 或 `"h264_mf"`（纯字符串）。
/// 反序列化同时兼容旧形态 `{"named":"h264_mf"}`（前向/向后兼容）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncoderSetting {
    /// 自动（启动前探测，按 §9 T1 行为）。
    Auto,
    /// 显式指定编码器名（如 "h264_mf"）。
    Named(String),
}

impl Default for EncoderSetting {
    fn default() -> Self {
        Self::Auto
    }
}

impl serde::Serialize for EncoderSetting {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Auto => serializer.serialize_str("auto"),
            Self::Named(name) => serializer.serialize_str(name),
        }
    }
}

impl<'de> serde::Deserialize<'de> for EncoderSetting {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{MapAccess, Visitor};
        use std::fmt;
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = EncoderSetting;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("\"auto\"、编码器名字符串或 {\"named\": \"...\"}")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(if v == "auto" {
                    EncoderSetting::Auto
                } else {
                    EncoderSetting::Named(v.to_string())
                })
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                self.visit_str(&v)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut named: Option<String> = None;
                while let Some(key) = map.next_key::<String>()? {
                    let value: String = map.next_value()?;
                    if key == "named" {
                        named = Some(value);
                    }
                }
                named
                    .map(EncoderSetting::Named)
                    .ok_or_else(|| serde::de::Error::missing_field("named"))
            }
        }
        deserializer.deserialize_any(V)
    }
}

/// 录制音频采样率四档（REQ-1：44.1k / 48k / 88.2k / 96k）。
pub const AUDIO_SAMPLE_RATES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];

/// 应用设置（§3.4 数据模型；字段默认值与规格一致）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// 输出目录（REQ-24：默认 `%USERPROFILE%\Videos\Screen_rec`）。
    #[serde(default = "default_output_dir")]
    pub output_dir: String,
    /// 帧率（默认 30）。
    #[serde(default = "default_fps")]
    pub fps: u32,
    /// 录制音频采样率（M15/REQ-1：默认 48000；字段级容错——坏类型只降级本字段）。
    #[serde(
        default = "default_sample_rate",
        deserialize_with = "deserialize_sample_rate"
    )]
    pub audio_sample_rate: u32,
    /// 成品容器（M17/REQ-16：默认 mkv；字段级容错——坏值只降级本字段）。
    #[serde(
        default = "default_output_container",
        serialize_with = "serialize_output_container",
        deserialize_with = "deserialize_output_container"
    )]
    pub output_container: sr_pipeline::naming::OutputContainer,
    /// 光标捕获开关（REQ-11：默认开）。
    #[serde(default = "default_cursor")]
    pub cursor: bool,
    /// 默认音轨模式（REQ-17：隔离为默认）。
    #[serde(default)]
    pub default_audio_mode: AudioMode,
    /// 麦克风设备（REQ-16：None=不启用）。
    #[serde(default)]
    pub mic_device: Option<String>,
    /// 编码器覆盖（§9 T1：Auto=探测，显式指定须 S2 矩阵验证过的名字）。
    #[serde(default)]
    pub encoder: EncoderSetting,
    /// 分段时长秒（REQ-21：默认 5）。
    #[serde(default = "default_segment_secs")]
    pub segment_secs: u32,
    /// M14：保留原始分段（调试/排查用）。默认 false = 成品验证成功后自动清理
    /// `<output_dir>/.screenrec_tmp/<session_id>`；旧配置缺字段按 false 兼容。
    #[serde(default)]
    pub keep_segments: bool,
}

fn default_output_dir() -> String {
    std::env::var("USERPROFILE")
        .map(|home| {
            Path::new(&home)
                .join("Videos")
                .join("Screen_rec")
                .to_string_lossy()
                .into_owned()
        })
        .unwrap_or_else(|_| "Videos\\Screen_rec".to_string())
}

fn default_fps() -> u32 {
    30
}

/// REQ-1：具名默认采样率（参照 `default_fps` 模式）。
fn default_sample_rate() -> u32 {
    48_000
}

/// REQ-16：具名默认成品容器（mkv；录制内部 MKV 分段行为不变）。
fn default_output_container() -> sr_pipeline::naming::OutputContainer {
    sr_pipeline::naming::OutputContainer::Mkv
}

fn serialize_output_container<S: serde::Serializer>(
    container: &sr_pipeline::naming::OutputContainer,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(container.as_str())
}

/// 字段级容错（REQ-16）：任意坏值只把本字段降级为 mkv。
fn deserialize_output_container<'de, D>(
    deserializer: D,
) -> Result<sr_pipeline::naming::OutputContainer, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
    use std::fmt;

    struct ContainerVisitor;
    impl<'de> Visitor<'de> for ContainerVisitor {
        type Value = sr_pipeline::naming::OutputContainer;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("\"mkv\" 或 \"mp4\"")
        }

        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
            Ok(sr_pipeline::naming::OutputContainer::parse(v)
                .unwrap_or(sr_pipeline::naming::OutputContainer::Mkv))
        }

        fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
            self.visit_str(&v)
        }

        fn visit_u64<E: serde::de::Error>(self, _v: u64) -> Result<Self::Value, E> {
            Ok(sr_pipeline::naming::OutputContainer::Mkv)
        }

        fn visit_i64<E: serde::de::Error>(self, _v: i64) -> Result<Self::Value, E> {
            Ok(sr_pipeline::naming::OutputContainer::Mkv)
        }

        fn visit_f64<E: serde::de::Error>(self, _v: f64) -> Result<Self::Value, E> {
            Ok(sr_pipeline::naming::OutputContainer::Mkv)
        }

        fn visit_bool<E: serde::de::Error>(self, _v: bool) -> Result<Self::Value, E> {
            Ok(sr_pipeline::naming::OutputContainer::Mkv)
        }

        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(sr_pipeline::naming::OutputContainer::Mkv)
        }

        fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(sr_pipeline::naming::OutputContainer::Mkv)
        }

        fn visit_some<D2: serde::Deserializer<'de>>(self, d: D2) -> Result<Self::Value, D2::Error> {
            d.deserialize_any(ContainerVisitor)
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
            Ok(sr_pipeline::naming::OutputContainer::Mkv)
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            Ok(sr_pipeline::naming::OutputContainer::Mkv)
        }
    }

    deserializer.deserialize_any(ContainerVisitor)
}

/// 档位归一化：四档之外的数值（含旧配置/编程构造的非法值）回落默认 48000。
pub fn normalize_sample_rate(rate: u32) -> u32 {
    if AUDIO_SAMPLE_RATES.contains(&rate) {
        rate
    } else {
        default_sample_rate()
    }
}

/// 字段级容错反序列化（REQ-1）：接受任意 JSON 值，只有合法四档 u32 保留；
/// 负数/字符串/null/溢出/浮点/对象/数组等坏类型只把**本字段**降级为默认，
/// 不影响同一份设置中的其它字段。
fn deserialize_sample_rate<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
    use std::fmt;

    struct SampleRateVisitor;
    impl<'de> Visitor<'de> for SampleRateVisitor {
        type Value = u32;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("四档采样率之一（44100/48000/88200/96000）")
        }

        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(u32::try_from(v)
                .map(normalize_sample_rate)
                .unwrap_or_else(|_| default_sample_rate()))
        }

        fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
            Ok(u32::try_from(v)
                .map(normalize_sample_rate)
                .unwrap_or_else(|_| default_sample_rate()))
        }

        fn visit_f64<E: serde::de::Error>(self, _v: f64) -> Result<Self::Value, E> {
            Ok(default_sample_rate())
        }

        fn visit_bool<E: serde::de::Error>(self, _v: bool) -> Result<Self::Value, E> {
            Ok(default_sample_rate())
        }

        fn visit_str<E: serde::de::Error>(self, _v: &str) -> Result<Self::Value, E> {
            Ok(default_sample_rate())
        }

        fn visit_string<E: serde::de::Error>(self, _v: String) -> Result<Self::Value, E> {
            Ok(default_sample_rate())
        }

        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(default_sample_rate())
        }

        fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(default_sample_rate())
        }

        fn visit_some<D2: serde::Deserializer<'de>>(self, d: D2) -> Result<Self::Value, D2::Error> {
            d.deserialize_any(SampleRateVisitor)
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
            Ok(default_sample_rate())
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            Ok(default_sample_rate())
        }
    }

    deserializer.deserialize_any(SampleRateVisitor)
}

fn default_cursor() -> bool {
    true
}

fn default_segment_secs() -> u32 {
    5
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            output_dir: default_output_dir(),
            fps: default_fps(),
            audio_sample_rate: default_sample_rate(),
            output_container: default_output_container(),
            cursor: default_cursor(),
            default_audio_mode: AudioMode::default(),
            mic_device: None,
            encoder: EncoderSetting::default(),
            segment_secs: default_segment_secs(),
            keep_segments: false,
        }
    }
}

impl Settings {
    /// REQ-1 归一化入口（磁盘加载与 `set_settings` 共用）：非法档位 → 48000。
    pub fn normalize(&mut self) {
        self.audio_sample_rate = normalize_sample_rate(self.audio_sample_rate);
    }

    /// 轻量校验（M1 最小：拒绝必然非法值；更严的取值域由后续里程碑按规格钉死）。
    pub fn validate(&self) -> Result<(), String> {
        if self.fps == 0 {
            return Err("fps 不得为 0".to_string());
        }
        if self.segment_secs == 0 {
            return Err("segment_secs 不得为 0".to_string());
        }
        if self.output_dir.trim().is_empty() {
            return Err("output_dir 不得为空".to_string());
        }
        Ok(())
    }
}

/// 读取设置；文件缺失或解析失败时落默认值并尝试写回（使存储位置立即可验证）。
/// 写回失败不阻塞启动（设置模块无致命路径；下次保存时会重试建目录）。
pub fn load_or_default(dir: &Path) -> Settings {
    let path = dir.join(SETTINGS_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<Settings>(&bytes) {
            Ok(mut s) => {
                // REQ-1：磁盘加载与 set_settings 复用同一归一化（非法档位 → 48000）。
                s.normalize();
                s
            }
            Err(e) => {
                eprintln!("[settings] {path:?} 解析失败，回退默认值: {e}");
                let s = Settings::default();
                let _ = save(dir, &s);
                s
            }
        },
        Err(_) => {
            let s = Settings::default();
            let _ = save(dir, &s);
            s
        }
    }
}

/// 原子写：先写同目录临时文件，再 rename 覆盖（Windows 下 std::fs::rename 使用
/// MOVEFILE_REPLACE_EXISTING，可覆盖既有目标；同卷保证原子性）。
pub fn save(dir: &Path, settings: &Settings) -> std::io::Result<()> {
    settings
        .validate()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{SETTINGS_FILE}.tmp"));
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, dir.join(SETTINGS_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("sr_m1d_settings_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn defaults_match_spec_section_3_4() {
        let s = Settings::default();
        assert_eq!(s.fps, 30, "§3.4：fps 默认 30");
        assert!(s.cursor, "§3.4：cursor 默认 true（REQ-11 默认开）");
        assert_eq!(
            s.default_audio_mode,
            AudioMode::Isolated,
            "REQ-17：隔离为默认"
        );
        assert_eq!(s.mic_device, None, "§3.4：mic_device 为 Option");
        assert_eq!(s.encoder, EncoderSetting::Auto, "§3.4：encoder 默认 Auto");
        assert_eq!(s.segment_secs, 5, "§3.4：segment_secs 默认 5");
        assert!(
            !s.keep_segments,
            "M14：keep_segments 默认 false（成功后自动清理原段）"
        );
        assert!(
            s.output_dir.ends_with("Screen_rec"),
            "REQ-24：默认输出目录以 Screen_rec 结尾（得到 {:?}）",
            s.output_dir
        );
        assert!(s.validate().is_ok());
    }

    #[test]
    fn serde_full_round_trip() {
        let s = Settings {
            output_dir: r"D:\rec".to_string(),
            fps: 60,
            cursor: false,
            default_audio_mode: AudioMode::AllTracks,
            mic_device: Some("Mic Array".to_string()),
            encoder: EncoderSetting::Named("h264_mf".to_string()),
            segment_secs: 10,
            keep_segments: true,
            audio_sample_rate: 88_200,
            output_container: sr_pipeline::naming::OutputContainer::Mp4,
        };
        let json = serde_json::to_string(&s).expect("serialize");
        let back: Settings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, s);
    }

    #[test]
    fn serde_partial_file_fills_defaults() {
        // 旧版/手写的不完整设置文件：缺失字段按 §3.4 默认值补齐（前向兼容）。
        let partial = r#"{"output_dir": "E:\\out", "fps": 60}"#;
        let s: Settings = serde_json::from_str(partial).expect("partial deserialize");
        assert_eq!(s.output_dir, r"E:\out");
        assert_eq!(s.fps, 60);
        assert!(s.cursor, "缺失字段应取默认 true");
        assert_eq!(s.segment_secs, 5, "缺失字段应取默认 5");
        assert!(
            !s.keep_segments,
            "M14：旧配置缺 keep_segments 时应取默认 false"
        );
        assert_eq!(s.default_audio_mode, AudioMode::Isolated);
        assert_eq!(s.encoder, EncoderSetting::Auto);
        assert_eq!(s.mic_device, None);
    }

    #[test]
    fn validate_rejects_invalid() {
        let mut s = Settings::default();
        s.fps = 0;
        assert!(s.validate().is_err(), "fps=0 应被拒绝");
        let mut s = Settings::default();
        s.segment_secs = 0;
        assert!(s.validate().is_err(), "segment_secs=0 应被拒绝");
        let mut s = Settings::default();
        s.output_dir = "  ".to_string();
        assert!(s.validate().is_err(), "空白 output_dir 应被拒绝");
    }

    #[test]
    fn encoder_setting_accepts_ui_string_and_legacy_tagged() {
        // UI 发送纯字符串（"auto" / "h264_mf"）；旧形态 {"named":"..."} 仍可读。
        let s: Settings = serde_json::from_str(r#"{"encoder":"h264_mf"}"#).expect("UI 字符串");
        assert_eq!(s.encoder, EncoderSetting::Named("h264_mf".into()));
        let s: Settings =
            serde_json::from_str(r#"{"encoder":{"named":"h264_qsv"}}"#).expect("旧形态");
        assert_eq!(s.encoder, EncoderSetting::Named("h264_qsv".into()));
        let s: Settings = serde_json::from_str(r#"{"encoder":"auto"}"#).expect("auto");
        assert_eq!(s.encoder, EncoderSetting::Auto);
        assert_eq!(
            serde_json::to_string(&EncoderSetting::Named("h264_mf".into())).expect("serialize"),
            r#""h264_mf""#
        );
        assert_eq!(
            serde_json::to_string(&EncoderSetting::Auto).expect("serialize"),
            r#""auto""#
        );
    }

    #[test]
    fn save_and_load_round_trip_atomic() {
        let dir = temp_dir("roundtrip");
        let s = Settings {
            fps: 60,
            keep_segments: true,
            ..Settings::default()
        };
        save(&dir, &s).expect("save");
        // rename 后不留临时文件（原子写语义）。
        assert!(
            !dir.join("settings.json.tmp").exists(),
            "临时文件应已 rename 消费"
        );
        let loaded = load_or_default(&dir);
        assert_eq!(loaded, s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_file_writes_default() {
        let dir = temp_dir("missing");
        let s = load_or_default(&dir);
        assert_eq!(s, Settings::default(), "缺失文件应返回默认值");
        assert!(dir.join("settings.json").exists(), "默认值应写回存储位置");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_corrupt_file_falls_back_to_default() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).expect("create dir");
        std::fs::write(dir.join("settings.json"), b"{ not json").expect("write corrupt");
        let s = load_or_default(&dir);
        assert_eq!(s, Settings::default(), "损坏文件应回退默认值");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─────────────────── M15/REQ-1：audio_sample_rate（四档 + 字段级容错） ───────────────────

    #[test]
    fn unit_settings_sample_rate_defaults_and_four_rate_round_trip() {
        assert_eq!(
            Settings::default().audio_sample_rate,
            48_000,
            "REQ-1：默认 48000"
        );
        for rate in [44_100u32, 48_000, 88_200, 96_000] {
            let mut s = Settings::default();
            s.audio_sample_rate = rate;
            let json = serde_json::to_string(&s).expect("serialize");
            let back: Settings = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back.audio_sample_rate, rate, "REQ-1：四档 {rate} 应可往返");
        }
    }

    #[test]
    fn unit_settings_sample_rate_missing_field_keeps_other_fields() {
        let json = r#"{"output_dir":"E:\\out","fps":60,"audio_sample_rate":88200}"#;
        let full: Settings = serde_json::from_str(json).expect("deserialize");
        assert_eq!(full.audio_sample_rate, 88_200, "显式值应保留");
        assert_eq!(full.fps, 60);

        let no_rate = r#"{"output_dir":"E:\\out","fps":60}"#;
        let s: Settings = serde_json::from_str(no_rate).expect("缺字段应可解析");
        assert_eq!(s.audio_sample_rate, 48_000, "REQ-1：缺字段 → 默认 48000");
        assert_eq!(s.output_dir, r"E:\out", "其余字段不得被重置");
        assert_eq!(s.fps, 60, "其余字段不得被重置");
    }

    #[test]
    fn unit_settings_sample_rate_bad_types_degrade_field_only() {
        // 字段级容错：负数字符串/null/溢出/非法档位/浮点只降级该字段，其余字段保留。
        let bad_values = [
            ("negative", r#"-48000"#),
            ("string", r#""96000""#),
            ("null", "null"),
            ("overflow_u64", r#"18446744073709551616"#),
            ("overflow_f64", r#"1e300"#),
            ("unknown_rate", r#"12345"#),
            ("float", r#"48000.5"#),
            ("object", r#"{"rate":96000}"#),
            ("array", r#"[96000]"#),
            ("bool", "true"),
        ];
        for (label, bad) in bad_values {
            let json = format!(r#"{{"output_dir":"D:\\kept","fps":60,"audio_sample_rate":{bad}}}"#);
            let s: Settings = serde_json::from_str(&json).unwrap_or_else(|e| {
                panic!("REQ-1：坏类型 {label} 不得导致整份设置解析失败（得到 {e}）")
            });
            assert_eq!(
                s.audio_sample_rate, 48_000,
                "REQ-1：{label} 应只把本字段降级为默认 48000"
            );
            assert_eq!(s.output_dir, r"D:\kept", "REQ-1：{label} 不得重置其余字段");
            assert_eq!(s.fps, 60, "REQ-1：{label} 不得重置其余字段");
        }
    }

    #[test]
    fn unit_settings_sample_rate_disk_load_normalizes() {
        let dir = temp_dir("rate-disk");
        std::fs::create_dir_all(&dir).expect("create dir");

        std::fs::write(
            dir.join("settings.json"),
            br#"{"output_dir":"D:\\out","fps":60,"audio_sample_rate":44100}"#,
        )
        .expect("write");
        let s = load_or_default(&dir);
        assert_eq!(s.audio_sample_rate, 44_100, "磁盘入口应保留合法档位");
        assert_eq!(s.fps, 60);

        std::fs::write(
            dir.join("settings.json"),
            br#"{"output_dir":"D:\\out","fps":60,"audio_sample_rate":12345}"#,
        )
        .expect("write");
        let s = load_or_default(&dir);
        assert_eq!(
            s.audio_sample_rate, 48_000,
            "磁盘入口应归一化非法档位（与 set_settings 同函数）"
        );
        assert_eq!(s.fps, 60, "归一化不得重置其余字段");

        std::fs::write(
            dir.join("settings.json"),
            br#"{"output_dir":"D:\\out","fps":60,"audio_sample_rate":"96000"}"#,
        )
        .expect("write");
        let s = load_or_default(&dir);
        assert_eq!(s.audio_sample_rate, 48_000, "磁盘坏类型 → 默认");
        assert_eq!(s.fps, 60, "磁盘坏类型不得重置其余字段");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unit_settings_sample_rate_normalize_is_idempotent() {
        let mut s = Settings::default();
        s.audio_sample_rate = 12_345;
        s.normalize();
        assert_eq!(s.audio_sample_rate, 48_000);
        s.normalize();
        assert_eq!(s.audio_sample_rate, 48_000, "归一化应幂等");
        for rate in [44_100u32, 48_000, 88_200, 96_000] {
            let mut s = Settings::default();
            s.audio_sample_rate = rate;
            s.normalize();
            assert_eq!(s.audio_sample_rate, rate, "合法档位不应被改写");
        }
    }

    // ─────────────────── M17/REQ-16：output_container（默认/容错/往返） ───────────────────

    #[test]
    fn unit_settings_output_container_default_and_round_trip() {
        use sr_pipeline::naming::OutputContainer;
        assert_eq!(
            Settings::default().output_container,
            OutputContainer::Mkv,
            "REQ-16：默认成品容器为 mkv"
        );
        for container in [OutputContainer::Mkv, OutputContainer::Mp4] {
            let mut s = Settings::default();
            s.output_container = container;
            let json = serde_json::to_string(&s).expect("serialize");
            let back: Settings = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(
                back.output_container, container,
                "REQ-16：{container:?} 应可往返持久化"
            );
        }
        let s: Settings = serde_json::from_str(r#"{"output_container":"mp4"}"#).expect("mp4");
        assert_eq!(s.output_container, OutputContainer::Mp4);
        let s: Settings = serde_json::from_str(r#"{"output_container":"mkv"}"#).expect("mkv");
        assert_eq!(s.output_container, OutputContainer::Mkv);
    }

    #[test]
    fn unit_settings_output_container_missing_field_keeps_other_fields() {
        use sr_pipeline::naming::OutputContainer;
        let json = r#"{"output_dir":"E:\\out","fps":60,"output_container":"mp4"}"#;
        let s: Settings = serde_json::from_str(json).expect("deserialize");
        assert_eq!(s.output_container, OutputContainer::Mp4);
        assert_eq!(s.output_dir, r"E:\out");
        assert_eq!(s.fps, 60);

        let s: Settings =
            serde_json::from_str(r#"{"output_dir":"E:\\out","fps":60}"#).expect("缺字段");
        assert_eq!(
            s.output_container,
            OutputContainer::Mkv,
            "REQ-16：缺字段 → mkv"
        );
        assert_eq!(s.output_dir, r"E:\out", "其余字段不得被重置");
        assert_eq!(s.fps, 60, "其余字段不得被重置");
    }

    #[test]
    fn unit_settings_output_container_bad_values_degrade_field_only() {
        use sr_pipeline::naming::OutputContainer;
        // 字段级容错：坏类型/未知字符串只降级本字段，其余字段保留（同 audio_sample_rate 模式）。
        let bad_values = [
            ("number", "1"),
            ("bool", "true"),
            ("null", "null"),
            ("object", r#"{"container":"mp4"}"#),
            ("array", r#"["mp4"]"#),
            ("unknown_string", r#""mov""#),
            ("empty_string", r#""""#),
            ("float", "1.5"),
        ];
        for (label, bad) in bad_values {
            let json = format!(r#"{{"output_dir":"D:\\kept","fps":60,"output_container":{bad}}}"#);
            let s: Settings = serde_json::from_str(&json).unwrap_or_else(|e| {
                panic!("REQ-16：坏值 {label} 不得导致整份设置解析失败（得到 {e}）")
            });
            assert_eq!(
                s.output_container,
                OutputContainer::Mkv,
                "REQ-16：{label} 应只把本字段降级为默认 mkv"
            );
            assert_eq!(s.output_dir, r"D:\kept", "REQ-16：{label} 不得重置其余字段");
            assert_eq!(s.fps, 60, "REQ-16：{label} 不得重置其余字段");
        }
    }

    #[test]
    fn unit_settings_output_container_disk_load_tolerates_bad_value() {
        use sr_pipeline::naming::OutputContainer;
        let dir = temp_dir("container-disk");
        std::fs::create_dir_all(&dir).expect("create dir");

        std::fs::write(
            dir.join("settings.json"),
            br#"{"output_dir":"D:\\out","fps":60,"output_container":"mp4"}"#,
        )
        .expect("write");
        let s = load_or_default(&dir);
        assert_eq!(s.output_container, OutputContainer::Mp4);
        assert_eq!(s.fps, 60);

        std::fs::write(
            dir.join("settings.json"),
            br#"{"output_dir":"D:\\out","fps":60,"output_container":123}"#,
        )
        .expect("write");
        let s = load_or_default(&dir);
        assert_eq!(
            s.output_container,
            OutputContainer::Mkv,
            "磁盘坏值应只降级本字段"
        );
        assert_eq!(s.output_dir, r"D:\out", "磁盘坏值不得重置其余字段");
        assert_eq!(s.fps, 60, "磁盘坏值不得重置其余字段");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
