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
//! 错误脱敏（M19c REQ-3/5/6，specs/005-log-redaction.md）：各 `MergeError` 的
//! `detail` 与 ffmpeg/ffprobe stderr 尾部在**构造时**按边界脱敏——路径仅保留
//! basename（`.screenrec_tmp` 会话后缀保留），错误码/退出码/诊断关键词
//! （permission denied、no space 等）原样保留；结构体路径字段（`input`/`out`）
//! 保留原始路径（恢复/校验语义与 `merge_error_string` 边界不变）。
//! `SR_DIAG_PATHS=1|true` 恢复完整路径（REQ-4）。`ToolResolveError::Display`
//! 不改动；运行时错误经 [`sr_core::redact::redact_tool_error`] 脱敏渲染。
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
use sr_core::redact::{paths_verbose, redact_path, redact_text_known_paths, redact_tool_error};
use sr_core::tools::{resolve_tool_binary, ResolvedTool, ToolResolveError};

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
///
/// 边界脱敏（M19c REQ-3）：各 `detail` 中的输入/输出/临时/工具路径默认替换为
/// `<redacted>\<basename>`（`.screenrec_tmp` 会话后缀保留）；`SR_DIAG_PATHS=1`
/// 保留完整路径。结构体路径字段保留原始路径（内部语义不变）。
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
                // 底层 detail 内嵌完整输入路径：按已知路径脱敏，保留拒绝原因/标记字节。
                detail: render_text(&detail, &[path]),
            });
        }
    }

    // ── 输出：不覆盖输入/既有文件；父目录可用（临时物同目录 ⇒ 原子改名同卷） ──
    for path in &inputs {
        if same_path(&out, path) {
            return Err(MergeError::OutputUnavailable {
                out: out.clone(),
                detail: format!("输出路径与输入相同（不得覆盖输入）：{}", render_path(path)),
            });
        }
    }
    if out.exists() {
        return Err(MergeError::OutputUnavailable {
            out: out.clone(),
            detail: format!("输出路径已存在（不覆盖策略）：{}", render_path(&out)),
        });
    }
    let out_parent = parent_dir(&out);
    if !out_parent.is_dir() {
        return Err(MergeError::OutputUnavailable {
            out: out.clone(),
            detail: format!("输出父目录不存在/不可用：{}", render_path(&out_parent)),
        });
    }

    // ── 临时物（RAII：任何 Err/早退路径删除未提交半成品） ──
    let tmp = temp_path_for(&out);
    if tmp.exists() {
        return Err(MergeError::OutputUnavailable {
            out: out.clone(),
            detail: format!("临时路径已存在（并发冲突）：{}", render_path(&tmp)),
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
            detail: format!("spawn ffmpeg 失败（{}）：{e}", render_path(&ffmpeg.path)),
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
            // 原始 stderr 尾部可能回显输入/临时/ffmpeg 完整路径：按已知路径替换，
            // 保留退出码与诊断关键词（permission denied、no space 等）。
            detail: render_text(
                &format!("ffmpeg 退出非零（{status}）；stderr 尾部：{tail}"),
                &known_merge_paths(&inputs, &tmp, &out, &ffmpeg.path),
            ),
        });
    }

    // ── 产物校验（非空 + 轨计数）→ 原子改名提交 ──
    let size = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        return Err(MergeError::SidecarFailed {
            detail: format!(
                "ffmpeg 退出 0 但未产出输出（0 字节/缺失）：{}",
                render_path(&tmp)
            ),
        });
    }
    let (video_tracks, audio_tracks) =
        probe_track_counts(&tmp).map_err(|detail| MergeError::SidecarFailed { detail })?;

    if out.exists() {
        // 缩短「校验-改名」竞争窗（并发对端在窗口内提交了同名产物；不覆盖）。
        return Err(MergeError::OutputUnavailable {
            out: out.clone(),
            detail: format!("提交时输出路径已存在（并发竞争）：{}", render_path(&out)),
        });
    }
    std::fs::rename(&tmp, &out).map_err(|e| MergeError::OutputUnavailable {
        out: out.clone(),
        detail: format!(
            "原子改名失败（{} → {}）：{e}",
            render_path(&tmp),
            render_path(&out)
        ),
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

/// 错误脱敏边界（M19c REQ-3/4）：`verbose=false`（默认）时单路径渲染为
/// [`redact_path`] 结果；`verbose=true`（`SR_DIAG_PATHS=1|true`）保留完整路径。
/// 纯函数形式便于测试不触碰进程级环境变量。
fn render_path_verbose(verbose: bool, path: &Path) -> String {
    if verbose {
        path.display().to_string()
    } else {
        redact_path(path)
    }
}

/// 错误脱敏边界（M19c REQ-3/4）：`verbose=false`（默认）时自由文本（stderr 尾部/
/// 底层 detail）按已知路径替换为 [`redact_text_known_paths`] 结果——不整段丢弃，
/// 退出码/错误关键词原样保留；`verbose=true` 原样保留全文。
fn render_text_verbose(verbose: bool, text: &str, known: &[&Path]) -> String {
    if verbose {
        text.to_string()
    } else {
        redact_text_known_paths(text, known)
    }
}

/// 运行期包装：读取 `SR_DIAG_PATHS` 后委托 [`render_path_verbose`]。
fn render_path(path: &Path) -> String {
    render_path_verbose(paths_verbose(), path)
}

/// 运行期包装：读取 `SR_DIAG_PATHS` 后委托 [`render_text_verbose`]。
fn render_text(text: &str, known: &[&Path]) -> String {
    render_text_verbose(paths_verbose(), text, known)
}

/// 工具解析错误的边界渲染：`ToolResolveError::Display` 不变（REQ-5），默认改用
/// [`redact_tool_error`]（保留工具名/尝试顺序/类别，路径脱敏）；`SR_DIAG_PATHS=1`
/// 保留 Display 完整路径。
fn render_tool_error(err: &ToolResolveError) -> String {
    if paths_verbose() {
        err.to_string()
    } else {
        redact_tool_error(err)
    }
}

/// ffmpeg stderr 尾部脱敏的已知路径清单：全部输入 + 临时物 + 输出 + ffmpeg 本体。
fn known_merge_paths<'a>(
    inputs: &'a [PathBuf],
    tmp: &'a Path,
    out: &'a Path,
    ffmpeg: &'a Path,
) -> Vec<&'a Path> {
    let mut known: Vec<&Path> = inputs.iter().map(|p| p.as_path()).collect();
    known.push(tmp);
    known.push(out);
    known.push(ffmpeg);
    known
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
///
/// 错误 detail 在本函数内完成边界脱敏（M19c）：ffprobe 解析失败走
/// [`render_tool_error`]；spawn 失败的 ffprobe 路径与非零退出的 stderr 尾部按
/// 已知路径（被探测产物 + ffprobe 本体）替换，退出码保留。
fn probe_track_counts(path: &Path) -> Result<(usize, usize), String> {
    let ffprobe = resolve_tool_binary("ffprobe").map_err(|e| render_tool_error(&e))?;
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
        .map_err(|e| format!("spawn ffprobe 失败（{}）：{e}", render_path(&ffprobe.path)))?;
    if !out.status.success() {
        return Err(render_text(
            &format!(
                "ffprobe 校验产物失败（exit={}；stderr={}）",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ),
            &[path, &ffprobe.path],
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

/// 统一运行时工具解析（M11-C P1-5：env → Tauri resource → exe 同目录 → 开发回退）。
/// 失败 detail 用脱敏渲染（M19c：`ToolResolveError::Display` 不变，见
/// [`render_tool_error`]）。
fn resolve_merge_tool(name: &str) -> Result<ResolvedTool, MergeError> {
    resolve_tool_binary(name).map_err(|e| MergeError::SidecarFailed {
        detail: render_tool_error(&e),
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

#[cfg(test)]
mod redact_tests {
    //! M19c REQ-3/5/6 边界脱敏回归（specs/005-log-redaction.md §5）：
    //! detail/stderr 无 `C:\Users\`（用户名）、basename 保留、错误码/退出码/
    //! 诊断关键词保留；`SR_DIAG_PATHS=1` 恢复完整路径。不触碰 ffmpeg/长媒体。

    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use super::*;

    /// 串行化全部测试：`merge_recordings` 内部读 `SR_DIAG_PATHS`，
    /// 与环境变量测试并行会竞态（同 sr-core::redact 测试模式）。
    static SR_DIAG_PATHS_LOCK: Mutex<()> = Mutex::new(());

    /// 环境变量快照守卫：无论测试是否 panic 都恢复原值。
    struct EnvSnapshot {
        saved: Option<std::ffi::OsString>,
    }

    impl EnvSnapshot {
        fn take() -> Self {
            Self {
                saved: std::env::var_os("SR_DIAG_PATHS"),
            }
        }
    }

    impl Drop for EnvSnapshot {
        fn drop(&mut self) {
            match &self.saved {
                Some(v) => std::env::set_var("SR_DIAG_PATHS", v),
                None => std::env::remove_var("SR_DIAG_PATHS"),
            }
        }
    }

    /// 本测试专属临时目录（进程 id + 用例名隔离；清理尽力而为）。
    fn test_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sr-merge-redact-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建测试目录");
        dir
    }

    /// 写入带 Matroska EBML 存续标记的假产物（通过 `check_input` 前置校验）。
    fn write_ebml_stub(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(
            &path,
            [0x1A, 0x45, 0xDF, 0xA3, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        )
        .expect("写入 EBML 假产物");
        path
    }

    #[test]
    fn render_path_verbose_false_redacts_true_restores() {
        let _lock = SR_DIAG_PATHS_LOCK.lock().expect("SR_DIAG_PATHS 锁");
        let p = Path::new(r"C:\Users\elvis\Videos\seg.mkv");
        assert_eq!(render_path_verbose(false, p), r"<redacted>\seg.mkv");
        assert_eq!(
            render_path_verbose(true, p),
            r"C:\Users\elvis\Videos\seg.mkv"
        );
    }

    #[test]
    fn render_text_verbose_keeps_keywords_and_basename_without_user_dir() {
        let _lock = SR_DIAG_PATHS_LOCK.lock().expect("SR_DIAG_PATHS 锁");
        let input = PathBuf::from(r"C:\Users\elvis\Videos\sessions\in1.mkv");
        let tmp = PathBuf::from(
            r"C:\Users\elvis\AppData\Local\Temp\.screenrec_tmp\s-9\out.mkv.merge-tmp-42-0.mkv",
        );
        let out = PathBuf::from(r"C:\Users\elvis\Videos\final.mkv");
        let ffmpeg = PathBuf::from(r"C:\Users\elvis\app\bin\ffmpeg.exe");
        let inputs = vec![input.clone()];
        let tail = format!(
            "{}: Permission denied; {}: No space left on device（exit code 1）",
            input.display(),
            tmp.display()
        );

        let known = known_merge_paths(&inputs, &tmp, &out, &ffmpeg);
        let redacted = render_text_verbose(false, &tail, &known);
        // 无完整路径/用户名。
        assert!(!redacted.contains(r"C:\Users\"), "{redacted}");
        assert!(!redacted.contains("elvis"), "{redacted}");
        // basename 保留；`.screenrec_tmp` 会话后缀保留。
        assert!(redacted.contains(r"<redacted>\in1.mkv"), "{redacted}");
        assert!(
            redacted.contains(r".screenrec_tmp\s-9\out.mkv.merge-tmp-42-0.mkv"),
            "{redacted}"
        );
        // 诊断关键词/退出码保留。
        assert!(redacted.contains("Permission denied"), "{redacted}");
        assert!(redacted.contains("No space left on device"), "{redacted}");
        assert!(redacted.contains("exit code 1"), "{redacted}");

        // SR_DIAG_PATHS=1：原样保留全文（REQ-4）。
        assert_eq!(render_text_verbose(true, &tail, &known), tail);
    }

    #[test]
    fn invalid_input_detail_redacted_keeps_reason() {
        let _lock = SR_DIAG_PATHS_LOCK.lock().expect("SR_DIAG_PATHS 锁");
        let dir = test_dir("invalid-input");
        let good = write_ebml_stub(&dir, "good.mkv");
        let missing = dir.join("missing-a.mkv");

        // 不存在：拒绝原因保留，路径只剩 basename。
        let err = merge_recordings(MergeConfig {
            inputs: vec![missing.clone(), good.clone()],
            out: dir.join("out.mkv"),
        })
        .expect_err("缺失输入应拒绝");
        let MergeError::InvalidInput { detail, .. } = err else {
            panic!("应为 InvalidInput");
        };
        assert!(!detail.contains(r"C:\Users\"), "{detail}");
        assert!(detail.contains(r"<redacted>\missing-a.mkv"), "{detail}");
        assert!(detail.contains("输入不存在/不可读"), "{detail}");

        // 容器不接纳：标记字节诊断信息保留。
        let garbage = dir.join("garbage-b.bin");
        std::fs::write(&garbage, [0u8; 16]).expect("写入非容器文件");
        let err = merge_recordings(MergeConfig {
            inputs: vec![good, garbage],
            out: dir.join("out.mkv"),
        })
        .expect_err("非 MKV/MP4 输入应拒绝");
        let MergeError::InvalidInput { detail, .. } = err else {
            panic!("应为 InvalidInput");
        };
        assert!(!detail.contains(r"C:\Users\"), "{detail}");
        assert!(detail.contains(r"<redacted>\garbage-b.bin"), "{detail}");
        assert!(detail.contains("输入未被 MKV/MP4 接纳"), "{detail}");
        assert!(detail.contains("前 12 字节"), "{detail}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn output_unavailable_detail_redacted_keeps_policy_reason() {
        let _lock = SR_DIAG_PATHS_LOCK.lock().expect("SR_DIAG_PATHS 锁");
        let dir = test_dir("output-exists");
        let a = write_ebml_stub(&dir, "a.mkv");
        let b = write_ebml_stub(&dir, "b.mkv");
        let out = dir.join("exists-out.mkv");
        std::fs::write(&out, b"placeholder").expect("预置已存在输出");

        let err = merge_recordings(MergeConfig {
            inputs: vec![a, b],
            out: out.clone(),
        })
        .expect_err("已存在输出应拒绝");
        let MergeError::OutputUnavailable { detail, .. } = err else {
            panic!("应为 OutputUnavailable");
        };
        assert!(!detail.contains(r"C:\Users\"), "{detail}");
        assert!(detail.contains(r"<redacted>\exists-out.mkv"), "{detail}");
        assert!(detail.contains("输出路径已存在（不覆盖策略）"), "{detail}");
        // 不覆盖策略本身不变：既有文件未被触碰。
        assert_eq!(std::fs::read(&out).expect("读取既有输出"), b"placeholder");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn input_count_detail_preserved_without_paths() {
        let _lock = SR_DIAG_PATHS_LOCK.lock().expect("SR_DIAG_PATHS 锁");
        let dir = test_dir("count");
        let a = write_ebml_stub(&dir, "only.mkv");
        let err = merge_recordings(MergeConfig {
            inputs: vec![a],
            out: dir.join("out.mkv"),
        })
        .expect_err("单输入应拒绝");
        let MergeError::InvalidInput { detail, .. } = err else {
            panic!("应为 InvalidInput");
        };
        assert!(!detail.contains(r"C:\Users\"), "{detail}");
        assert!(!detail.contains('<'), "{detail}");
        assert!(detail.contains("合并需要 ≥2 个输入（得到 1）"), "{detail}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sr_diag_paths_env_switches_runtime_rendering() {
        let _lock = SR_DIAG_PATHS_LOCK.lock().expect("SR_DIAG_PATHS 锁");
        let _snapshot = EnvSnapshot::take();
        let p = Path::new(r"C:\Users\elvis\Videos\seg.mkv");

        std::env::remove_var("SR_DIAG_PATHS");
        assert_eq!(render_path(p), r"<redacted>\seg.mkv");

        std::env::set_var("SR_DIAG_PATHS", "1");
        assert_eq!(render_path(p), r"C:\Users\elvis\Videos\seg.mkv");

        std::env::set_var("SR_DIAG_PATHS", "0");
        assert_eq!(render_path(p), r"<redacted>\seg.mkv");
    }

    #[test]
    fn tool_resolve_error_detail_redacted_without_changing_display() {
        let _lock = SR_DIAG_PATHS_LOCK.lock().expect("SR_DIAG_PATHS 锁");
        let err = ToolResolveError {
            name: "ffmpeg".to_string(),
            attempted: vec![PathBuf::from(r"C:\Users\elvis\app\bin\ffmpeg.exe")],
        };
        // Display 本身不变（REQ-5：--self-check 依赖完整路径）。
        assert!(
            err.to_string()
                .contains(r"C:\Users\elvis\app\bin\ffmpeg.exe"),
            "{}",
            err
        );
        // 运行时 detail 渲染默认脱敏。
        let detail = render_tool_error(&err);
        assert!(!detail.contains(r"C:\Users\"), "{detail}");
        assert!(detail.contains("ffmpeg.exe"), "{detail}");
        assert!(detail.contains("无法定位 ffmpeg"), "{detail}");
    }
}
