//! 统一错误枚举（specs/001-screen-recorder.md §3.4）。
//!
//! 变体与规格 §3.4 的 E_* 错误码一一对应，`code()` 返回规格错误码字符串。
//! 当前消费方（L1/L2 测试）仅要求 `Debug`（经 `.expect` 触发）；后续任务收紧断言时以
//! `code()` 为契约锚点。`Internal` 为实现期 OS/环境调用失败的兜底（非规格错误码），
//! 不得作为跨模块契约码外溢使用。

use std::fmt;

/// 规格错误枚举（§3.4）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// E_TARGET_LOST：目标窗口/进程丢失（窗口关闭、PID 消失等）。
    TargetLost,
    /// E_LOOPBACK_UNAVAILABLE：进程环回不可用/激活被拒。
    LoopbackUnavailable,
    /// E_CONSENT_DENIED：权限/用户同意被拒（麦克风隐私、WGC 等）。
    ConsentDenied,
    /// E_ENCODER_UNAVAILABLE：无可用编码器（启动前探测失败，§9 T1）。
    EncoderUnavailable,
    /// E_PIPE_STALL：命名管道阻塞/背压超限。
    PipeStall,
    /// E_SIDECAR_EXIT：ffmpeg sidecar 非零退出/被终止。
    SidecarExit,
    /// E_DISK_FULL：磁盘空间不足（录制中/收尾中）。
    DiskFull,
    /// E_SEGMENT_CORRUPT：分段损坏（尾段不完整/校验失败）。
    SegmentCorrupt,
    /// E_RECOVERY_FAILED：恢复流程失败（保留原段）。
    RecoveryFailed,
    /// E_MERGE_INCOMPATIBLE：合并输入不可读/未被 MKV 接纳（异构布局不算，REQ-23）。
    MergeIncompatible,
    /// E_IDENTITY_AMBIGUOUS：身份解析不可靠（无法可靠归属，REQ-14）。
    IdentityAmbiguous,
    /// 实现期 OS/环境调用失败的内部兜底（非 §3.4 错误码）。
    Internal(String),
}

impl Error {
    /// 规格 §3.4 错误码字符串；`Internal` 无规格码，返回 "E_INTERNAL"。
    pub fn code(&self) -> &'static str {
        match self {
            Self::TargetLost => "E_TARGET_LOST",
            Self::LoopbackUnavailable => "E_LOOPBACK_UNAVAILABLE",
            Self::ConsentDenied => "E_CONSENT_DENIED",
            Self::EncoderUnavailable => "E_ENCODER_UNAVAILABLE",
            Self::PipeStall => "E_PIPE_STALL",
            Self::SidecarExit => "E_SIDECAR_EXIT",
            Self::DiskFull => "E_DISK_FULL",
            Self::SegmentCorrupt => "E_SEGMENT_CORRUPT",
            Self::RecoveryFailed => "E_RECOVERY_FAILED",
            Self::MergeIncompatible => "E_MERGE_INCOMPATIBLE",
            Self::IdentityAmbiguous => "E_IDENTITY_AMBIGUOUS",
            Self::Internal(_) => "E_INTERNAL",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Internal(detail) => write!(f, "{}: {detail}", self.code()),
            other => write!(f, "{}", other.code()),
        }
    }
}

impl std::error::Error for Error {}
