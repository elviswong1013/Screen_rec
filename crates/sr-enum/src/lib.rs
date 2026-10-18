//! sr-enum：窗口/显示器/音频会话枚举 + 应用身份解析（有界规则）。
//!
//! 职责（specs/001-screen-recorder.md §3.2）：REQ-1..4、REQ-14。
//! M1-C 绿阶段：REQ-1..4 枚举 API 已实现（L2 见 sr-itests enum_targets/enum_audio）；
//! REQ-14 完整有界规则（后代树/有界提升/宿主禁入）属 M4+，
//! 当前 identity 仅提供各枚举共用的最小进程身份底座。
//!
//! 注意：本 crate 存在顶层模块 `windows`，引用外部 `windows` crate 一律用 `::windows::` 全限定路径。

pub mod audio_sessions;
pub mod identity;
pub mod monitors;
pub mod render_endpoints;
pub mod windows;
