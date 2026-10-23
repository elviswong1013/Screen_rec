//! sr-core：全局类型与契约。
//!
//! 职责（specs/001-screen-recorder.md §3.2）：TargetRef/RouteConfig/SessionManifest/Settings/错误枚举/状态枚举。
//! M1-C 绿阶段：error（§3.4 错误码）与 events（REQ-31 Channel 语义）已实现；
//! types/state 仍为占位，由后续里程碑实现。

/// 核心数据模型（§3.4）。
pub mod types {
    // TODO(M1+): TargetRef / RouteConfig / VideoCfg / AudioPlan / AudioSourceSpec /
    // SessionManifest v1（原子写）/ Settings（REQ-24、REQ-30）。
}

pub mod error;
pub mod events;

/// 边界层日志/错误脱敏原语（M19c REQ-1，specs/005-log-redaction.md）。
pub mod redact;

/// 运行时工具定位（M11-C P1-5：ffmpeg/ffprobe 安装后解析，不依赖开发机源码目录）。
pub mod tools;

/// 状态枚举与路由/会话状态机（route_state 事件载荷）。
pub mod state {
    // TODO(M1+): 启动协议状态（采集就绪/管道建立/编码器就绪/输出开始，§3.1）与
    // 停止/收尾/恢复状态（REQ-21/22、§9 T5）。
}
