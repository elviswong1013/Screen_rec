//! 合并执行（REQ-23）：≥2 产物 → 单 MKV（`-map 0 -map 1 … -c copy` 全轨保留，
//! 多视频轨并存）；**异构（分辨率/帧率/时长/轨数不同）不得误拒**；不用 `-shortest`
//! （保留完整时长 = 各文件按本地零点合并后的 max）；不得覆盖输入；失败/取消不提交
//! （先写同目录临时物 → 校验 → 原子改名，REQ-24 安全提交同精神；输入逐字节不变）。
//!
//! 流程：[`crate::inputs::check_input`] 逐输入前置校验 → 输出路径校验（与输入同路径/
//! 已存在（不覆盖策略）/父目录不可用 → `OutputUnavailable`）→ sidecar（spec §8 锁定
//! n8.1 release）流拷贝到临时物 → 非空 + ffprobe 轨计数校验 → 原子改名提交。任一失败
//! 路径经 RAII 守卫清理临时物（`out` 不出现半成品、输入不被触碰）。
//!
//! 取消形态（m7-red-report §8-3 处置）：[`merge_recordings`] 为同步一次调用（无取消点）；
//! 需要外部取消的调用方使用 [`merge_recordings_cancellable`]——置位标志后（≤50ms 轮询）
//! 终止 sidecar、清理临时物并返回可判别错误 [`MergeError::Cancelled`]。
//!
//! 进度上报（Channel）属 `src-tauri` 命令层后续接线，本同步 API 不承载。
//!
//! 轨道命名/默认选择规则（REQ-23「规则明确」）：**不改写任何轨道元数据**——`-c copy`
//! 流拷贝保留源轨 codec/尺寸/采样率/标题，默认轨 disposition 亦随流拷贝（实测输出
//! ≤1 默认轨/类型；见 m7-green-report §4）。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::inputs::check_input;
use sr_core::tools::{resolve_tool_binary, ResolvedTool};

/// REQ-23 合并配置。
#[derive(Clone, Debug)]
pub struct MergeConfig {
    /// 输入产物路径（≥2；异构允许）。
    pub inputs: Vec<PathBuf>,
    /// 成品 MKV 输出路径（不得与任一输入同路径；不覆盖既有文件）。
    pub out: PathBuf,
}

/// 合并成品摘要。
#[derive(Clone, Debug)]
pub struct MergeOutcome {
    /// 成品路径（== `cfg.out`）。
    pub out: PathBuf,
    /// 成品视频轨总数（各输入之和；多视频轨并存）。
    pub video_tracks: usize,
    /// 成品音频轨总数（各输入之和）。
    pub audio_tracks: usize,
}

/// 合并错误（测试契约最小变体集 + 取消变体，见 m7-red-report §3.1/§8-3）。
#[derive(Debug)]
pub enum MergeError {
    /// E_MERGE_INCOMPATIBLE（§3.4「仅不可读/未被 MKV 接纳」）：不存在/不可读/空/
    /// 非 Matroska 接纳的输入，或输入数 <2。**异构布局不属于此错误。**
    InvalidInput { input: PathBuf, detail: String },
    /// `out` 不可安全创建/提交：与任一输入同路径（禁止覆盖输入）、已存在（不覆盖策略）、
    /// 父目录不可用、原子改名失败。
    OutputUnavailable { out: PathBuf, detail: String },
    /// sidecar 执行失败（非零退出/产物校验失败；stderr 摘要）。
    SidecarFailed { detail: String },
    /// 外部取消：已终止 sidecar 并清理临时物（无半成品提交、输入保留）。
    Cancelled { detail: String },
}

/// 进程内临时物序号（同进程多次/并发合并的路径唯一性）。
static MERGE_SEQ: AtomicU32 = AtomicU32::new(0);

/// 合并 ≥2 产物 → 单 MKV（REQ-23；同步一次调用，无取消点——取消见
/// [`merge_recordings_cancellable`]）。
pub fn merge_recordings(cfg: MergeConfig) -> Result<MergeOutcome, MergeError> {
    merge_impl(cfg, None)
}

/// 可取消形态：`cancel` 置位（任意线程）→ 终止 sidecar、清理临时物、返回
/// [`MergeError::Cancelled`]；输入/输出路径不留半成品。
pub fn merge_recordings_cancellable(
    cfg: MergeConfig,
    cancel: &AtomicBool,
) -> Result<MergeOutcome, MergeError> {
    merge_impl(cfg, Some(cancel))
}

fn merge_impl(cfg: MergeConfig, cancel: Option<&AtomicBool>) -> Result<MergeOutcome, MergeError> {
    let MergeConfig { inputs, out } = cfg;

    // ── 输入：数量（<2 明确拒绝，不得静默成功）+ 逐条前置校验（异构一律放行） ──
    if inputs.len() < 2 {
        return Err(MergeError::InvalidInput {
            input: inputs.first().cloned().unwrap_or_default(),
            detail: format!("合并需要 ≥2 个输入（得到 {}）", inputs.len()),
        });
    }
    for path in &inputs {
        if let Err(detail) = check_input(path) {
            return Err(MergeError::InvalidInput {
                input: path.clone(),
                detail,
            });
        }
    }

    // ── 输出：不覆盖输入/既有文件；父目录可用（临时物同目录 ⇒ 原子改名同卷） ──
    for path in &inputs {
        if same_path(&out, path) {
            return Err(MergeError::OutputUnavailable {
                out: out.clone(),
                detail: format!("输出路径与输入相同（不得覆盖输入）：{}", path.display()),
            });
        }
    }
    if out.exists() {
        return Err(MergeError::OutputUnavailable {
            out: out.clone(),
            detail: format!("输出路径已存在（不覆盖策略）：{}", out.display()),
        });
    }
    let out_parent = parent_dir(&out);
    if !out_parent.is_dir() {
        return Err(MergeError::OutputUnavailable {
            out: out.clone(),
            detail: format!("输出父目录不存在/不可用：{}", out_parent.display()),
        });
    }

    // ── 临时物（RAII：任何 Err/早退路径删除未提交半成品） ──
    let tmp = temp_path_for(&out);
    if tmp.exists() {
        return Err(MergeError::OutputUnavailable {
            out: out.clone(),
            detail: format!("临时路径已存在（并发冲突）：{}", tmp.display()),
        });
    }
    let mut tmp_guard = TempGuard::new(tmp.clone());

    // ── sidecar 流拷贝（锁定构建；显式逐输入全轨映射 + c copy；无 -shortest） ──
    let args = build_merge_args(&inputs, &tmp);
    let ffmpeg = resolve_merge_tool("ffmpeg")?;
    let mut child = Command::new(&ffmpeg.path)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| MergeError::SidecarFailed {
            detail: format!("spawn ffmpeg 失败（{}）：{e}", ffmpeg.path.display()),
        })?;
    let stderr_tail = Arc::new(Mutex::new(String::new()));
    let drainer = spawn_stderr_drainer(&mut child, stderr_tail.clone());

    let status = loop {
        if let Some(c) = cancel {
            if c.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = drainer.join();
                return Err(MergeError::Cancelled {
                    detail: "外部取消：已终止 sidecar 并清理临时物（未提交成品）".into(),
                });
            }
        }
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = drainer.join();
                return Err(MergeError::SidecarFailed {
                    detail: format!("等待 sidecar 失败：{e}"),
                });
            }
        }
    };
    let _ = drainer.join();
    let tail = stderr_tail.lock().map(|t| t.clone()).unwrap_or_default();
    if !status.success() {
        return Err(MergeError::SidecarFailed {
            detail: format!("ffmpeg 退出非零（{status}）；stderr 尾部：{tail}"),
        });
    }

    // ── 产物校验（非空 + 轨计数）→ 原子改名提交 ──
    let size = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        return Err(MergeError::SidecarFailed {
            detail: format!(
                "ffmpeg 退出 0 但未产出输出（0 字节/缺失）：{}",
                tmp.display()
            ),
        });
    }
    let (video_tracks, audio_tracks) =
        probe_track_counts(&tmp).map_err(|detail| MergeError::SidecarFailed { detail })?;

    if out.exists() {
        // 缩短「校验-改名」竞争窗（并发对端在窗口内提交了同名产物；不覆盖）。
        return Err(MergeError::OutputUnavailable {
            out: out.clone(),
            detail: format!("提交时输出路径已存在（并发竞争）：{}", out.display()),
        });
    }
    std::fs::rename(&tmp, &out).map_err(|e| MergeError::OutputUnavailable {
        out: out.clone(),
        detail: format!("原子改名失败（{} → {}）：{e}", tmp.display(), out.display()),
    })?;
    tmp_guard.disarm();

    Ok(MergeOutcome {
        out,
        video_tracks,
        audio_tracks,
    })
}

/// 临时物 RAII 守卫：未提交（Err/早退）时删除，保证「失败不提交」。
struct TempGuard {
    path: PathBuf,
    armed: bool,
}

impl TempGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// 输出父目录（空 = 当前目录）。
fn parent_dir(out: &Path) -> PathBuf {
    match out.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// 与产物同目录的临时物路径（唯一：pid + 进程内序号）。
fn temp_path_for(out: &Path) -> PathBuf {
    let seq = MERGE_SEQ.fetch_add(1, Ordering::Relaxed);
    let name = out
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "merge.mkv".into());
    parent_dir(out).join(format!(
        ".{name}.merge-tmp-{}-{seq}.mkv",
        std::process::id()
    ))
}

/// 路径同一性（Windows 大小写/短名不敏感：存在路径 canonicalize 后比较；退化为字面比较）。
fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

/// 合并 argv（§3.3 基准：逐输入 `-i` → 逐输入显式全轨 `-map i` → `-c copy` → MKV）。
/// 不用 `-shortest`（保留完整时长）；不写 `-y`（临时物为全新路径，若意外存在则 sidecar
/// 失败而非覆盖）。
fn build_merge_args(inputs: &[PathBuf], tmp: &Path) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-loglevel".into(),
        "error".into(),
    ];
    for p in inputs {
        args.push("-i".into());
        args.push(p.to_string_lossy().to_string());
    }
    for i in 0..inputs.len() {
        args.push("-map".into());
        args.push(i.to_string());
    }
    args.extend(["-c".into(), "copy".into(), "-f".into(), "matroska".into()]);
    args.push(tmp.to_string_lossy().to_string());
    args
}

/// 产物轨计数（ffprobe；逐流 codec_type 行）。
fn probe_track_counts(path: &Path) -> Result<(usize, usize), String> {
    let ffprobe = resolve_tool_binary("ffprobe").map_err(|e| e.to_string())?;
    let out = Command::new(&ffprobe.path)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("spawn ffprobe 失败（{}）：{e}", ffprobe.path.display()))?;
    if !out.status.success() {
        return Err(format!(
            "ffprobe 校验产物失败（exit={}；stderr={}）",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let (mut v, mut a) = (0usize, 0usize);
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        match line.trim() {
            "video" => v += 1,
            "audio" => a += 1,
            _ => {}
        }
    }
    Ok((v, a))
}

/// 统一运行时工具解析（M11-C P1-5：env → Tauri resource → exe 同目录 → 开发回退）；
/// 失败时错误信息由 `sr_core::tools` 逐条列出尝试路径。
fn resolve_merge_tool(name: &str) -> Result<ResolvedTool, MergeError> {
    resolve_tool_binary(name).map_err(|e| MergeError::SidecarFailed {
        detail: e.to_string(),
    })
}

/// stderr 持续排空（防管道背压死锁；保留尾部供诊断——sr-pipeline 同款模式）。
fn spawn_stderr_drainer(
    child: &mut Child,
    tail: Arc<Mutex<String>>,
) -> std::thread::JoinHandle<()> {
    use std::io::Read;
    const TAIL_BYTES: usize = 16 * 1024;
    let mut stderr = child.stderr.take().expect("stderr 已管道化");
    std::thread::Builder::new()
        .name("sr-merge-stderr".into())
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
