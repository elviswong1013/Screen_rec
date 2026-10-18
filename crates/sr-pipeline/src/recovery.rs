//! 恢复状态机（M8/REQ-22）：会话清单（`manifest.json`，原子更新）+ 目录重扫核对 +
//! 崩溃收尾（幂等）+ 显式丢弃。
//!
//! 契约（测试即契约，docs/milestones/m8-red-report.md §3.2 + recovery.rs）：
//! - 会话目录 = `<output_dir>/.screenrec_tmp/<session_id>/`（REQ-24/D12），内含
//!   `manifest.json` 与段文件；成品（REQ-24 命名）位于 output_dir 根。
//! - 完成段判定：顶层 `*.mkv` 中 size>0 且 ffprobe 可解析者「有效」；封口 = 有效且
//!   （清单标 closed 或存在更高序号的有效文件）；尾段 = 最高序号的有效未封口文件，
//!   回收可选（失败/弃用不删文件）。0 字节/坏 EBML 一律排除。
//! - 活动判定 = 会话内段/WAV 的**独占打开探测**（share_mode 0）：任何在写句柄都会
//!   使探测失败 → `Active` 拒绝收尾；**不得以 pid 存活作判据**（同进程内失败后必须
//!   可立即重试收尾，m8-red §3.2-3）。
//! - 陈旧清单（「段完成但清单未更新」）→ 目录重扫并集，完成段必参与收尾。
//! - 成品验证成功后按保留策略清理会话目录（[`finalize_session`] 默认清理；
//!   [`finalize_session_with_options`] 的 `keep_segments=true` 保留原段）。失败/活动拒绝/
//!   清单损坏路径永不删除原段；幂等（收尾成功后清单记 `final_path`，二次调用先按
//!   收尾同口径复核成品再复用——损坏成品报错并保留原段；默认清理后会话目录已不存在，
//!   再次调用返回 `NotFound`；需重试/核对原段时用 keep=true）。
//! - WAV 崩溃残留就地封口（复用 [`sr_audio::wav::repair_wav`]），列入 `repaired_wavs`。
//!   WAV 恢复的成品仍在会话目录内，故 WAV 路由不触发清理（避免删除录音本身）。
//!
//! 清单格式：`manifest.json` 为受限 JSON（本模块内手写序列化/解析——仓库保持零新增
//! 外部依赖；字段集即 [`SessionManifest`] schema v1，未知/畸形内容 → 解析失败）。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::naming::{
    next_available, safe_commit, sanitize_target_name, session_tmp_dir, CommitError,
    SESSION_TMP_DIRNAME,
};

/// 会话目录内清单文件名（钉死）。
pub const MANIFEST_FILE: &str = "manifest.json";

/// 路由身份（§3.4；展示/诊断用途——恢复可收尾性判据为活动文件探测，非 pid，见模块注）。
#[derive(Clone, Debug)]
pub struct RouteIdentity {
    pub pid: u32,
    pub created_at_unix_ms: u64,
    pub name: String,
    pub path: PathBuf,
}

/// 清单段条目。
#[derive(Clone, Debug)]
pub struct ManifestSegment {
    pub path: PathBuf,
    pub t_start: f64,
    pub t_end: f64,
    pub closed: bool,
}

/// 清单路由条目。
#[derive(Clone, Debug)]
pub struct ManifestRoute {
    pub route_id: String,
    pub identity: RouteIdentity,
    pub streams: Vec<String>,
    pub segments: Vec<ManifestSegment>,
    pub state: String,
    pub final_path: Option<PathBuf>,
}

/// 会话清单（schema v1，§3.4）。
#[derive(Clone, Debug)]
pub struct SessionManifest {
    pub version: u32,
    pub session_id: String,
    pub created_at_unix_ms: u64,
    pub start_qpc: i64,
    pub routes: Vec<ManifestRoute>,
}

/// 恢复错误（§3.5：E_RECOVERY_FAILED 保留原段；活动会话拒绝收尾）。
#[derive(Debug)]
pub enum RecoveryError {
    NotFound { dir: PathBuf },
    Active { session_id: String },
    NoUsableSegments { detail: String },
    RecoveryFailed { detail: String },
}

/// 可恢复会话（扫描视角）。
#[derive(Clone, Debug)]
pub struct RecoverableSession {
    pub session_id: String,
    pub dir: PathBuf,
    /// 损坏/缺失 → None（不静默丢弃目录）。
    pub manifest: Option<SessionManifest>,
    /// 可收尾完整段（0 字节/坏 EBML/活动文件排除）。
    pub sealed_segments: Vec<PathBuf>,
    /// 末可解析段（未封口形态；不得当封口段用）。
    pub tail_segment: Option<PathBuf>,
    /// 活动文件探测（独占打开失败）。
    pub active: bool,
}

/// 收尾产出。
#[derive(Clone, Debug)]
pub struct RecoveryOutcome {
    /// route_id → 成品（output_dir 根，REQ-24 命名；不含 WAV 路由）。
    pub route_finals: Vec<(String, PathBuf)>,
    /// 参与收尾的封口段。`keep_segments=true` 时保留；默认收尾成功后随会话目录清理
    /// （路径不再存在）。
    pub segments_used: Vec<PathBuf>,
    pub discarded: Vec<PathBuf>,
    /// WAV 崩溃残留就地封口。
    pub repaired_wavs: Vec<PathBuf>,
}

fn fail(detail: impl Into<String>) -> RecoveryError {
    RecoveryError::RecoveryFailed {
        detail: detail.into(),
    }
}

// ─────────────────────────── 受限 JSON（手写；字段集 = schema v1） ───────────────────────────

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn manifest_to_json(m: &SessionManifest) -> String {
    let mut out = String::with_capacity(256 + m.routes.len() * 256);
    out.push_str(&format!(
        "{{\"version\":{},\"session_id\":\"{}\",\"created_at_unix_ms\":{},\"start_qpc\":{},\"routes\":[",
        m.version,
        json_escape(&m.session_id),
        m.created_at_unix_ms,
        m.start_qpc
    ));
    for (i, r) in m.routes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"route_id\":\"{}\",\"identity\":{{\"pid\":{},\"created_at_unix_ms\":{},\"name\":\"{}\",\"path\":\"{}\"}},\"streams\":[",
            json_escape(&r.route_id),
            r.identity.pid,
            r.identity.created_at_unix_ms,
            json_escape(&r.identity.name),
            json_escape(&r.identity.path.to_string_lossy())
        ));
        for (j, s) in r.streams.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            out.push_str(&format!("\"{}\"", json_escape(s)));
        }
        out.push_str("],\"segments\":[");
        for (j, s) in r.segments.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            out.push_str(&format!(
                "{{\"path\":\"{}\",\"t_start\":{},\"t_end\":{},\"closed\":{}}}",
                json_escape(&s.path.to_string_lossy()),
                s.t_start,
                s.t_end,
                s.closed
            ));
        }
        out.push_str(&format!(
            "],\"state\":\"{}\",\"final_path\":",
            json_escape(&r.state)
        ));
        match &r.final_path {
            Some(p) => out.push_str(&format!("\"{}\"", json_escape(&p.to_string_lossy()))),
            None => out.push_str("null"),
        }
        out.push('}');
    }
    out.push_str("]}");
    out
}

#[derive(Debug, Clone)]
enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            bytes: s.as_bytes(),
            pos: 0,
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.bytes.get(self.pos), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn parse_value(&mut self) -> Result<Json, String> {
        self.skip_ws();
        match self
            .peek()
            .ok_or_else(|| "意外结束（空 JSON）".to_string())?
        {
            b'{' => self.parse_object(),
            b'[' => self.parse_array(),
            b'"' => Ok(Json::Str(self.parse_string()?)),
            b't' => {
                self.expect_lit("true")?;
                Ok(Json::Bool(true))
            }
            b'f' => {
                self.expect_lit("false")?;
                Ok(Json::Bool(false))
            }
            b'n' => {
                self.expect_lit("null")?;
                Ok(Json::Null)
            }
            b'-' | b'0'..=b'9' => Ok(Json::Num(self.parse_number()?)),
            other => Err(format!(
                "非法 JSON 字符 {:?}（pos={}）",
                other as char, self.pos
            )),
        }
    }

    fn expect_lit(&mut self, lit: &str) -> Result<(), String> {
        if self.bytes[self.pos..].starts_with(lit.as_bytes()) {
            self.pos += lit.len();
            Ok(())
        } else {
            Err(format!("期望字面量 {lit}（pos={}）", self.pos))
        }
    }

    fn parse_number(&mut self) -> Result<String, String> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E') {
                self.pos += 1;
            } else {
                break;
            }
        }
        if start == self.pos {
            return Err("空数字".into());
        }
        std::str::from_utf8(&self.bytes[start..self.pos])
            .map(|s| s.to_string())
            .map_err(|_| "数字非 UTF-8".to_string())
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if self.peek() != Some(b'"') {
            return Err(format!("期望字符串（pos={}）", self.pos));
        }
        self.pos += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = self.peek().ok_or_else(|| "字符串未闭合".to_string())?;
            self.pos += 1;
            match c {
                b'"' => return String::from_utf8(out).map_err(|e| format!("字符串非 UTF-8：{e}")),
                b'\\' => {
                    let e = self.peek().ok_or_else(|| "转义未完成".to_string())?;
                    self.pos += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            if self.pos + 4 > self.bytes.len() {
                                return Err("\\u 转义截断".into());
                            }
                            let hex = std::str::from_utf8(&self.bytes[self.pos..self.pos + 4])
                                .map_err(|_| "\\u 转义非 UTF-8".to_string())?;
                            self.pos += 4;
                            let cp = u32::from_str_radix(hex, 16)
                                .map_err(|_| "\\u 转义非十六进制".to_string())?;
                            let ch = char::from_u32(cp)
                                .ok_or_else(|| "\\u 码点非法（代理对不支持）".to_string())?;
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        other => return Err(format!("未知转义 \\{}", other as char)),
                    }
                }
                _ => out.push(c),
            }
        }
    }

    fn parse_object(&mut self) -> Result<Json, String> {
        self.pos += 1; // '{'
        let mut obj: Vec<(String, Json)> = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Obj(obj));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(format!("对象键后期望 ':'（pos={}）", self.pos));
            }
            self.pos += 1;
            let val = self.parse_value()?;
            obj.push((key, val));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Obj(obj));
                }
                _ => return Err(format!("对象内期望 ',' 或 '}}'（pos={}）", self.pos)),
            }
        }
    }

    fn parse_array(&mut self) -> Result<Json, String> {
        self.pos += 1; // '['
        let mut arr: Vec<Json> = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Arr(arr));
        }
        loop {
            let v = self.parse_value()?;
            arr.push(v);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Arr(arr));
                }
                _ => return Err(format!("数组内期望 ',' 或 ']'（pos={}）", self.pos)),
            }
        }
    }
}

fn jget<'a>(obj: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    obj.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn jstr(v: &Json) -> Result<String, String> {
    match v {
        Json::Str(s) => Ok(s.clone()),
        other => Err(format!("期望字符串：{other:?}")),
    }
}

fn jnum(v: &Json, what: &str) -> Result<String, String> {
    match v {
        Json::Num(s) => Ok(s.clone()),
        other => Err(format!("期望数字（{what}）：{other:?}")),
    }
}

fn jf64(v: &Json, what: &str) -> Result<f64, String> {
    jnum(v, what)?
        .parse::<f64>()
        .map_err(|e| format!("{what} 非浮点数：{e}"))
}

fn manifest_from_json(text: &str) -> Result<SessionManifest, String> {
    let mut p = JsonParser::new(text);
    let root = p.parse_value()?;
    p.skip_ws();
    if p.pos != p.bytes.len() {
        return Err(format!("JSON 尾随内容（pos={}）", p.pos));
    }
    let obj = match root {
        Json::Obj(o) => o,
        _ => return Err("顶层应为 JSON 对象".into()),
    };
    let version = jnum(jget(&obj, "version").ok_or("缺 version 字段")?, "version")?
        .parse::<u32>()
        .map_err(|e| format!("version 非整数：{e}"))?;
    if version != 1 {
        return Err(format!("不支持的 manifest 版本 {version}"));
    }
    let session_id = jstr(jget(&obj, "session_id").ok_or("缺 session_id 字段")?)?;
    let created_at_unix_ms = jnum(
        jget(&obj, "created_at_unix_ms").ok_or("缺 created_at_unix_ms 字段")?,
        "created_at_unix_ms",
    )?
    .parse::<u64>()
    .map_err(|e| format!("created_at_unix_ms 非整数：{e}"))?;
    let start_qpc = jnum(
        jget(&obj, "start_qpc").ok_or("缺 start_qpc 字段")?,
        "start_qpc",
    )?
    .parse::<i64>()
    .map_err(|e| format!("start_qpc 非整数：{e}"))?;
    let routes_json = jget(&obj, "routes").ok_or("缺 routes 字段")?;
    let Json::Arr(routes_json) = routes_json else {
        return Err("routes 应为数组".into());
    };
    let mut routes = Vec::with_capacity(routes_json.len());
    for (i, rv) in routes_json.iter().enumerate() {
        let Json::Obj(ro) = rv else {
            return Err(format!("routes[{i}] 应为对象"));
        };
        let route_id = jstr(jget(ro, "route_id").ok_or("route 缺 route_id")?)?;
        let Json::Obj(io) = jget(ro, "identity").ok_or("route 缺 identity")? else {
            return Err("identity 应为对象".into());
        };
        let identity = RouteIdentity {
            pid: jnum(jget(io, "pid").ok_or("identity 缺 pid")?, "pid")?
                .parse::<u32>()
                .map_err(|e| format!("pid 非整数：{e}"))?,
            created_at_unix_ms: jnum(
                jget(io, "created_at_unix_ms").ok_or("identity 缺 created_at_unix_ms")?,
                "created_at",
            )?
            .parse::<u64>()
            .map_err(|e| format!("identity.created_at 非整数：{e}"))?,
            name: jstr(jget(io, "name").ok_or("identity 缺 name")?)?,
            path: PathBuf::from(jstr(jget(io, "path").ok_or("identity 缺 path")?)?),
        };
        let Json::Arr(streams_json) = jget(ro, "streams").ok_or("route 缺 streams")? else {
            return Err("streams 应为数组".into());
        };
        let streams = streams_json
            .iter()
            .map(jstr)
            .collect::<Result<Vec<_>, _>>()?;
        let Json::Arr(segments_json) = jget(ro, "segments").ok_or("route 缺 segments")? else {
            return Err("segments 应为数组".into());
        };
        let mut segments = Vec::with_capacity(segments_json.len());
        for (j, sv) in segments_json.iter().enumerate() {
            let Json::Obj(so) = sv else {
                return Err(format!("segments[{j}] 应为对象"));
            };
            segments.push(ManifestSegment {
                path: PathBuf::from(jstr(jget(so, "path").ok_or("segment 缺 path")?)?),
                t_start: jf64(jget(so, "t_start").ok_or("segment 缺 t_start")?, "t_start")?,
                t_end: jf64(jget(so, "t_end").ok_or("segment 缺 t_end")?, "t_end")?,
                closed: match jget(so, "closed").ok_or("segment 缺 closed")? {
                    Json::Bool(b) => *b,
                    other => return Err(format!("closed 应为布尔：{other:?}")),
                },
            });
        }
        let state = jstr(jget(ro, "state").ok_or("route 缺 state")?)?;
        let final_path = match jget(ro, "final_path") {
            Some(Json::Null) | None => None,
            Some(Json::Str(s)) => Some(PathBuf::from(s)),
            Some(other) => return Err(format!("final_path 应为字符串或 null：{other:?}")),
        };
        routes.push(ManifestRoute {
            route_id,
            identity,
            streams,
            segments,
            state,
            final_path,
        });
    }
    Ok(SessionManifest {
        version,
        session_id,
        created_at_unix_ms,
        start_qpc,
        routes,
    })
}

// ─────────────────────────── 清单原子读写 ───────────────────────────

static MANIFEST_TMP_SEQ: AtomicU32 = AtomicU32::new(0);

/// 原子保存：临时文件 + rename（成功后无残留；可覆盖更新）。
pub fn save_manifest_atomic(dir: &Path, m: &SessionManifest) -> Result<(), RecoveryError> {
    let text = manifest_to_json(m);
    let seq = MANIFEST_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".manifest.{}.{}.tmp", std::process::id(), seq));
    std::fs::write(&tmp, text).map_err(|e| {
        fail(format!(
            "写 manifest 临时文件失败（{}）：{e}",
            tmp.display()
        ))
    })?;
    let target = dir.join(MANIFEST_FILE);
    std::fs::rename(&tmp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        fail(format!(
            "manifest 原子替换失败（{}）：{e}",
            target.display()
        ))
    })
}

/// 读取并解析会话清单；损坏/缺失 → [`RecoveryError::RecoveryFailed`]。
pub fn load_manifest(dir: &Path) -> Result<SessionManifest, RecoveryError> {
    let path = dir.join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| fail(format!("读 manifest 失败（{}）：{e}", path.display())))?;
    manifest_from_json(&text)
        .map_err(|e| fail(format!("manifest 解析失败（{}）：{e}", path.display())))
}

// ─────────────────────────── 段分类 / 活动探测 ───────────────────────────

fn ffprobe_path() -> Result<PathBuf, String> {
    crate::sidecar::ffprobe_exe()
}

fn list_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file() && p.extension().map_or(false, |x| x.eq_ignore_ascii_case(ext))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// 「有效」段：size>0 且 ffprobe 可解析（封口集合的唯一候选来源）。
/// ffprobe 不可解析/未定位 → 全部视为无效（恢复扫描仍列出目录）。
fn is_valid_mkv(ffprobe: Option<&Path>, p: &Path) -> bool {
    if !std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false) {
        return false;
    }
    let Some(ffprobe) = ffprobe else { return false };
    Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(p)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 活动文件探测（REQ-22「恢复扫描排除活动文件」）：会话内段/WAV 任一无法独占打开
/// （share_mode 0；任何在写句柄都会冲突）→ 视为仍在写入。文件不存在（瞬态）不阻塞。
fn session_is_active(dir: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    for p in list_ext(dir, "mkv").into_iter().chain(list_ext(dir, "wav")) {
        match std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&p)
        {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return true,
        }
    }
    false
}

/// 封口/尾段切分：封口 = 清单标 closed 或存在更高序号的有效文件；尾段 = 最高序号的
/// 有效未封口文件。
fn split_sealed(
    valid_sorted: &[PathBuf],
    closed: &HashSet<PathBuf>,
) -> (Vec<PathBuf>, Option<PathBuf>) {
    let mut sealed = Vec::new();
    let mut tail = None;
    for (i, p) in valid_sorted.iter().enumerate() {
        if closed.contains(p) || i + 1 < valid_sorted.len() {
            sealed.push(p.clone());
        } else {
            tail = Some(p.clone());
        }
    }
    (sealed, tail)
}

// ─────────────────────────── 扫描 ───────────────────────────

/// 扫描 `<output_dir>/.screenrec_tmp/*`；非会话目录跳过。清单损坏 → `manifest=None`
/// （仍列出目录与可收尾段，不静默丢弃）。
pub fn scan_sessions(output_dir: &Path) -> Vec<RecoverableSession> {
    let mut out = Vec::new();
    let root = output_dir.join(SESSION_TMP_DIRNAME);
    let Ok(rd) = std::fs::read_dir(&root) else {
        return out;
    };
    let ffprobe = ffprobe_path().ok();
    let mut dirs: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for dir in dirs {
        let manifest = load_manifest(&dir).ok();
        let mkvs = list_ext(&dir, "mkv");
        let wavs = list_ext(&dir, "wav");
        if manifest.is_none() && mkvs.is_empty() && wavs.is_empty() {
            continue; // 非会话目录
        }
        let closed: HashSet<PathBuf> = manifest
            .as_ref()
            .map(|m| {
                m.routes
                    .iter()
                    .flat_map(|r| r.segments.iter())
                    .filter(|s| s.closed)
                    .map(|s| s.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        let valids: Vec<PathBuf> = mkvs
            .into_iter()
            .filter(|p| is_valid_mkv(ffprobe.as_deref(), p))
            .collect();
        let (sealed_segments, tail_segment) = split_sealed(&valids, &closed);
        let active = session_is_active(&dir);
        out.push(RecoverableSession {
            session_id: dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            dir,
            manifest,
            sealed_segments,
            tail_segment,
            active,
        });
    }
    out
}

// ─────────────────────────── 收尾 / 丢弃 ───────────────────────────

/// 对话内 WAV 就地封口（幂等）；返回封口的 WAV 路径。
fn repair_wavs_in(dir: &Path) -> Result<Vec<PathBuf>, RecoveryError> {
    let mut repaired = Vec::new();
    for wav in list_ext(dir, "wav") {
        sr_audio::wav::repair_wav(&wav)
            .map_err(|e| fail(format!("WAV 封口失败（{}）：{e:?}", wav.display())))?;
        repaired.push(wav);
    }
    Ok(repaired)
}

/// 崩溃收尾（幂等）：清单可读 + 会话非活动 → 逐路收尾；成品落 output_dir 根
/// （REQ-24 命名；安全提交）。等价于 [`finalize_session_with_options`] 且
/// `keep_segments=false`（M14c 默认策略：成品验证成功后清理会话目录）。
pub fn finalize_session(session_dir: &Path) -> Result<RecoveryOutcome, RecoveryError> {
    finalize_session_with_options(session_dir, false)
}

/// M14c：崩溃收尾 + 显式保留策略（命令层把设置 `keep_segments` 透传至此）。
///
/// `keep_segments == false`（默认设置）：全部路由收尾成功且成品校验通过后递归清理
/// `<output_dir>/.screenrec_tmp/<session_id>`；`true`：保留原段（调试/排查/幂等重试）。
/// 删除失败只记录日志、不改变收尾成功归因。失败/活动拒绝/清单损坏路径不删除原段。
/// WAV 路由的成品（修复后的 WAV）位于会话目录内，故只要本次收尾修复了 WAV 就不清理，
/// 避免删除录音本身。
pub fn finalize_session_with_options(
    session_dir: &Path,
    keep_segments: bool,
) -> Result<RecoveryOutcome, RecoveryError> {
    if !session_dir.is_dir() {
        return Err(RecoveryError::NotFound {
            dir: session_dir.to_path_buf(),
        });
    }
    let mut manifest = load_manifest(session_dir)?; // 损坏/缺失 → RecoveryFailed（保留原段）
    if session_is_active(session_dir) {
        return Err(RecoveryError::Active {
            session_id: manifest.session_id.clone(),
        });
    }
    let output_root = session_dir
        .parent()
        .and_then(|p| p.parent())
        .filter(|p| p.join(SESSION_TMP_DIRNAME) == *session_dir.parent().unwrap_or(Path::new("")))
        .ok_or_else(|| fail("会话目录结构异常（应为 <output_dir>/.screenrec_tmp/<session_id>）"))?
        .to_path_buf();

    let ffprobe = ffprobe_path().map_err(|e| fail(e))?;
    let mut outcome = RecoveryOutcome {
        route_finals: Vec::new(),
        segments_used: Vec::new(),
        discarded: Vec::new(),
        repaired_wavs: Vec::new(),
    };

    for route_idx in 0..manifest.routes.len() {
        let route = manifest.routes[route_idx].clone();
        // 幂等：清单已记成品且存在 → 先按收尾同口径复核，再复用（无操作）。
        // 损坏/零字节成品不得视为成功：保留原段并报错，避免默认清理删掉原段。
        if let Some(f) = &route.final_path {
            if f.exists() {
                crate::finalize::verify_final_media(&ffprobe, f).map_err(|e| {
                    fail(format!(
                        "清单已记成品但校验失败（route {}）：{e}；原段保留、拒绝清理",
                        route.route_id
                    ))
                })?;
                outcome
                    .route_finals
                    .push((route.route_id.clone(), f.clone()));
                continue;
            }
        }
        // WAV 路由（纯录音）：崩溃残留就地封口，无 MKV 成品。
        if route.streams.iter().any(|s| s == "wav") {
            outcome.repaired_wavs.extend(repair_wavs_in(session_dir)?);
            continue;
        }
        // MKV 路由：目录重扫（陈旧清单并集）→ 封口段 concat → 安全提交。
        let closed: HashSet<PathBuf> = route
            .segments
            .iter()
            .filter(|s| s.closed)
            .map(|s| s.path.clone())
            .collect();
        let valids: Vec<PathBuf> = list_ext(session_dir, "mkv")
            .into_iter()
            .filter(|p| is_valid_mkv(Some(&ffprobe), p))
            .collect();
        let (sealed, tail) = split_sealed(&valids, &closed);
        if sealed.is_empty() {
            return Err(RecoveryError::NoUsableSegments {
                detail: format!(
                    "route {} 无可收尾段（{}）",
                    route.route_id,
                    session_dir.display()
                ),
            });
        }
        let list_path = session_dir.join(format!(".finalize.{}.ffconcat", route.route_id));
        // M10b：带音轨路由的段 `duration` 以视频轨为锚点校准（消除 concat 段首前导
        // 空洞）；纯视频路由保持容器时长语义。
        let has_audio = route.streams.iter().any(|s| s.starts_with('a'));
        crate::finalize::write_segment_list(&list_path, &sealed, has_audio).map_err(|e| fail(e))?;
        let tmp = session_dir.join(format!(
            ".final.{}.{}.mkv.tmp",
            route.route_id,
            std::process::id()
        ));
        let ffmpeg = crate::sidecar::ffmpeg_exe().map_err(|e| fail(e))?;
        let st = Command::new(ffmpeg)
            .args(["-hide_banner", "-nostdin", "-loglevel", "error"])
            .args(["-f", "concat", "-safe", "0", "-i"])
            .arg(&list_path)
            .args(["-map", "0", "-c", "copy", "-f", "matroska"])
            .arg(&tmp)
            .status()
            .map_err(|e| fail(format!("spawn concat 失败：{e}")))?;
        let _ = std::fs::remove_file(&list_path);
        if !st.success() {
            let _ = std::fs::remove_file(&tmp);
            return Err(fail(format!("concat 失败（exit={st}）；原段保留")));
        }
        let stem = format!(
            "{}_{}",
            crate::naming::format_timestamp(
                crate::route::current_unix_ms(),
                crate::route::local_utc_offset_minutes()
            ),
            sanitize_target_name(&route.identity.name)
        );
        let final_path = next_available(&output_root, &stem, "mkv");
        let verify =
            |p: &Path| -> Result<(), String> { crate::finalize::verify_final_media(&ffprobe, p) };
        if let Err(e) = safe_commit(&tmp, &final_path, verify) {
            let _ = std::fs::remove_file(&tmp);
            return Err(match e {
                CommitError::FinalExists { final_path } => {
                    fail(format!("成品名冲突（{}）", final_path.display()))
                }
                CommitError::VerifyFailed { detail } => fail(detail),
                CommitError::Io { detail } => fail(format!("成品提交失败：{detail}")),
            });
        }
        outcome
            .route_finals
            .push((route.route_id.clone(), final_path.clone()));
        outcome.segments_used.extend(sealed);
        if let Some(t) = tail {
            outcome.discarded.push(t);
        }
        manifest.routes[route_idx].final_path = Some(final_path);
        manifest.routes[route_idx].state = "final".into();
        save_manifest_atomic(session_dir, &manifest)?;
    }
    // M14c：全部路由收尾成功（MKV 成品已由 safe_commit 校验；WAV 已就地封口）→
    // 按保留策略清理会话目录。WAV 恢复的成品在会话目录内，不能清理。
    if !outcome.route_finals.is_empty() && outcome.repaired_wavs.is_empty() {
        cleanup_session_after_finalize(session_dir, keep_segments);
    }
    Ok(outcome)
}

/// M14c：恢复收尾成功后按保留策略清理会话目录（与正常收尾共用 keep_segments 语义）。
///
/// `keep_segments=true`：保留原段；false：递归删除会话目录。删除失败（文件占用/权限等）
/// 只记录日志并保留现场——调用方不得因此把收尾判失败（成品已提交）。
fn cleanup_session_after_finalize(session_dir: &Path, keep_segments: bool) {
    if keep_segments || !session_dir.is_dir() {
        return;
    }
    if let Err(e) = discard_session(session_dir) {
        eprintln!(
            "[recovery] 恢复收尾成功，但清理原段失败（保留会话目录 {}）：{e:?}",
            session_dir.display()
        );
    }
}

/// 显式丢弃：删除整个会话目录（含段与清单）。不存在 → `NotFound`。
pub fn discard_session(session_dir: &Path) -> Result<(), RecoveryError> {
    if !session_dir.is_dir() {
        return Err(RecoveryError::NotFound {
            dir: session_dir.to_path_buf(),
        });
    }
    std::fs::remove_dir_all(session_dir).map_err(|e| {
        fail(format!(
            "删除会话目录失败（{}）：{e}",
            session_dir.display()
        ))
    })
}

/// 会话目录新建（录制路径用）：创建 `<output_dir>/.screenrec_tmp/<session_id>`。
pub(crate) fn create_session_dir(
    output_dir: &Path,
    session_id: &str,
) -> Result<PathBuf, RecoveryError> {
    let dir = session_tmp_dir(output_dir, session_id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| fail(format!("创建会话临时目录失败（{}）：{e}", dir.display())))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生成一个能通过收尾同口径校验（ffprobe 可解析、时长 > 0）的最小 MKV。
    fn write_valid_final(path: &Path) {
        let ffmpeg = crate::sidecar::ffmpeg_exe().expect("定位锁定 ffmpeg");
        let st = Command::new(ffmpeg)
            .args([
                "-hide_banner",
                "-nostdin",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=64x64:r=10:d=0.2",
                "-c:v",
                "mpeg4",
                "-f",
                "matroska",
            ])
            .arg(path)
            .status()
            .expect("spawn 锁定 ffmpeg 生成测试成品");
        assert!(st.success(), "生成测试成品失败（{st}）");
    }

    /// M14c 单元测试夹具：`<temp>/.screenrec_tmp/sess-1` + output_dir 根下已提交成品。
    /// 成品由锁定 ffmpeg 生成（通过真实校验）；收尾走「清单已记成品且仍存在」幂等分支，
    /// 无需实际执行 concat。
    fn temp_final_session(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "sr_m14c_recovery_cleanup_{tag}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let session = root.join(SESSION_TMP_DIRNAME).join("sess-1");
        std::fs::create_dir_all(&session).expect("创建会话目录");
        let final_path = root.join("20261018_120000_Target.mkv");
        write_valid_final(&final_path);
        std::fs::write(session.join("out0_00000.mkv"), b"segment-bytes").expect("写原段占位");
        let manifest = SessionManifest {
            version: 1,
            session_id: "sess-1".into(),
            created_at_unix_ms: 0,
            start_qpc: 0,
            routes: vec![ManifestRoute {
                route_id: "route-1".into(),
                identity: RouteIdentity {
                    pid: 0,
                    created_at_unix_ms: 0,
                    name: "Target".into(),
                    path: PathBuf::from("test"),
                },
                streams: vec!["v".into(), "a".into()],
                segments: Vec::new(),
                state: "final".into(),
                final_path: Some(final_path.clone()),
            }],
        };
        save_manifest_atomic(&session, &manifest).expect("写会话清单");
        (root, session, final_path)
    }

    #[test]
    fn recovery_finalize_cleanup_removes_session_dir_by_default() {
        let (root, session, final_path) = temp_final_session("default");
        let outcome = finalize_session(&session).expect("M14c：幂等收尾基线应成功");
        assert_eq!(outcome.route_finals.len(), 1);
        assert!(
            final_path.exists(),
            "清理只删会话目录，成品必须保留（{}）",
            final_path.display()
        );
        assert!(
            !session.exists(),
            "M14c 默认：恢复收尾成功后应递归删除会话目录（{}）",
            session.display()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn recovery_finalize_rejects_corrupt_recorded_final_and_keeps_session() {
        let (root, session, final_path) = temp_final_session("corrupt");
        std::fs::write(&final_path, b"").expect("写零字节成品");
        let segment = session.join("out0_00000.mkv");

        let err = finalize_session(&session)
            .expect_err("清单已记成品但成品零字节：不得判收尾成功，更不得清理原段");
        let detail = match &err {
            RecoveryError::RecoveryFailed { detail } => detail.clone(),
            other => panic!("应报 RecoveryFailed（得到 {other:?}）"),
        };
        assert!(
            detail.contains("成品校验失败"),
            "错误应可见地指出成品校验失败（得到 {detail}）"
        );
        assert!(
            detail.contains(&final_path.display().to_string()),
            "错误应包含损坏成品路径（得到 {detail}）"
        );
        assert!(
            session.exists() && session.join(MANIFEST_FILE).exists(),
            "校验失败必须保留会话目录/清单（{}）",
            session.display()
        );
        assert!(
            segment.exists(),
            "校验失败必须保留原段（{}）",
            segment.display()
        );
        assert_eq!(
            std::fs::metadata(&final_path).expect("成品元数据").len(),
            0,
            "损坏成品不得被静默改写/删除"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn recovery_finalize_cleanup_keeps_session_dir_when_requested() {
        let (root, session, final_path) = temp_final_session("keep");
        let outcome =
            finalize_session_with_options(&session, true).expect("M14c：keep=true 收尾应成功");
        assert_eq!(outcome.route_finals[0].1, final_path);
        assert!(
            session.join(MANIFEST_FILE).exists(),
            "M14c keep_segments=true：会话目录/清单应保留（{}）",
            session.display()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn recovery_finalize_cleanup_retains_wav_only_session() {
        let root = std::env::temp_dir().join(format!(
            "sr_m14c_recovery_cleanup_wav_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let session = root.join(SESSION_TMP_DIRNAME).join("sess-wav");
        std::fs::create_dir_all(&session).expect("创建会话目录");
        let wav = session.join("crash.wav");
        let mut writer = sr_audio::wav::WavWriter::create(&wav, 48_000, 2).expect("创建 WAV 写器");
        writer.write_silence_frames(480).expect("写 WAV 数据");
        writer.finalize().expect("封口 WAV");
        drop(writer); // 关闭写句柄（活动探测要求独占打开成功）
        let manifest = SessionManifest {
            version: 1,
            session_id: "sess-wav".into(),
            created_at_unix_ms: 0,
            start_qpc: 0,
            routes: vec![ManifestRoute {
                route_id: "route-wav".into(),
                identity: RouteIdentity {
                    pid: 0,
                    created_at_unix_ms: 0,
                    name: "Wav".into(),
                    path: PathBuf::from("test"),
                },
                streams: vec!["wav".into()],
                segments: Vec::new(),
                state: "recording".into(),
                final_path: None,
            }],
        };
        save_manifest_atomic(&session, &manifest).expect("写会话清单");

        let outcome = finalize_session_with_options(&session, false).expect("WAV 残留会话应可收尾");
        assert_eq!(outcome.repaired_wavs, vec![wav.clone()]);
        assert!(outcome.route_finals.is_empty());
        assert!(
            wav.exists(),
            "M14c：WAV 恢复的成品在会话目录内，默认策略不得删除（{}）",
            wav.display()
        );
        assert!(
            session.exists(),
            "M14c：WAV 会话不得被恢复清理删除（{}）",
            session.display()
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
