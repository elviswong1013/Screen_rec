//! ffmpeg sidecar（REQ-20/D3/D4/D8）：编码命令构建（§3.3 基准骨架）、锁定构建定位、
//! D4 编码器链逐级实际初始化探测、sidecar spawn（进程级 Kill-on-close Job 绑定）+
//! stderr 持续排空；M8 增补：锁定 n8.1 构建的许可/校验和审计（REQ-29）。
//!
//! 基准命令（specs/001-screen-recorder.md §3.3；S5 修订：force_key_frames、0 字节段排除）：
//! `ffmpeg -hide_banner -nostdin -f rawvideo -pix_fmt nv12 -s {W}x{H} -framerate {fps}
//!  -i {vpipe} [-f f32le -ar {out_rate} -ac 2 -i {apipe}...] -map 0:v [-map k:a...]
//!  -c:v {encoder} -b:v 12M -g {2*fps} -force_key_frames expr:gte(t,n_forced*{segment_secs})
//!  [-c:a aac -b:a 192k] -f segment -segment_time {secs} -reset_timestamps 1
//!  -segment_format matroska -segment_list {list} [-segment_list_type ffconcat] {pattern}`

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;

/// 视频输入管道规格（rawvideo/nv12；固定画布尺寸，CFR 由帧槽节奏器保证）。
#[derive(Debug, Clone)]
pub struct VideoPipeSpec {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub pipe_path: String,
}

/// 音频输入管道规格（f32le/立体声；采样率 = 成品输出率，M15/REQ-3 参数化）。
#[derive(Debug, Clone)]
pub struct AudioPipeSpec {
    pub pipe_path: String,
    /// 输出率（`-ar`；44.1k/48k/88.2k/96k）。
    pub sample_rate: u32,
}

/// 构建 ffmpeg 编码 argv（不含 exe 本体；含 -hide_banner/-nostdin）。
/// 输出 pattern 为最后一个参数（S1/S1b 惯例）。
pub fn build_encode_args(
    video: &VideoPipeSpec,
    audio: &[AudioPipeSpec],
    video_encoder: &str,
    segment_secs: u32,
    segment_list: &Path,
    output_pattern: &Path,
) -> Vec<String> {
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-nostdin".into()];

    // 视频输入（NV12 连续帧 ← 命名管道）。
    args.extend([
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "nv12".into(),
        "-s".into(),
        format!("{}x{}", video.width, video.height),
        "-framerate".into(),
        video.fps.to_string(),
        "-i".into(),
        video.pipe_path.clone(),
    ]);

    // 音频输入（f32le/{输出率}/立体声；每输入管道独立；M15/REQ-3 `-ar` 参数化）。
    for a in audio {
        args.extend([
            "-f".into(),
            "f32le".into(),
            "-ar".into(),
            a.sample_rate.to_string(),
            "-ac".into(),
            "2".into(),
            "-i".into(),
            a.pipe_path.clone(),
        ]);
    }

    // 显式映射（视频 + 每路音频）。
    args.extend(["-map".into(), "0:v".into()]);
    for k in 0..audio.len() {
        args.extend(["-map".into(), format!("{}:a", k + 1)]);
    }

    // 编码器（D4 链探测结果透传）+ 显式钉码率 + 闭合 GOP（2×fps）+ 段首 IDR（S5 冻结）。
    // 码率必须显式钉死（S2 矩阵校准）：NVENC 默认 ~2Mbps 在 1080p 高运动内容下会
    // 间歇性压垮色度（M5 红报告 §8-2；scratch 以 12M 验证 8/8 轮消除）。
    args.extend(["-c:v".into(), video_encoder.into()]);
    args.extend(["-b:v".into(), "12M".into()]);
    args.extend(["-g".into(), (2 * video.fps).to_string()]);
    args.extend([
        "-force_key_frames".into(),
        format!("expr:gte(t,n_forced*{segment_secs})"),
    ]);

    // 音频编码（有音频时收缩）。
    if !audio.is_empty() {
        args.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()]);
    }

    // 5s MKV 分段（闭合 GOP + reset_timestamps + ffconcat 段清单）。
    args.extend([
        "-f".into(),
        "segment".into(),
        "-segment_time".into(),
        segment_secs.to_string(),
        "-reset_timestamps".into(),
        "1".into(),
        "-segment_format".into(),
        "matroska".into(),
        "-segment_list".into(),
        segment_list.to_string_lossy().to_string(),
        "-segment_list_type".into(),
        "ffconcat".into(),
    ]);

    // 输出 pattern 收尾。
    args.push(output_pattern.to_string_lossy().to_string());
    args
}

/// 锁定 ffmpeg n8.1 release 构建（spec §8 M0 冻结；BtbN release 构建）。
///
/// M11-C P1-5：运行时按「env 覆盖 → Tauri resource_dir → exe 同目录 → 开发回退」解析
/// （`sr_core::tools`）；失败错误列出全部尝试路径（**原始路径**由调用方按边界脱敏渲染）。
pub fn ffmpeg_exe() -> Result<PathBuf, sr_core::tools::ToolResolveError> {
    sr_core::tools::resolve_tool_binary("ffmpeg").map(|r| r.path)
}

/// 锁定 ffprobe（与 ffmpeg 同一解析器；收尾时间轴校准按段读取包时间戳）。
pub fn ffprobe_exe() -> Result<PathBuf, sr_core::tools::ToolResolveError> {
    sr_core::tools::resolve_tool_binary("ffprobe").map(|r| r.path)
}

// ─────────────────────── M19c：stderr 边界脱敏渲染原语（REQ-2/4） ───────────────────────

/// stderr 路径渲染：默认脱敏（仅 basename / `.screenrec_tmp` 会话后缀）；
/// `SR_DIAG_PATHS=1|true` 显式恢复完整路径（REQ-4）。错误 detail 一律走
/// `sr_core::redact::redact_path`（UI 出口不随环境变量变化）。
pub(crate) fn render_path_for_stderr(path: &Path) -> String {
    if sr_core::redact::paths_verbose() {
        path.to_string_lossy().into_owned()
    } else {
        sr_core::redact::redact_path(path)
    }
}

/// stderr 已知路径批量替换（ffmpeg/ffprobe 原始 stderr 尾部用；E_DISK_FULL 等
/// 关键词分类必须在调用本函数**之前**完成，REQ-5）；`SR_DIAG_PATHS=1|true` 显式
/// 恢复完整文本（REQ-4）。
pub(crate) fn render_text_for_stderr(text: &str, known: &[&Path]) -> String {
    if sr_core::redact::paths_verbose() {
        text.to_string()
    } else {
        sr_core::redact::redact_text_known_paths(text, known)
    }
}

/// stderr 工具解析错误渲染：默认 [`sr_core::redact::redact_tool_error`]（保留工具名/
/// 尝试顺序/错误类别，路径脱敏）；`SR_DIAG_PATHS=1|true` 显式恢复完整路径。
/// 不改 [`sr_core::tools::ToolResolveError`] 的 `Display`（`--self-check` 契约，REQ-5）。
pub(crate) fn render_tool_error_for_stderr(e: &sr_core::tools::ToolResolveError) -> String {
    if sr_core::redact::paths_verbose() {
        e.to_string()
    } else {
        sr_core::redact::redact_tool_error(e)
    }
}

/// D4 编码器链（定稿序）：`h264_nvenc → h264_qsv → h264_amf → h264_mf`（链末软件 MFT）。
const ENCODER_CHAIN: [&str; 4] = ["h264_nvenc", "h264_qsv", "h264_amf", "h264_mf"];

/// D4 探测结果缓存（进程内一次；实际初始化探测成本 ~秒级，逐路复用）。
static ENCODER_CACHE: OnceLock<Option<String>> = OnceLock::new();

/// D4 编码器探测：链序逐级**实际初始化**（合成帧短编码），首个可用者胜出。
pub fn detect_encoder() -> Option<String> {
    ENCODER_CACHE
        .get_or_init(|| {
            let exe = match ffmpeg_exe() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!(
                        "[sr-pipeline] 编码器探测跳过：{}",
                        render_tool_error_for_stderr(&e)
                    );
                    return None;
                }
            };
            for enc in ENCODER_CHAIN {
                if probe_encoder(&exe, enc) {
                    return Some(enc.to_string());
                }
            }
            None
        })
        .clone()
}

/// 单编码器实际初始化探测（pub 形态：显式设置必须可观察地成功/失败）。
pub fn probe_named_encoder(name: &str) -> bool {
    match ffmpeg_exe() {
        Ok(exe) => probe_encoder(&exe, name),
        Err(e) => {
            eprintln!(
                "[sr-pipeline] 显式编码器探测失败：{}",
                render_tool_error_for_stderr(&e)
            );
            false
        }
    }
}

/// M18/REQ-21：以**真实区域尺寸**实际初始化编码器（探测仍用 256×256）。
///
/// 区域路不得缩放/补边；尺寸不受编码器支持时启动必须失败。使用合成源短编码，
/// 与 D4 探测同口径（8s 兜底）。
pub fn probe_encoder_size(name: &str, width: u32, height: u32) -> bool {
    if width < 2 || height < 2 || width % 2 != 0 || height % 2 != 0 {
        return false;
    }
    let exe = match ffmpeg_exe() {
        Ok(exe) => exe,
        Err(e) => {
            eprintln!(
                "[sr-pipeline] 区域尺寸编码器探测失败：{}",
                render_tool_error_for_stderr(&e)
            );
            return false;
        }
    };
    let source = format!("color=c=black:s={width}x{height}:r=30:d=0.2");
    let mut child = match Command::new(exe)
        .args([
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &source,
            "-c:v",
            name,
            "-f",
            "null",
            "-",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[sr-pipeline] 区域尺寸编码器探测 spawn 失败：{e}");
            return false;
        }
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    loop {
        match child.try_wait() {
            Ok(Some(code)) => return code.success(),
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(25)),
            Err(_) => return false,
        }
    }
}

/// 单编码器实际初始化探测（含诊断明细；probe 模块共用——「实际初始化」而非仅枚举）。
pub(crate) fn probe_encoder_detail(exe: &Path, encoder: &str) -> (bool, String) {
    let mut child = match Command::new(exe)
        .args([
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=256x256:r=30:d=0.2",
            "-c:v",
            encoder,
            "-f",
            "null",
            "-",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return (false, format!("spawn 失败: {e}")),
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    loop {
        match child.try_wait() {
            Ok(Some(code)) => {
                return if code.success() {
                    (true, "lavfi 短编码实际初始化成功".into())
                } else {
                    (false, format!("实际初始化失败（exit={code}）"))
                };
            }
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return (false, "实际初始化超时（8s）".into());
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(25)),
            Err(e) => return (false, format!("等待失败: {e}")),
        }
    }
}

/// D4 单编码器实际初始化探测（lavfi 合成源短编码 → null；8s 兜底强杀）。
fn probe_encoder(exe: &Path, encoder: &str) -> bool {
    probe_encoder_detail(exe, encoder).0
}

/// 拉起 sidecar（stdin 关闭、stdout 丢弃、stderr 管道由调用方持续排空）；
/// 进程级 Kill-on-close Job 绑定（D8/REQ-22：主进程消失 → sidecar 连带终止）。
pub fn spawn_sidecar(args: &[String]) -> std::io::Result<Child> {
    // 错误消息在边界即脱敏（M19c：spawn 失败文本会进入 StartFailed detail/stderr）。
    let exe = ffmpeg_exe().map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            sr_core::redact::redact_tool_error(&e),
        )
    })?;
    let child = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    bind_to_kill_job(&child);
    Ok(child)
}

/// stderr 持续排空线程（S1b §7.6：防 stderr 管道背压死锁；保留尾部用于诊断）。
pub fn spawn_stderr_drainer(
    child: &mut Child,
    tail: std::sync::Arc<std::sync::Mutex<String>>,
) -> std::thread::JoinHandle<()> {
    use std::io::Read;
    const TAIL_BYTES: usize = 16 * 1024;
    let mut stderr = child.stderr.take().expect("stderr 已管道化");
    std::thread::Builder::new()
        .name("sr-sidecar-stderr".into())
        .spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match stderr.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let mut tail = tail.lock().expect("stderr 尾部锁");
                        tail.push_str(&String::from_utf8_lossy(&buf[..n]));
                        let excess = tail.len().saturating_sub(TAIL_BYTES);
                        if excess > 0 {
                            tail.drain(..excess);
                        }
                    }
                    Err(_) => break,
                }
            }
        })
        .expect("spawn stderr 排空线程")
}

// ─────────────────────────── 进程级 Kill-on-close Job（D8/REQ-22） ───────────────────────────

/// 进程级 Job 句柄（有意保持打开至进程退出：KILL_ON_JOB_CLOSE 在**最后一个句柄
/// 关闭**时生效——主进程死亡 → 句柄关闭 → 全部 sidecar 连带终止）。
struct JobHandle(windows::Win32::Foundation::HANDLE);
// SAFETY：句柄为内核对象引用（值语义），仅用于 AssignProcessToJobObject；生命周期
// 与进程一致（从不关闭），跨线程只读共享。
unsafe impl Send for JobHandle {}
unsafe impl Sync for JobHandle {}

static KILL_JOB: std::sync::OnceLock<Option<JobHandle>> = std::sync::OnceLock::new();

/// 进程级 Job（best-effort：环境不支持时降级为「无 Job 回收」，不阻塞录制）。
fn kill_job() -> Option<windows::Win32::Foundation::HANDLE> {
    KILL_JOB
        .get_or_init(|| {
            use windows::Win32::System::JobObjects::{
                CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            };
            unsafe {
                let job = match CreateJobObjectW(None, None) {
                    Ok(j) => j,
                    Err(e) => {
                        eprintln!(
                            "[sr-pipeline] CreateJobObjectW 失败（sidecar 无 Job 回收）: {e}"
                        );
                        return None;
                    }
                };
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if let Err(e) = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) {
                    eprintln!(
                        "[sr-pipeline] SetInformationJobObject 失败（sidecar 无 Job 回收）: {e}"
                    );
                    return None;
                }
                Some(JobHandle(job))
            }
        })
        .as_ref()
        .map(|j| j.0)
}

/// 把 sidecar 绑定到进程级 Kill-on-close Job（失败仅告警：录制继续，无连带回收）。
fn bind_to_kill_job(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::AssignProcessToJobObject;
    let Some(job) = kill_job() else { return };
    unsafe {
        if let Err(e) =
            AssignProcessToJobObject(job, HANDLE(child.as_raw_handle() as *mut core::ffi::c_void))
        {
            eprintln!("[sr-pipeline] sidecar Job 绑定失败（无 Job 回收）: {e}");
        }
    }
}

// ─────────────────────────── 锁定构建许可/校验和审计（M8/REQ-29） ───────────────────────────

/// REQ-29：锁定 n8.1 release 构建的冻结事实（审计锚点；与 spec §8 M0 冻结项一致）。
pub struct LockedBuildFacts {
    /// 版本串必含标记（`ffmpeg -version` 首行核验）。
    pub version_marker: &'static str,
    /// 锁定压缩包文件名（`tools/ffmpeg/` 下）。
    pub zip_file_name: &'static str,
    /// 锁定压缩包 SHA256（64 位大写十六进制；与 M0 冻结校验和一致）。
    pub zip_sha256: &'static str,
    /// buildconf 必含项（LGPL/许可锚点）。
    pub required_buildconf: &'static [&'static str],
    /// 禁用/GPL 编码器标记（`-encoders` 输出中应零命中；spec §1 GPL 禁入）。
    pub forbidden_encoder_markers: &'static [&'static str],
    /// 禁用的 muxer 标记（`-muxers` 输出中应零命中；当前用于 chromaprint/FFTW 回归）。
    pub forbidden_muxer_markers: &'static [&'static str],
}

static LOCKED_BUILD_FACTS: LockedBuildFacts = LockedBuildFacts {
    version_marker: "n8.1",
    zip_file_name: "ffmpeg-n8.1.3-18-g46d899d7a0-20261019-win64-lgpl.zip",
    zip_sha256: "015EB37E45AFB52901DFF77562819A524DEEED07DDEFADDE766ABB14339BC502",
    required_buildconf: &[
        "--enable-version3",
        "--disable-libx264",
        "--disable-libx265",
        "--disable-libxavs2",
        "--disable-libxvid",
    ],
    forbidden_encoder_markers: &["libx264", "libx265", "libxavs2", "libxvid"],
    forbidden_muxer_markers: &["chromaprint"],
};

/// 锁定构建冻结事实（REQ-29 审计锚点）。
pub fn locked_build_facts() -> &'static LockedBuildFacts {
    &LOCKED_BUILD_FACTS
}

/// 锁定构建实际审计结果（REQ-29：审计**实际二进制**而非仅元数据）。
pub struct BuildAudit {
    /// `-version` 首行版本串。
    pub version: String,
    /// 压缩包实测 SHA256（由 `certutil -hashfile` 计算）。
    pub zip_sha256: String,
    /// buildconf 必含项逐条命中（marker, hit）。
    pub buildconf_hits: Vec<(String, bool)>,
    /// 禁用/GPL 编码器标记命中（应为空）。
    pub forbidden_hits: Vec<String>,
    /// 禁用 muxer 标记命中（应为空；当前覆盖 chromaprint/FFTW）。
    pub forbidden_muxer_hits: Vec<String>,
    /// 随包 LICENSE.txt = LGPL v3 文本。
    pub license_text_ok: bool,
}

/// `certutil -hashfile <path> SHA256`（Windows 内置；无第三方工具依赖）。
fn certutil_sha256(path: &Path) -> Result<String, String> {
    let out = Command::new("certutil")
        .args(["-hashfile", path.to_str().ok_or("非 utf-8 路径")?, "SHA256"])
        .output()
        .map_err(|e| format!("spawn certutil 失败: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    for line in text.lines() {
        let compact: String = line.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if compact.len() == 64 {
            return Ok(compact.to_ascii_uppercase());
        }
    }
    Err(format!("certutil 输出未含 SHA256 摘要: {text}"))
}

/// REQ-29：对解压目录**实际运行** `-version`/`-buildconf`/`-encoders`（+ 读随包
/// LICENSE.txt + 计算 zip SHA256）并核验锁定事实。
pub fn audit_locked_build(ffmpeg_home: &Path, zip_path: &Path) -> Result<BuildAudit, String> {
    let exe = ffmpeg_home.join("bin/ffmpeg.exe");
    let run = |args: &[&str]| -> Result<String, String> {
        let out = Command::new(&exe)
            .args(args)
            .output()
            .map_err(|e| format!("spawn {} 失败: {e}", exe.display()))?;
        Ok(format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ))
    };
    let version = run(&["-hide_banner", "-version"])?;
    let buildconf = run(&["-hide_banner", "-buildconf"])?;
    let encoders = run(&["-hide_banner", "-encoders"])?;
    let muxers = run(&["-hide_banner", "-muxers"])?;
    let facts = locked_build_facts();
    let buildconf_hits = facts
        .required_buildconf
        .iter()
        .map(|m| ((*m).to_string(), buildconf.contains(m)))
        .collect();
    let forbidden_hits = facts
        .forbidden_encoder_markers
        .iter()
        .filter(|m| encoders.contains(**m))
        .map(|m| (*m).to_string())
        .collect();
    let forbidden_muxer_hits = facts
        .forbidden_muxer_markers
        .iter()
        .filter(|m| muxers.contains(**m))
        .map(|m| (*m).to_string())
        .collect();
    let license_text_ok = std::fs::read_to_string(ffmpeg_home.join("LICENSE.txt"))
        .map(|t| t.contains("GNU LESSER GENERAL PUBLIC LICENSE") && t.contains("Version 3"))
        .unwrap_or(false);
    Ok(BuildAudit {
        version: version.lines().next().unwrap_or("").to_string(),
        zip_sha256: certutil_sha256(zip_path)?,
        buildconf_hits,
        forbidden_hits,
        forbidden_muxer_hits,
        license_text_ok,
    })
}
