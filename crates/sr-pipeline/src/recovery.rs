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
//!   可立即重试收尾，m8-red §3.2-3）。M17：收尾事务全程另持会话级占用（进程内注册表 +
//!   会话目录 `.finalize.lock` 独占句柄），覆盖 concat/remux/`.tmp` 阶段；失败立即释放。
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

use sr_core::redact::{render_path, render_tool_error};

use crate::naming::{next_available, sanitize_target_name, session_tmp_dir, SESSION_TMP_DIRNAME};

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
            render_path(&tmp)
        ))
    })?;
    let target = dir.join(MANIFEST_FILE);
    std::fs::rename(&tmp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        fail(format!(
            "manifest 原子替换失败（{}）：{e}",
            render_path(&target)
        ))
    })
}

/// 读取并解析会话清单；损坏/缺失 → [`RecoveryError::RecoveryFailed`]。
pub fn load_manifest(dir: &Path) -> Result<SessionManifest, RecoveryError> {
    let path = dir.join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| fail(format!("读 manifest 失败（{}）：{e}", render_path(&path))))?;
    manifest_from_json(&text)
        .map_err(|e| fail(format!("manifest 解析失败（{}）：{e}", render_path(&path))))
}

// ─────────────────────────── 段分类 / 活动探测 ───────────────────────────

fn ffprobe_path() -> Result<PathBuf, String> {
    // RecoveryFailed detail 出口：错误文本在构造点脱敏（M19c REQ-3）。
    crate::sidecar::ffprobe_exe().map_err(|e| render_tool_error(&e))
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
///
/// M17/REQ-17：收尾事务全程持有会话级占用（进程内注册表 + 会话目录占用标记以
/// share_mode 0 打开）；活动探测以该占用为准，覆盖 `.tmp`/remux 阶段——不得仅凭
/// PID 存活或 `.tmp` 存在判断。
fn session_is_active(dir: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    if occupied_registry()
        .lock()
        .map(|set| set.contains(&occupancy_key(dir)))
        .unwrap_or(false)
    {
        return true;
    }
    let marker = dir.join(OCCUPANCY_MARKER);
    if marker.exists() {
        match std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&marker)
        {
            Ok(_) => {} // 残余/已释放标记：不判活动
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return true, // 其它进程正在收尾（独占句柄）
        }
    }
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

// ─────────────────────── M17/REQ-17：会话级占用（正常/恢复共用） ───────────────────────

/// 会话目录内占用标记文件名（独占句柄；崩溃后句柄释放 → 残余标记不判活动）。
pub(crate) const OCCUPANCY_MARKER: &str = ".finalize.lock";

/// 占用失败（同进程/跨进程已有收尾事务持有）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OccupancyError {
    AlreadyActive,
}

fn occupied_registry() -> &'static std::sync::Mutex<HashSet<PathBuf>> {
    static REGISTRY: std::sync::OnceLock<std::sync::Mutex<HashSet<PathBuf>>> =
        std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| std::sync::Mutex::new(HashSet::new()))
}

fn occupancy_key(dir: &Path) -> PathBuf {
    dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf())
}

/// 会话级占用守卫：构造即持有（注册表 + 独占标记）；Drop 立即释放（允许失败后重试）。
pub(crate) struct SessionOccupancy {
    key: PathBuf,
    marker: PathBuf,
    file: Option<std::fs::File>,
}

impl SessionOccupancy {
    pub(crate) fn acquire(dir: &Path) -> Result<Self, OccupancyError> {
        use std::os::windows::fs::OpenOptionsExt;
        let key = occupancy_key(dir);
        {
            let mut set = occupied_registry().lock().expect("会话占用注册表");
            if !set.insert(key.clone()) {
                return Err(OccupancyError::AlreadyActive);
            }
        }
        let marker = dir.join(OCCUPANCY_MARKER);
        match std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .share_mode(0)
            .open(&marker)
        {
            Ok(file) => Ok(Self {
                key,
                marker,
                file: Some(file),
            }),
            Err(_) => {
                if let Ok(mut set) = occupied_registry().lock() {
                    set.remove(&key);
                }
                Err(OccupancyError::AlreadyActive)
            }
        }
    }
}

impl Drop for SessionOccupancy {
    fn drop(&mut self) {
        // 先释放独占句柄，再移除标记（删除失败仅残留无害标记）。
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.marker);
        if let Ok(mut set) = occupied_registry().lock() {
            set.remove(&self.key);
        }
    }
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
            .map_err(|e| fail(format!("WAV 封口失败（{}）：{e:?}", render_path(&wav))))?;
        repaired.push(wav);
    }
    Ok(repaired)
}

/// M19a：恢复成品 stem。
///
/// `include_window_title == false`（设置默认）→ `{timestamp}_recovered-{sanitize(route_id)}`
/// （不含旧窗口标题）；`true` → `{timestamp}_{sanitize(identity_name)}`（旧行为）。
pub fn recovery_output_stem(
    identity_name: &str,
    route_id: &str,
    include_window_title: bool,
    timestamp: &str,
) -> String {
    if include_window_title {
        format!("{timestamp}_{}", sanitize_target_name(identity_name))
    } else {
        format!("{timestamp}_recovered-{}", sanitize_target_name(route_id))
    }
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
    finalize_session_with_options_and_container(
        session_dir,
        keep_segments,
        crate::naming::OutputContainer::Mkv,
    )
}

/// M17/REQ-18：崩溃收尾 + 成品容器（未完成路由按此容器产出；已有有效 `final_path`
/// 幂等复用，不因设置变化再生成另一容器）。窗口标题隐私开关传 `true`，旧行为不变。
pub fn finalize_session_with_options_and_container(
    session_dir: &Path,
    keep_segments: bool,
    container: crate::naming::OutputContainer,
) -> Result<RecoveryOutcome, RecoveryError> {
    finalize_session_with_options_container_and_privacy(session_dir, keep_segments, container, true)
}

/// M19a：崩溃收尾 + 容器 + 窗口标题隐私开关。
///
/// `include_window_title == false`（设置默认）：未完成路由的成品名使用通用
/// `{timestamp}_recovered-{route_id}`，不含旧 manifest 的窗口标题；`true`：保留旧行为
/// `{timestamp}_{sanitize(identity.name)}`。既有有效 `final_path` 仍幂等复用。
pub fn finalize_session_with_options_container_and_privacy(
    session_dir: &Path,
    keep_segments: bool,
    container: crate::naming::OutputContainer,
    include_window_title: bool,
) -> Result<RecoveryOutcome, RecoveryError> {
    finalize_session_impl(session_dir, keep_segments, container, include_window_title)
}

/// 既有成品容器（P1-3：按扩展名推断，复用前校验容器族与声明一致）。
fn existing_final_container(path: &Path) -> Option<crate::naming::OutputContainer> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "mkv" => Some(crate::naming::OutputContainer::Mkv),
        "mp4" => Some(crate::naming::OutputContainer::Mp4),
        _ => None,
    }
}

fn finalize_session_impl(
    session_dir: &Path,
    keep_segments: bool,
    container: crate::naming::OutputContainer,
    include_window_title: bool,
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
    let occupancy = SessionOccupancy::acquire(session_dir).map_err(|_| RecoveryError::Active {
        session_id: manifest.session_id.clone(),
    })?;
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
        // P1-3：期望轨数（清单 streams）+ 既有成品容器族（扩展名）都必须匹配；
        // 缺轨/错容器替代文件不得视为成功，避免默认清理删除原段。
        if let Some(f) = &route.final_path {
            if f.exists() {
                let verified = if route.streams.iter().any(|s| s == "wav") {
                    crate::finalize::verify_final_media(&ffprobe, f)
                } else {
                    match existing_final_container(f) {
                        Some(container) => crate::finalize::verify_final_media_with_container(
                            &ffprobe,
                            f,
                            crate::finalize::MediaExpectation::from_streams(&route.streams),
                            container,
                        ),
                        None => Err(format!(
                            "成品扩展名不受支持（{}；无法确定容器族）",
                            render_path(f)
                        )),
                    }
                };
                verified.map_err(|e| {
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
        // MKV 路由：目录重扫（陈旧清单并集）→ 统一收尾事务（M17：按容器 remux）。
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
                    render_path(session_dir)
                ),
            });
        }
        let stem = recovery_output_stem(
            &route.identity.name,
            &route.route_id,
            include_window_title,
            &crate::naming::format_timestamp(
                crate::route::current_unix_ms(),
                crate::route::local_utc_offset_minutes(),
            ),
        );
        let final_path = next_available(&output_root, &stem, container.extension());
        let list_path = session_dir.join(format!(".finalize.{}.ffconcat", route.route_id));
        let job = crate::finalize::FinalizeJob {
            session_dir,
            output_dir: output_root.clone(),
            final_path: &final_path,
            list_path: &list_path,
            segments: &sealed,
            expected: crate::finalize::MediaExpectation::from_streams(&route.streams),
            container,
        };
        crate::finalize::run_finalize_transaction(&job, &crate::route::FfmpegRunner, None)
            .map_err(|e| fail(e))?;
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
    // M14c：全部路由收尾成功（成品已由 safe_commit 校验；WAV 已就地封口）→
    // 按保留策略清理会话目录。WAV 恢复的成品在会话目录内，不能清理。
    // 清理前释放占用（删除会话目录时占用标记不得仍被打开）。
    drop(occupancy);
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
            crate::sidecar::render_path_for_stderr(session_dir)
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
            render_path(session_dir)
        ))
    })
}

/// 会话目录新建（录制路径用）：创建 `<output_dir>/.screenrec_tmp/<session_id>`。
pub(crate) fn create_session_dir(
    output_dir: &Path,
    session_id: &str,
) -> Result<PathBuf, RecoveryError> {
    let dir = session_tmp_dir(output_dir, session_id);
    std::fs::create_dir_all(&dir).map_err(|e| {
        fail(format!(
            "创建会话临时目录失败（{}）：{e}",
            render_path(&dir)
        ))
    })?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生成一个能通过收尾同口径校验（容器族/期望轨数 v+a/时长 > 0）的最小 MKV。
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
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=0.2",
                "-map",
                "0:v",
                "-map",
                "1:a",
                "-c:v",
                "mpeg4",
                "-c:a",
                "aac",
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
    fn recovery_finalize_failure_releases_occupancy_for_immediate_retry() {
        use crate::naming::OutputContainer;
        let root =
            std::env::temp_dir().join(format!("sr_m17_recovery_retry_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let session = root.join(SESSION_TMP_DIRNAME).join("sess-empty");
        std::fs::create_dir_all(&session).expect("创建会话目录");
        let segment = session.join("out0_00000.mkv");
        std::fs::write(&segment, b"").expect("写 0 字节段");
        let manifest = SessionManifest {
            version: 1,
            session_id: "sess-empty".into(),
            created_at_unix_ms: 0,
            start_qpc: 0,
            routes: vec![ManifestRoute {
                route_id: "route-1".into(),
                identity: RouteIdentity {
                    pid: 0,
                    created_at_unix_ms: 0,
                    name: "Empty".into(),
                    path: PathBuf::from("test"),
                },
                streams: vec!["v".into()],
                segments: Vec::new(),
                state: "recording".into(),
                final_path: None,
            }],
        };
        save_manifest_atomic(&session, &manifest).expect("写会话清单");

        let first =
            finalize_session_with_options_and_container(&session, true, OutputContainer::Mp4)
                .expect_err("无可收尾段必须显式失败");
        assert!(
            matches!(first, RecoveryError::NoUsableSegments { .. }),
            "应报 NoUsableSegments（得到 {first:?}）"
        );
        // 失败后占用必须立即释放：第二次不得报 Active（同进程可立即重试）。
        let second =
            finalize_session_with_options_and_container(&session, true, OutputContainer::Mp4)
                .expect_err("重试仍应因无可收尾段失败");
        assert!(
            matches!(second, RecoveryError::NoUsableSegments { .. }),
            "REQ-17：失败后应立即释放占用（得到 {second:?}）"
        );
        assert!(segment.exists(), "失败路径不得清理原段");
        assert!(
            !session.join(OCCUPANCY_MARKER).exists(),
            "失败路径应移除占用标记"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // ─────────────────── M19a：恢复命名隐私（默认不含旧标题） ───────────────────

    #[test]
    fn recovery_output_stem_default_does_not_include_identity_title() {
        let stem = recovery_output_stem("Secret Window Title", "route-7", false, "20261023_120000");
        assert_eq!(stem, "20261023_120000_recovered-route-7");
        assert!(
            !stem.contains("Secret Window Title"),
            "M19a：默认 false 不得包含旧标题（得到 {stem:?}）"
        );
    }

    #[test]
    fn recovery_output_stem_true_keeps_sanitized_identity_title() {
        let title = "Secret/Window:Title";
        let stem = recovery_output_stem(title, "route-7", true, "20261023_120000");
        assert_eq!(
            stem,
            format!("20261023_120000_{}", sanitize_target_name(title))
        );
        assert!(
            stem.contains("Secret_Window_Title"),
            "M19a：true 应保留（sanitize 后的）旧标题（得到 {stem:?}）"
        );
    }

    #[test]
    fn recovery_old_v1_manifest_with_title_identity_still_parses() {
        // 旧 v1 manifest（identity.name 含窗口标题、无任何 M19a 新字段）必须仍可解析；
        // 收尾/幂等路径由既有 temp_final_session/recorded_final_session 测试覆盖。
        let json = r#"{"version":1,"session_id":"sess-old","created_at_unix_ms":1,"start_qpc":2,"routes":[{"route_id":"route-1","identity":{"pid":1,"created_at_unix_ms":2,"name":"Old Window Title","path":"C:\\app.exe"},"streams":["v"],"segments":[],"state":"recording","final_path":null}]}"#;
        let m = manifest_from_json(json).expect("旧 v1 manifest（含标题）必须仍可解析");
        assert_eq!(m.version, 1);
        assert_eq!(m.session_id, "sess-old");
        assert_eq!(m.routes.len(), 1);
        assert_eq!(m.routes[0].identity.name, "Old Window Title");
        assert!(m.routes[0].final_path.is_none());
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
        // M19c REQ-6：绝对路径断言改为「basename 存在、父目录不存在」——校验对象
        // （正确的成品名）意图不变，仅不再暴露父目录/用户名。
        assert!(
            detail.contains("20261018_120000_Target.mkv"),
            "错误应包含损坏成品 basename（得到 {detail}）"
        );
        assert!(
            !detail.contains(&root.to_string_lossy().to_string()),
            "默认错误不得包含成品父目录/绝对路径（得到 {detail}）"
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

    /// M19c REQ-2/3：manifest 读取失败 detail 只保留 basename（与 `.screenrec_tmp`
    /// 会话后缀），不含父目录/用户名。
    #[test]
    fn recovery_manifest_error_detail_is_redacted() {
        let root =
            std::env::temp_dir().join(format!("sr_m19c_recovery_redact_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let session = root.join(SESSION_TMP_DIRNAME).join("sess-redact");
        std::fs::create_dir_all(&session).expect("创建会话目录");

        let err = load_manifest(&session).expect_err("缺失 manifest 应报 RecoveryFailed");
        let detail = match &err {
            RecoveryError::RecoveryFailed { detail } => detail.clone(),
            other => panic!("应报 RecoveryFailed（得到 {other:?}）"),
        };
        assert!(
            detail.contains("读 manifest 失败"),
            "归因保留（得到 {detail}）"
        );
        assert!(
            detail.contains("manifest.json"),
            "basename 保留（得到 {detail}）"
        );
        assert!(
            detail.contains(".screenrec_tmp"),
            "会话目录后缀保留（得到 {detail}）"
        );
        assert!(
            !detail.contains(&root.to_string_lossy().to_string()),
            "父目录/用户名不得出现在默认错误（得到 {detail}）"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 仅视频的最小 MKV（缺轨负例）。
    fn write_video_only_mkv(path: &Path) {
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
            .expect("spawn 锁定 ffmpeg 生成仅视频成品");
        assert!(st.success(), "生成仅视频成品失败（{st}）");
    }

    /// 有效 MP4 成品（v+a；跨设置幂等复用正例）。
    fn write_valid_mp4(path: &Path) {
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
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=0.2",
                "-map",
                "0:v",
                "-map",
                "1:a",
                "-c:v",
                "mpeg4",
                "-c:a",
                "aac",
                "-f",
                "mp4",
            ])
            .arg(path)
            .status()
            .expect("spawn 锁定 ffmpeg 生成 MP4 成品");
        assert!(st.success(), "生成 MP4 成品失败（{st}）");
    }

    /// 清单已记成品的最小会话（成品由调用方生成；原段占位用于断言保留）。
    fn recorded_final_session(
        tag: &str,
        final_name: &str,
        streams: &[&str],
    ) -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "sr_m17c_recovery_idem_{tag}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let session = root.join(SESSION_TMP_DIRNAME).join("sess-1");
        std::fs::create_dir_all(&session).expect("创建会话目录");
        let final_path = root.join(final_name);
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
                streams: streams.iter().map(|s| s.to_string()).collect(),
                segments: Vec::new(),
                state: "final".into(),
                final_path: Some(final_path.clone()),
            }],
        };
        save_manifest_atomic(&session, &manifest).expect("写会话清单");
        (root, session, final_path)
    }

    #[test]
    fn recovery_finalize_rejects_final_missing_declared_track_and_keeps_segment() {
        let (root, session, final_path) =
            recorded_final_session("missing_track", "20261018_120000_Target.mkv", &["v", "a0"]);
        write_video_only_mkv(&final_path);
        let segment = session.join("out0_00000.mkv");

        let err = finalize_session(&session)
            .expect_err("P1-3：清单声明 a0 但成品缺音轨：不得判成功，更不得清理原段");
        let detail = match &err {
            RecoveryError::RecoveryFailed { detail } => detail.clone(),
            other => panic!("应报 RecoveryFailed（得到 {other:?}）"),
        };
        assert!(
            detail.contains("轨数不符"),
            "错误应指出期望轨数不符（得到 {detail}）"
        );
        assert!(
            session.exists() && segment.exists(),
            "缺轨负例必须保留会话目录/原段（{}）",
            session.display()
        );
        assert!(
            final_path.exists(),
            "既有成品不得被删除/改写（{}）",
            final_path.display()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn recovery_finalize_rejects_wrong_container_family_and_keeps_segment() {
        let (root, session, final_path) =
            recorded_final_session("wrong_container", "20261018_120000_Target.mp4", &["v"]);
        write_video_only_mkv(&final_path); // 扩展名 .mp4，内容却是 matroska
        let segment = session.join("out0_00000.mkv");

        let err = finalize_session(&session)
            .expect_err("P1-3：扩展名/容器族不符：不得判成功，更不得清理原段");
        let detail = match &err {
            RecoveryError::RecoveryFailed { detail } => detail.clone(),
            other => panic!("应报 RecoveryFailed（得到 {other:?}）"),
        };
        assert!(
            detail.contains("容器族不符"),
            "错误应指出容器族不符（得到 {detail}）"
        );
        assert!(
            session.exists() && segment.exists(),
            "错容器负例必须保留会话目录/原段（{}）",
            session.display()
        );
        assert!(
            final_path.exists(),
            "既有成品不得被删除/改写（{}）",
            final_path.display()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn recovery_finalize_reuses_valid_existing_container_across_setting_change() {
        use crate::naming::OutputContainer;
        let (root, session, final_path) =
            recorded_final_session("cross_setting", "20261018_120000_Target.mp4", &["v", "a0"]);
        write_valid_mp4(&final_path);
        let final_bytes = std::fs::read(&final_path).expect("读既有成品");

        let outcome =
            finalize_session_with_options_and_container(&session, true, OutputContainer::Mkv)
                .expect("P1-3：既有有效 MP4 应跨设置变化（当前 MKV）幂等复用");
        assert_eq!(outcome.route_finals.len(), 1);
        assert_eq!(
            outcome.route_finals[0].1, final_path,
            "应复用既有 MP4，不因设置变化再生成另一容器"
        );
        assert_eq!(
            std::fs::read(&final_path).expect("复用后读成品"),
            final_bytes,
            "幂等复用不得改写既有成品"
        );
        let media_count = std::fs::read_dir(&root)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| {
                        e.path().extension().map_or(false, |x| {
                            x.eq_ignore_ascii_case("mp4") || x.eq_ignore_ascii_case("mkv")
                        })
                    })
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(media_count, 1, "不得生成第二容器成品");
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
