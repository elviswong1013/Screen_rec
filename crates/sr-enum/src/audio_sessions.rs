//! 活动音频会话枚举（REQ-4，specs/001-screen-recorder.md §2.1）。
//!
//! 经 wasapi 0.26 枚举默认渲染端点的活动音频会话（`SessionState::Active`），
//! 返回**值快照**：同一 `Vec` 不因后续音源增减而变化（「全选=启动时快照」的数据模型钩子）。
//! 同一 PID 的多会话归一为一条（同根去重，REQ-4）；身份（PID+创建时间+映像名）与
//! REQ-1 同源；身份不可得（进程已退出等）的会话不列出（REQ-14：不静默编造）。
//! 音源退出后其会话随进程消失（WASAPI 会话回收延迟由调用方轮询容忍，§4）。

use sr_core::error::Error;
use wasapi::{deinitialize, initialize_mta, DeviceEnumerator, Direction, SessionState};

/// 活动音频会话条目（REQ-4）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSessionEntry {
    /// 会话属主进程 PID。
    pub pid: u32,
    /// 属主进程创建时间（UNIX epoch 毫秒；与 REQ-1 身份同源）。
    pub process_created_at: u64,
    /// 属主进程映像名，如 "ffplay.exe"。
    pub process_name: String,
    /// 应用名（非空）。
    pub app_name: String,
}

/// 列出默认渲染端点上的活动音频会话（值快照）。
pub fn list_audio_sessions() -> Result<Vec<AudioSessionEntry>, Error> {
    // 本函数为叶子调用：COM(MTA) 成功初始化则成对释放；
    // 已初始化（S_FALSE）同样需要配平；RPC_E_CHANGED_MODE 表示他处已初始化，直接复用。
    let com_owned = initialize_mta().is_ok();
    let _com_guard = ComGuard { owned: com_owned };

    let map_err =
        |context: &'static str, e: wasapi::WasapiError| Error::Internal(format!("{context}: {e}"));

    let enumerator = DeviceEnumerator::new().map_err(|e| map_err("WASAPI DeviceEnumerator", e))?;
    let device = enumerator
        .get_default_device(&Direction::Render)
        .map_err(|e| map_err("WASAPI 默认渲染端点不可用", e))?;
    let manager = device
        .get_iaudiosessionmanager()
        .map_err(|e| map_err("WASAPI AudioSessionManager", e))?;
    let sessions = manager
        .get_audiosessionenumerator()
        .map_err(|e| map_err("WASAPI AudioSessionEnumerator", e))?;
    let count = sessions
        .get_count()
        .map_err(|e| map_err("WASAPI 会话计数", e))?;

    let mut out: Vec<AudioSessionEntry> = Vec::new();
    for i in 0..count {
        let Ok(control) = sessions.get_session(i) else {
            continue; // 会话恰在回收（竞态），跳过
        };
        let Ok(state) = control.get_state() else {
            continue;
        };
        if state != SessionState::Active {
            continue; // 仅活动会话（REQ-4：活动音频会话）
        }
        let Ok(pid) = control.get_process_id() else {
            continue;
        };
        if pid == 0 {
            continue; // 系统级会话无进程身份可归属
        }
        // 同根去重（REQ-4：同应用多会话归一为一条）。
        if out.iter().any(|entry| entry.pid == pid) {
            continue;
        }
        let Some(identity) = crate::identity::process_identity(pid) else {
            continue; // 进程已退出/身份不可得 → 不列出（不静默编造）
        };
        out.push(AudioSessionEntry {
            pid,
            process_created_at: identity.created_unix_ms,
            process_name: identity.process_name,
            app_name: identity.app_name,
        });
    }
    Ok(out)
}

/// COM 初始化配平守卫：仅在本函数成功 CoInitializeEx 时于作用域结束调用 CoUninitialize。
struct ComGuard {
    owned: bool,
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.owned {
            deinitialize();
        }
    }
}
