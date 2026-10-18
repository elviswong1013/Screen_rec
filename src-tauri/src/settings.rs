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

/// 应用设置（§3.4 数据模型；字段默认值与规格一致）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// 输出目录（REQ-24：默认 `%USERPROFILE%\Videos\Screen_rec`）。
    #[serde(default = "default_output_dir")]
    pub output_dir: String,
    /// 帧率（默认 30）。
    #[serde(default = "default_fps")]
    pub fps: u32,
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
            Ok(s) => s,
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
}
