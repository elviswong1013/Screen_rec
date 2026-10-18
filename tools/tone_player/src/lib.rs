//! tone_player：L2/L3 测试助手（specs/001-screen-recorder.md §3.2/§4）。
//!
//! M1-A 骨架占位：仅模块声明，功能由后续里程碑实现。

/// 可区分音调播放（§4：A=440Hz、B=880Hz、麦克风=1200Hz）。
pub mod tone {
    // TODO(M2+): 音调播放，供 FFT 断言（it_audio_*）使用。
}

/// 闪光/节拍参考信号（§4：同步内容分析 manual_soak / it_sync_content_10min 用）。
pub mod flash {
    // TODO(M3+): 闪光/节拍输出。
}
