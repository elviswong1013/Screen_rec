//! 命名管道视频写（specs/001-screen-recorder.md §3.3 命名管道协议；S1/S1b 冻结项）。
//!
//! - Rust = 服务端（`CreateNamedPipeW`，字节模式、单实例、`PIPE_REJECT_REMOTE_CLIENTS`、
//!   in/out 缓冲 **8MiB**），sidecar（ffmpeg）为客户端；
//! - `ConnectNamedPipe` 无内建超时 → **外置超时**（≥5s）：watcher 线程 `CancelIoEx` 解除
//!   （S1b 实验 3：995=自主取消；535=客户端抢先连接，正常成功）；
//! - 数据面 = **阻塞式整帧 WriteFile**（即背压：慢读零丢帧、内存有界，S1b 实验 1）；
//! - 错误映射（S1b §7 实测）：`232 ERROR_NO_DATA` / `109 ERROR_BROKEN_PIPE` → 对端退出
//!   （产品 E_SIDECAR_EXIT 语义）；`995 ERROR_OPERATION_ABORTED` → 自主取消；
//! - EOF（关闭写句柄）是唯一优雅停止路径（S1b 实验 5a：ffmpeg ~100ms 封尾、exit 0）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, ERROR_INVALID_HANDLE, ERROR_NO_DATA, ERROR_OPERATION_ABORTED,
    ERROR_PIPE_CONNECTED, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    FlushFileBuffers, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_OUTBOUND,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::IO::CancelIoEx;

/// 视频管道缓冲（S1/S1b 定稿 8MiB；16MiB 仅小幅削减冷启动阻塞，稳态无差异）。
pub const VIDEO_PIPE_BUFFER_BYTES: u32 = 8 * 1024 * 1024;
/// 音频管道缓冲（§3.3「音频管道另设小缓冲」；f32le/48k/2ch=384KB/s，256KiB≈0.68s）。
pub const AUDIO_PIPE_BUFFER_BYTES: u32 = 256 * 1024;
/// 管道写超时（S1b 冻结下限 3s，取 5s：仅覆盖「活但不读」；对端死亡由内核即时
/// 解除 232/109，不受此值影响）。M4 背压用例的 2s 停滞必须落在超时窗内不触发。
pub const PIPE_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// 管道选项（缓冲大小 + 写超时）。
#[derive(Clone, Copy, Debug)]
pub struct PipeOptions {
    pub buffer_bytes: u32,
    /// None = 无写超时（永久阻塞背压）；Some(≥3s) = 「活但不读」超限后
    /// CancelIoEx 解除（995 → `PipeWriteError::Timeout`）。
    pub write_timeout: Option<Duration>,
}

impl Default for PipeOptions {
    fn default() -> Self {
        Self {
            buffer_bytes: VIDEO_PIPE_BUFFER_BYTES,
            write_timeout: Some(PIPE_WRITE_TIMEOUT),
        }
    }
}

/// 写错误（按 S1b 实测错误码分类，供产品层映射）。
#[derive(Debug)]
pub enum PipeWriteError {
    /// 对端已退出/断开（232/109）→ E_SIDECAR_EXIT 语义。
    PeerGone { os_code: u32 },
    /// 自主取消（995，CancelIoEx 解除的阻塞写；无超时配置时即本语义）。
    Cancelled,
    /// 「活但不读」写超时（995 + 停滞标记；S1b §7.3 冻结路径）。
    Timeout,
    /// 其它 I/O 错误（含句柄已关闭）。
    Io { detail: String },
}

/// 连接错误。
#[derive(Debug)]
pub enum PipeConnectError {
    /// 外置超时到点（CancelIoEx 解除，995）。
    Timeout,
    /// 建管/连接的其它失败。
    Failed { detail: String },
}

/// 内核句柄的可跨线程包装（句柄值本身线程无关；访问均经协议顺序/互斥约束）。
#[derive(Clone, Copy)]
struct SendHandle(HANDLE);
unsafe impl Send for SendHandle {}
unsafe impl Sync for SendHandle {}

impl SendHandle {
    /// CancelIoEx 解除该句柄上的阻塞 I/O（连接超时路径）。
    fn cancel_io(self) {
        unsafe {
            let _ = CancelIoEx(self.0, None);
        }
    }
}

struct PipeInner {
    name: String,
    handle: Mutex<Option<SendHandle>>,
    connected: AtomicBool,
    closed: AtomicBool,
    /// 当前阻塞写起始时刻（停滞 watcher 判定「活但不读」用；None=无在途写）。
    writing_since: Mutex<Option<Instant>>,
    /// 停滞超时触发的 CancelIoEx 标记（区分 995=停滞超时 vs 其它自主取消）。
    stalled: AtomicBool,
    /// 写停滞 watcher 线程（连接成功后启动；close 时回收）。
    watcher: Mutex<Option<std::thread::JoinHandle<()>>>,
}

/// 视频命名管道（服务端；可克隆共享，写与 EOF 关闭按协议顺序单线程发生）。
#[derive(Clone)]
pub struct VideoPipe {
    inner: Arc<PipeInner>,
    opts: PipeOptions,
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

impl VideoPipe {
    /// 创建命名管道 server（先于 sidecar spawn；§3.3 启动协议第一步；默认选项）。
    pub fn create(name: &str) -> Result<Self, PipeConnectError> {
        Self::create_with(name, PipeOptions::default())
    }

    /// 创建命名管道 server（自定义缓冲/写超时：视频 8MiB、音频小缓冲）。
    pub fn create_with(name: &str, opts: PipeOptions) -> Result<Self, PipeConnectError> {
        let name_wide = to_wide(name);
        unsafe {
            // FILE_FLAG_FIRST_PIPE_INSTANCE：同名冲突显式失败（不静默接管他人管道）。
            let h = CreateNamedPipeW(
                PCWSTR(name_wide.as_ptr()),
                PIPE_ACCESS_OUTBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                opts.buffer_bytes,
                opts.buffer_bytes,
                0,
                None,
            );
            let handle = if h.is_invalid() {
                return Err(PipeConnectError::Failed {
                    detail: format!(
                        "CreateNamedPipeW({name}) 失败: {}",
                        std::io::Error::last_os_error()
                    ),
                });
            } else {
                SendHandle(h)
            };
            Ok(Self {
                inner: Arc::new(PipeInner {
                    name: name.to_string(),
                    handle: Mutex::new(Some(handle)),
                    connected: AtomicBool::new(false),
                    closed: AtomicBool::new(false),
                    writing_since: Mutex::new(None),
                    stalled: AtomicBool::new(false),
                    watcher: Mutex::new(None),
                }),
                opts,
            })
        }
    }

    pub fn name(&self) -> &str {
        &self.inner.name
    }

    pub fn is_connected(&self) -> bool {
        self.inner.connected.load(Ordering::Acquire)
    }

    /// 等待 sidecar 连接（阻塞 `ConnectNamedPipe` + 外置超时 watcher `CancelIoEx`；
    /// S1b：真实客户端连接 15–29ms，超时预算 ≥5s）。
    pub fn connect_wait(&self, timeout: Duration) -> Result<(), PipeConnectError> {
        let handle = self
            .inner
            .handle
            .lock()
            .expect("管道句柄锁")
            .ok_or_else(|| PipeConnectError::Failed {
                detail: "管道已关闭".into(),
            })?
            .0;
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(PipeConnectError::Failed {
                detail: "管道已关闭".into(),
            });
        }

        // watcher：到点仍未连接 → CancelIoEx 解除阻塞连接（995）。
        let watcher_connected = Arc::new(AtomicBool::new(false));
        let watcher_handle = SendHandle(handle);
        let watcher_flag = Arc::new(AtomicBool::new(false));
        let watcher = {
            let connected_flag = watcher_connected.clone();
            let stop_flag = watcher_flag.clone();
            std::thread::Builder::new()
                .name("sr-pipe-connect-watch".into())
                .spawn(move || {
                    let deadline = std::time::Instant::now() + timeout;
                    while std::time::Instant::now() < deadline {
                        if connected_flag.load(Ordering::Acquire)
                            || stop_flag.load(Ordering::Acquire)
                        {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    if !connected_flag.load(Ordering::Acquire) {
                        watcher_handle.cancel_io();
                    }
                })
                .map_err(|e| PipeConnectError::Failed {
                    detail: format!("启动连接 watcher 失败: {e}"),
                })?
        };

        let result = unsafe { ConnectNamedPipe(handle, None) };
        watcher_connected.store(true, Ordering::Release);
        watcher_flag.store(true, Ordering::Release);
        let _ = watcher.join();

        match result {
            Ok(()) => {}
            // 535：客户端在 ConnectNamedPipe 之前已连接（正常成功，S1b §7.4）。
            Err(e) if e.code() == hresult_from(ERROR_PIPE_CONNECTED) => {}
            Err(e) if e.code() == hresult_from(ERROR_OPERATION_ABORTED) => {
                // 超时取消：监听实例可原位复用（S1b 实验 3），交由调用方决策。
                unsafe {
                    let _ = DisconnectNamedPipe(handle);
                }
                return Err(PipeConnectError::Timeout);
            }
            Err(e) => {
                return Err(PipeConnectError::Failed {
                    detail: format!("ConnectNamedPipe: {e}"),
                });
            }
        }
        self.inner.connected.store(true, Ordering::Release);
        self.start_stall_watcher();
        Ok(())
    }

    /// 停滞 watcher（「活但不读」写超时；S1b §7.3：对端死亡无需超时——内核即时
    /// 解除 232/109，本 watcher 仅覆盖阻塞写超限路径）。
    fn start_stall_watcher(&self) {
        let Some(timeout) = self.opts.write_timeout else {
            return;
        };
        let watcher_inner = self.inner.clone();
        let handle = std::thread::Builder::new()
            .name("sr-pipe-stall-watch".into())
            .spawn(move || {
                let inner = watcher_inner;
                loop {
                    if inner.closed.load(Ordering::Acquire) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                    if inner.closed.load(Ordering::Acquire) {
                        return;
                    }
                    let since = *inner.writing_since.lock().expect("写起始锁");
                    if let Some(t0) = since {
                        if t0.elapsed() >= timeout {
                            inner.stalled.store(true, Ordering::Release);
                            let h = inner.handle.lock().expect("管道句柄锁").map(|h| h.0);
                            if let Some(h) = h {
                                unsafe {
                                    let _ = CancelIoEx(h, None);
                                }
                            }
                            return;
                        }
                    }
                }
            })
            .ok();
        *self.inner.watcher.lock().expect("watcher 锁") = handle;
    }

    /// 阻塞式整帧写（=背压；调用方为帧回调线程）。
    pub fn write_all(&self, data: &[u8]) -> Result<(), PipeWriteError> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(PipeWriteError::Io {
                detail: "管道已关闭（EOF 已发出）".into(),
            });
        }
        let handle = self
            .inner
            .handle
            .lock()
            .expect("管道句柄锁")
            .ok_or_else(|| PipeWriteError::Io {
                detail: "管道句柄已关闭".into(),
            })?
            .0;
        *self.inner.writing_since.lock().expect("写起始锁") = Some(Instant::now());
        let mut off = 0usize;
        let result = (|| {
            while off < data.len() {
                let mut written: u32 = 0;
                let res =
                    unsafe { WriteFile(handle, Some(&data[off..]), Some(&mut written), None) };
                if res.is_err() || written == 0 {
                    return Err(classify_write_error(
                        res.err(),
                        self.inner.stalled.load(Ordering::Acquire),
                    ));
                }
                off += written as usize;
            }
            Ok(())
        })();
        *self.inner.writing_since.lock().expect("写起始锁") = None;
        result
    }

    /// EOF：关闭写句柄（唯一优雅停止路径；须在帧回调线程结束后调用，S1b 实验 5a）。
    pub fn close(&self) {
        if self.inner.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let handle = self
            .inner
            .handle
            .lock()
            .expect("管道句柄锁")
            .take()
            .map(|h| h.0);
        if let Some(handle) = handle {
            unsafe {
                if self.inner.connected.load(Ordering::Acquire) {
                    let _ = FlushFileBuffers(handle);
                }
                let _ = CloseHandle(handle);
            }
        }
        // 回收停滞 watcher（closed 置位后自行退出）。
        let watcher = self.inner.watcher.lock().expect("watcher 锁").take();
        if let Some(w) = watcher {
            let _ = w.join();
        }
    }
}

fn hresult_from(code: windows::Win32::Foundation::WIN32_ERROR) -> windows::core::HRESULT {
    windows::core::HRESULT::from_win32(code.0)
}

/// 写错误分类（S1b §7.4：232 为主、109 兼容；995=取消/停滞超时，由停滞标记区分）。
fn classify_write_error(err: Option<windows::core::Error>, stalled: bool) -> PipeWriteError {
    match err {
        Some(e) => {
            let code = e.code();
            if code == hresult_from(ERROR_NO_DATA) || code == hresult_from(ERROR_BROKEN_PIPE) {
                PipeWriteError::PeerGone {
                    os_code: if code == hresult_from(ERROR_NO_DATA) {
                        232
                    } else {
                        109
                    },
                }
            } else if code == hresult_from(ERROR_OPERATION_ABORTED) {
                if stalled {
                    PipeWriteError::Timeout
                } else {
                    PipeWriteError::Cancelled
                }
            } else if code == hresult_from(ERROR_INVALID_HANDLE) {
                PipeWriteError::Io {
                    detail: format!("管道句柄无效（已关闭）: {e}"),
                }
            } else {
                PipeWriteError::Io {
                    detail: format!("WriteFile: {e}"),
                }
            }
        }
        None => PipeWriteError::Io {
            detail: "WriteFile 写入 0 字节".into(),
        },
    }
}
