# M8 红测试报告（TDD 红阶段：探测/降级、许可审计、恢复状态机（完整）、命名收尾 REQ-28/29/22/24）

- 日期: 2026-10-17
- 规格: specs/001-screen-recorder.md §2.5（REQ-22 收尾完整 / REQ-24 命名·目录·提交）、§2.7（REQ-28 探测与降级）、§2.8（REQ-29 许可与构建）、§3.2（模块划分）、§3.3（probe_capabilities / recover_sessions / finalize_session / discard_session 命令）、§3.4（SessionManifest v1 / 错误码 E_DISK_FULL、E_RECOVERY_FAILED 等）、§3.5（崩溃残留 / 磁盘不足行）、§4（REQ-22/24/28/29 行）、§5 M8 行、§8 M0 冻结项（S2/S3/S5）
- Spike 依据: docs/spikes/s5/s5-crash-recovery.md（段/尾段/concat/锁/满盘启动期实测与恢复规则）、docs/spikes/s2/s2-encoder-matrix.md（LGPL 构建 buildconf/许可结论）、docs/spikes/s3/s3-audio-isolation.md（激活/隐私类语义）
- 先例: docs/milestones/m6-red-report.md、m7-red-report.md（契约/门/双层自检方法论）；tests/sr-itests/tests/{multi_route,audio_m6,merge}.rs 与 common/mod.rs（helper 形态/`#[ignore]`/BLOCKED=fail/GATE）
- 角色: **executor-backup 承担 Test Writer（红阶段）**（GLM 系 test-writer 通道额度不可用）。本任务**只写测试**：生产 src 零改动（crates/*、src-tauri/* 全部逐字未动）；未 git 提交。
- 环境实测: Windows 11 25H2，`Microsoft Windows [Version 10.0.26200.9711]`；cargo/rustc **1.99.0** msvc；锁定 ffmpeg n8.1 release（`tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/`，zip SHA256 `337B85465B9CD0CFCB3376B84988912C3999E3FB69BC025F0B4360B4B1C8C240`）。仓库无 git 提交历史（全部 untracked，与 M1–M7 一致）。
- 会话注记: 工具 shell 不继承 `SESSIONNAME`（L2 需 `$env:SESSIONNAME='Console'`）；icacls 注入需 `USERNAME`（本机 `elvis`）；cargo-deny **未安装**（审计用例按其缺失走 BLOCKED，见 §6.6/§8-1）。
- 到场状态: 本次改动前基线 `cargo test --workspace` = **exit 0**（`%TEMP%\opencode\m8_baseline_ws.log`）；M7 已达成（spec §5 状态行）。

## 1. 交付物

| 文件 | 性质 | 内容 |
|---|---|---|
| `crates/sr-pipeline/tests/safe_commit.rs` | 新增（L1 ×2） | `unit_session_tmp_dir_contract` / `unit_safe_commit_no_overwrite_and_verify`（REQ-24 D12：`.screenrec_tmp/<session_id>` 纯函数布局；安全提交=临时名→验证→rename、禁覆盖、失败保留临时文件、可重试、跨卷 Io） |
| `crates/sr-pipeline/tests/license_and_build_audit.rs` | 新增（L1 ×1） | `license_and_build_audit`（§4 点名；REQ-29：工具链锁定 + cargo-deny 配置/白名单 + 锁定构建实际审计（version/buildconf/zip SHA256/LICENSE.txt）+ 来源记录检索 + `cargo deny check licenses`） |
| `tests/sr-itests/tests/probe.rs` | 新增（L2 ×1） | `it_probe_permission_matrix_injected`（§4 点名；REQ-28：可注入故障钩子矩阵 + 降级归类 + 真实探测 smoke；GATE） |
| `tests/sr-itests/tests/recovery.rs` | 新增（L2 ×2） | `it_crash_kill_matrix_recovery_state_machine`（§4 点名；REQ-22：合成残留矩阵 + 真实子进程强杀矩阵 + Job 回收 + 活动文件排除 + 幂等 + 丢弃）、`it_fault_injection_disk_full`（§4 点名；REQ-22 磁盘写失败注入 + E_DISK_FULL + 原段保留 + 可重试收尾；GATE） |
| `docs/milestones/m8-red-report.md` | 新增 | 本报告 |

未改动：生产 src（含 sr-pipeline/lib.rs 的 `recovery` 骨架与命名/审计面全部零改动）、`specs/`、`docs/spikes/`、既有测试文件与 `common/mod.rs`（helper 零改动——本红 helper 全部内聚新文件）、`tests/sr-itests/Cargo.toml`、根 `Cargo.toml`、`Cargo.lock`（**零新依赖、零 manifest 改动**）。

## 2. REQ → 测试 → 文件映射（§4 命名优先，无自创用例名）

| REQ | 测试名 | 层 | 断言要点 |
|---|---|---|---|
| REQ-28 | `it_probe_permission_matrix_injected`（§4 点名） | L2 | 注入矩阵：OS 过旧→`os_supported=false`；编码器逐个失败→链序保留、`selected_encoder`=首个 init_ok、全败 None（不 panic）；loopback/mic/wgc 三类**分别**归类（Timeout≠Denied≠Canceled≠Unavailable≠Failed；不把超时归因权限）；画布上限透传。真实 smoke：结构性不变量（OS build 非空/支持、D4 链 4 候选序、selected 与 init 一致、明细非空）。 |
| REQ-29 | `license_and_build_audit`（§4 点名） | L1 | 工具链锁定 `channel="1.99.0"`；`deny.toml` 存在且 `[licenses] allow` 含 MIT/Apache-2.0、无 GPL 系；锁定构建**实际运行**：版本串含标记、zip SHA256 与事实一致、buildconf 必含项全中（version3/GPL 组件 disable）、禁用编码器零命中、LICENSE.txt=LGPL v3；docs/ 中存在 zip SHA256 来源记录；`cargo deny check licenses` exit 0（工具缺失→BLOCKED）。 |
| REQ-22 | `it_crash_kill_matrix_recovery_state_machine`（§4 点名） | L2 | manifest 原子写/往返/无临时残留；陈旧清单→目录重扫（完成段不丢）；封口判定（0 字节/坏 EBML 排除；末可解析段=尾段）；幂等收尾（二次无操作、成品字节不变、无第二成品）；原段永不删除；清单损坏/缺失→`RecoveryFailed` 且保留原段、可显式丢弃；<5s 短封口段保留；WAV 崩溃残留就地封口并入清单；**真实子进程**：主进程强杀→Job 对象连带回收 sidecar（孤儿=0）、活动文件排除（活跃会话拒绝收尾）、崩溃收尾成品可播；**编码进程**被杀→E_SIDECAR_EXIT→同进程恢复收尾；`discard_session` 隔离与 `NotFound`。 |
| REQ-22（磁盘满） | `it_fault_injection_disk_full`（§4 点名） | L2 | 录制中途对会话目录 `icacls /deny (OI)(CI)W`（ENOSPC 类写失败注入）→ 该路 `Err(DiskFull)`（E_DISK_FULL）；封口段全部保留且独立可读时长；解除注入后恢复收尾成功（可重试）、成品可播、原段不删。 |
| REQ-24（剩余面） | `unit_session_tmp_dir_contract`；`unit_safe_commit_no_overwrite_and_verify` | L1 | 会话临时目录=输出目录内 `.screenrec_tmp/<session_id>`（纯函数）；安全提交顺序语义（FinalExists→verify 不调用；VerifyFailed→成品不出现、临时文件保留；成功→rename、字节一致、纯分配器避开成品；跨卷→Io）。§4 的 `unit_naming_sanitize_no_overwrite` 中 sanitize/禁覆盖基础面已由 M3 `output_naming` 承载，本红补其未覆盖的 D12 面（§7-R1）。 |

## 3. API 契约清单（测试即契约；供绿阶段实现）

### 3.1 `crates/sr-pipeline` —— `pub mod probe`（新增模块；REQ-28）

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerStatus { Ok, Denied, Timeout, Canceled, Unavailable, Failed }

#[derive(Clone, Debug)]
pub struct LayerReport { pub status: LayerStatus, pub detail: String }

#[derive(Clone, Debug)]
pub struct EncoderProbe { pub name: String, pub init_ok: bool, pub detail: String }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanvasLimits { pub max_width: u32, pub max_height: u32 }

#[derive(Clone, Debug)]
pub struct ProbeCapabilities {
    pub os_build: String,               // 如 "10.0.26200.9711"
    pub os_supported: bool,             // 最低支持阈值实现定（17763=false / 26200=true 已钉）
    pub encoders: Vec<EncoderProbe>,    // D4 链序（nvenc→qsv→amf→mf）
    pub selected_encoder: Option<String>, // 链序首个 init_ok；全败 None（§9 T1 明确报错，不静默降级）
    pub loopback: LayerReport,          // 音频激活类（REQ-28 三分）——超时/取消独立归类
    pub mic: LayerReport,               // 麦克风隐私类
    pub wgc: LayerReport,               // WGC 授权类
    pub canvas_limits: CanvasLimits,
}

#[derive(Clone, Debug, Default)]
pub struct ProbeInjections {            // 可注入故障钩子（测试/预演）：Some=整层替身
    pub os_build: Option<String>,
    pub encoder_init: Option<Vec<(String, bool)>>, // 链序；整体替换
    pub loopback: Option<LayerStatus>,
    pub mic: Option<LayerStatus>,
    pub wgc: Option<LayerStatus>,
    pub canvas_limits: Option<CanvasLimits>,
}

pub fn probe_capabilities() -> ProbeCapabilities;                        // 真实分层探测
pub fn probe_capabilities_injected(inj: &ProbeInjections) -> ProbeCapabilities;
```

### 3.2 `crates/sr-pipeline` —— `pub mod recovery`（骨架 TODO(M8) 落地；REQ-22）

```rust
pub const MANIFEST_FILE: &str = "manifest.json";        // 会话目录内 manifest 文件名（钉死）

#[derive(Clone, Debug)]
pub struct RouteIdentity { pub pid: u32, pub created_at_unix_ms: u64, pub name: String, pub path: PathBuf }

#[derive(Clone, Debug)]
pub struct ManifestSegment { pub path: PathBuf, pub t_start: f64, pub t_end: f64, pub closed: bool }

#[derive(Clone, Debug)]
pub struct ManifestRoute {
    pub route_id: String,
    pub identity: RouteIdentity,        // §3.4（身份仅作展示/诊断，不作可收尾性判据——见下「活动判定」）
    pub streams: Vec<String>,
    pub segments: Vec<ManifestSegment>,
    pub state: String,
    pub final_path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct SessionManifest { pub version: u32 /*=1*/, pub session_id: String, pub created_at_unix_ms: u64, pub start_qpc: i64, pub routes: Vec<ManifestRoute> }

pub fn save_manifest_atomic(dir: &Path, m: &SessionManifest) -> Result<(), RecoveryError>; // 临时文件+rename；成功后无残留；可覆盖更新
pub fn load_manifest(dir: &Path) -> Result<SessionManifest, RecoveryError>;                 // 损坏/缺失 → RecoveryFailed

#[derive(Clone, Debug)]
pub struct RecoverableSession {
    pub session_id: String,
    pub dir: PathBuf,
    pub manifest: Option<SessionManifest>, // 损坏/缺失 → None（不静默丢弃目录）
    pub sealed_segments: Vec<PathBuf>,     // 可收尾完整段（0 字节/坏 EBML 排除；活动文件排除）
    pub tail_segment: Option<PathBuf>,     // 末可解析段（未封口形态；回收可选，不得当封口段用）
    pub active: bool,                      // 活动文件探测（独占打开失败）；见下
}
pub fn scan_sessions(output_dir: &Path) -> Vec<RecoverableSession>;    // 扫 <output_dir>/.screenrec_tmp/*；非会话目录跳过

#[derive(Clone, Debug)]
pub struct RecoveryOutcome {
    pub route_finals: Vec<(String, PathBuf)>, // route_id → 成品（output_dir 根，REQ-24 命名；不含 WAV 路由）
    pub segments_used: Vec<PathBuf>,
    pub discarded: Vec<PathBuf>,
    pub repaired_wavs: Vec<PathBuf>,          // WAV 崩溃残留就地封口（复用 sr_audio::wav::repair_wav）
}
pub fn finalize_session(session_dir: &Path) -> Result<RecoveryOutcome, RecoveryError>; // 幂等
pub fn discard_session(session_dir: &Path) -> Result<(), RecoveryError>;

#[derive(Debug)]
pub enum RecoveryError {
    NotFound { dir: PathBuf },
    Active { session_id: String },          // 活动文件在写 → 拒绝收尾（不动文件）
    NoUsableSegments { detail: String },
    RecoveryFailed { detail: String },      // E_RECOVERY_FAILED：清单损坏/缺失/收尾校验失败；**保留原段**
}
```

**行为规则（scratch 实测标定，§6.5/§6.6）**：

1. **会话目录**：`<output_dir>/.screenrec_tmp/<session_id>/`（REQ-24/D12）；录制开始时创建并写 manifest；段与 ffconcat 清单在会话目录内，成品仍在 `output_dir` 根（REQ-24 命名 + `next_available` 禁覆盖）。
2. **完成段判定**（S5 §11 规则）：候选=会话目录顶层 `*.mkv`；**有效**=`size>0` 且 ffprobe 可解析；封口 = 有效且（manifest.closed 或存在更高序号的**有效**文件）；**尾段**=最高序号的有效文件且未被 manifest 标 closed；0 字节/坏 EBML 一律排除（不得进封口集合）。
3. **活动判定**（REQ-22「恢复扫描排除活动文件」）：对会话内段/WAV 做**独占打开探测**（Windows `share_mode(0)`；任何在写句柄都会使探测失败）→ `active=true` 且 `finalize_session` 返回 `Err(Active)`。**不得**以 `identity.pid` 存活作为可收尾性判据：同进程内失败（如写失败）后必须立即可重试收尾（§3.5「收尾失败可重试」）——rehearsal 首版用 pid 判定即被该用例否决（§6.6-R2）。
4. **幂等**：成功后 manifest 记录 `final_path`；二次 `finalize_session` 直接复用（成品字节不变、无第二成品）。「收尾中再崩」按此恢复。
5. **清单损坏/缺失**：`load_manifest`→`RecoveryFailed`；`scan_sessions` 仍列出（manifest=None）+ 目录重扫段；`finalize_session` 拒绝（`RecoveryFailed`）且**保留原段**；`discard_session` 为显式丢弃路径。
6. **陈旧清单**（「段完成但清单未更新」）：目录重扫并集，完成段必须参与收尾（S5 §9c 的 concat 静默跳段教训）。
7. **原段永不删除**：成功/失败/活动拒绝路径都不得删改段文件（测试逐字节/存在性核验）。
8. **WAV**：崩溃残留（未封口 RIFF）在 finalize 中就地封口（RIFF/data 长度回填，复用 `repair_wav`），列入 `repaired_wavs`；幂等（二次 repair trimmed=0）。

### 3.3 `crates/sr-pipeline` —— `pub route::RouteError` 增补变体（REQ-22 磁盘写失败）

```rust
/// E_DISK_FULL（§3.4/§3.5）：录制/收尾中输出写失败（磁盘满/写拒绝等 ENOSPC 类）。
/// 实现须扫描**完整 stderr 尾部（16KB）**判定写失败关键字（"Permission denied"/"No space left" 等）；
/// 只取最后 3 行摘要会漏判（rehearsal 实证，§6.6-R1）。该路径=安全停止、保留原段、收尾可重试。
DiskFull { detail: String },
```

### 3.4 `crates/sr-pipeline` —— `pub mod naming` 增补（REQ-24/D12）

```rust
pub const SESSION_TMP_DIRNAME: &str = ".screenrec_tmp";

/// <output_dir>/.screenrec_tmp/<session_id>（纯函数：不创建、不要求存在）。
pub fn session_tmp_dir(output_dir: &Path, session_id: &str) -> PathBuf;

#[derive(Debug)]
pub enum CommitError {
    FinalExists { final_path: PathBuf },   // 禁止覆盖现有文件（verify 不被调用）
    VerifyFailed { detail: String },       // 验证失败：成品不出现、临时文件保留（可修复/重试）
    Io { detail: String },                 // rename/IO 失败（跨卷等；临时文件保留）
}

/// 安全提交：final 已存在 → FinalExists；否则 verify(tmp) → rename(tmp, final)。
pub fn safe_commit<F>(tmp_path: &Path, final_path: &Path, verify: F) -> Result<(), CommitError>
where F: FnOnce(&Path) -> Result<(), String>;
```

### 3.5 `crates/sr-pipeline` —— `pub mod sidecar` 增补（REQ-29 审计）

```rust
pub struct LockedBuildFacts {
    pub version_marker: &'static str,               // 版本串必含（如 "n8.1"）
    pub zip_file_name: &'static str,                // 如 "ffmpeg-n8.1-latest-win64-lgpl-8.1.zip"
    pub zip_sha256: &'static str,                   // 64 位大写十六进制（冻结校验和）
    pub required_buildconf: &'static [&'static str],// ["--enable-version3","--disable-libx264",…]
    pub forbidden_encoder_markers: &'static [&'static str], // ["libx264","libx265","libxavs2","libxvid"]
}
pub fn locked_build_facts() -> &'static LockedBuildFacts;

pub struct BuildAudit {
    pub version: String,                  // 首行版本串
    pub zip_sha256: String,               // 实测压缩包 SHA256（与 facts 比对）
    pub buildconf_hits: Vec<(String, bool)>,
    pub forbidden_hits: Vec<String>,      // 应空
    pub license_text_ok: bool,            // 随包 LICENSE.txt=LGPL v3
}
/// 对解压目录实际运行 -version/-buildconf/-encoders + 读 LICENSE.txt + 计算 zip SHA256。
pub fn audit_locked_build(ffmpeg_home: &Path, zip_path: &Path) -> Result<BuildAudit, String>;
```

**工具缺失语义**：锁定构建/压缩包缺失、或 cargo-deny 未安装 → 审计测试 **BLOCKED（fail，不静默 skip）**；本机 cargo-deny 缺失，绿阶段需安装（需用户授权）或由编排者裁定豁免口径（§8-1）。

## 4. 运行命令与门控

```text
# L1（随默认 cargo test 执行）
cargo test -p sr-pipeline --test safe_commit -- --nocapture
cargo test -p sr-pipeline --test license_and_build_audit -- --nocapture

# L2（显式执行；须交互式用户会话 + 可用编码器 + 锁定 n8.1；$env:SESSIONNAME='Console'）
cargo test -p sr-itests --test probe -- --ignored --nocapture
cargo test -p sr-itests --test recovery -- --ignored --nocapture
cargo test -p sr-itests --test recovery it_crash_kill_matrix_recovery_state_machine -- --ignored --nocapture
cargo test -p sr-itests --test recovery it_fault_injection_disk_full -- --ignored --nocapture

# 全仓（红阶段 exit 101 属预期：仅 4 个新测试目标的预期未解析符号）
cargo test --workspace
```

BLOCKED 触发点：非交互会话（SESSIONNAME）；锁定 ffmpeg/ffprobe 缺失；录制子进程报 EncoderUnavailable/ConsentDenied；icacls 不可用；USERNAME 缺失；cargo-deny 缺失（审计）。文件内 `GATE` Mutex 串行（probe 1 用例、recovery 2 用例；与录制族 L2 同机不混跑）。

## 5. 依赖与锁定构建

- **零新依赖、零 manifest 改动**：4 个新测试文件仅用既有依赖（sr-pipeline/sr-audio/windows 0.62 既有 features；recovery.rs 复用 Toolhelp32 既有 feature）；Cargo.toml / Cargo.lock 与 M7 完全一致（哈希 §6.7）。
- 素材与核验全部走锁定 n8.1 release（lavfi 合成段/WAV 字节构造/ffprobe/解码）；磁盘满注入用系统 `icacls` + `taskkill`（无第三方工具）；`license_and_build_audit` 用 `certutil`（Windows 内置）与 `cargo deny`（缺失→BLOCKED）。

## 6. 红证据（实际执行；全部日志在 `%TEMP%\opencode\`）

### 6.1 改动前基线（对照）

| 命令 | 结果 |
|---|---|
| `cargo test --workspace`（新文件移出后） | **exit 0**（`m8_baseline_ws.log`） |

### 6.2 定向红（新测试目标）

```text
cargo test -p sr-pipeline --test safe_commit --test license_and_build_audit --no-run   → exit 101（m8_red_targeted_pipeline_final2.log）
cargo test -p sr-itests  --test probe --test recovery --no-run                          → exit 101（m8_red_targeted_itests_final2.log）

error[E0432]: unresolved imports `sr_pipeline::naming::safe_commit`, `…::session_tmp_dir`, `…::CommitError`, `…::SESSION_TMP_DIRNAME`          （safe_commit.rs:31）
error[E0432]: unresolved imports `sr_pipeline::sidecar::audit_locked_build`, `…::locked_build_facts`, `…::BuildAudit`, `…::LockedBuildFacts`    （license_and_build_audit.rs:26）
error[E0432]: unresolved import `sr_pipeline::probe`                                                                                          （probe.rs:30）
error[E0432]: unresolved imports `sr_pipeline::recovery::{discard_session, finalize_session, load_manifest, save_manifest_atomic, scan_sessions, ManifestRoute, ManifestSegment, RecoverableSession, RecoveryError, RecoveryOutcome, RouteIdentity, SessionManifest}`（recovery.rs:47）
error[E0599]: no variant named `DiskFull` found for enum `RouteError`                                                                          （recovery.rs:806）
```

**红因确认**：4 个目标各恰含「新测试引用的预期公共符号 unresolved（实现缺失）」错误（E0432 合计 4 条；E0599 1 条=新错误变体）；无其它类别错误、**0 警告**（首轮曾因初版 recovery.rs 临时借位产生 3 条 E0716、因 S1 断言顺序缺陷（段先于 manifest 创建）需修正，均为测试自身缺陷，见 §6.8）。

### 6.3 工作区红

| 命令 | 结果 |
|---|---|
| `cargo test --workspace` | **exit 101**；错误集合=§6.2（4×E0432+1×E0599+4×could-not-compile；唯一 `warning:` 行为 cargo「build failed, waiting…」驱动信息，非 rustc lint）（`m8_red_ws_final2.log`） |

### 6.4 既有绿不退化

| 命令 | 结果 |
|---|---|
| `cargo test --workspace --exclude sr-itests --exclude sr-pipeline` | **exit 0**（`m8_l1_regress_other_final.log`） |
| `cargo test -p sr-pipeline --test output_naming --test ffmpeg_args_snapshot` | **exit 0**（2 passed；`m8_l1_regress_pipeline_final.log`） |
| `cargo test -p sr-itests --test route --test audio_isolation --test multi_route --test audio_m6 --test merge --test enum_targets --test enum_audio --test shot --no-run` | Finished、**exit 0**（`m8_l2_compile_regress_final.log`） |

### 6.5 Scratch 第 1 层：机制实跑（仓库外 `%TEMP%\opencode\m8scratch`；日志 `m8_scratch_layer1_final.log`）

| 检查 | 实测 |
|---|---|
| **无 Job 对象时**：父（powershell）被杀 → ffmpeg 子进程**仍存活（孤儿）**；`taskkill /T /F` 后消失 | recovery 用例的「Job 回收 sidecar」断言可判别（无 Job 必挂） |
| **录制中途写拒绝注入**：`icacls <dir> /deny <user>:(OI)(CI)W` → ffmpeg 段切换 `Failed to open segment … Permission denied`、`Task finished with error code: -13`、`Conversion failed!`；恢复 ACL 后已封口段（5.200s/4.800s）独立可读时长 | 磁盘满用例机制成立（ENOSPC 类写失败路径） |
| **截断尾段**（60% 字节）：ffprobe exit 0（声明 duration 仍在）、解码 exit 0（带 prematurely 告警）；`concat -c copy` 出可全局解码成品 | 尾段回收/排除规则与容差标定 |

### 6.6 Scratch 第 2 层：dress rehearsal（仓库外副本 `%TEMP%\opencode\sr-m8-red-ws`；ffmpeg 以 junction 挂入；**测试文件 SHA 与仓库逐字一致**，见 §6.7）

对 4 个测试文件原文，在副本内加 scratch-only 假实现（naming 增补 / sidecar 审计+Job 绑定 / probe / recovery + route 会话目录与 manifest + `Cargo.toml` 增 JobObjects feature / deny.toml / docs 来源记录 / cargo-deny PATH shim）后实跑：

| 轮次 | 结果 |
|---|---|
| `safe_commit`（2 用例） | **PASS**（含跨卷 Io 分支；0.02s） |
| `license_and_build_audit`（带 cargo-deny shim） | **PASS**（0.25s：deny 白名单解析、版本 `n8.1.3-18-g46d899d7a0-20261015`、zip SHA256 一致、buildconf 全中、LICENSE ok、docs 来源记录命中、`cargo deny check licenses` exit 0） |
| `license_and_build_audit`（无 shim，真实工具缺失） | **exit 101 + BLOCKED**（设计分支：cargo-deny 未安装 → 不静默 skip） |
| `probe`（真实 smoke+注入矩阵） | **PASS**（5.88s；真实探测 os=10.0.26200.9711、selected=`h264_nvenc`——n8.1 构建下 NVENC 可初始化） |
| `recovery`（全 2 用例，最终文件） | **PASS（2 passed，47.61s）**：S1–S5 合成矩阵 + 相位 R（主进程强杀→Job 回收、活跃拒绝、崩溃收尾 10.100s、幂等、丢弃）+ 相位 R2（编码进程杀→`E_SIDECAR_EXIT`→恢复收尾）+ 磁盘满注入（`DiskFull`→原段保留→重试收尾成功） |

**rehearsal 发现并回写的契约要点（重要）**：
- R1：写失败分类必须扫描完整 stderr 尾部（16KB）——只取最后 3 行摘要会漏掉 `Permission denied`（`sidecar_exit` 首版实测漏判）；已写入 §3.3。
- R2：`active` 判定必须基于**活动文件独占打开探测**而非 pid 存活——同进程内失败（写拒绝注入）后 pid 仍存活，pid 判据会拒绝可重试收尾（首版实测 `Active` 误拒）；已写入 §3.2-3。
- R3：录制内容的段长受编码器/GOP 对齐影响（实测 5.0–6.1s 波动）——真实杀进程相位断言取「≥1 个封口段」与保守时长下界（合成相位保留精确多段断言）。

### 6.7 最终冻结复核与哈希

复跑（最终文件）：定向红 `exit 101`（同 §6.2 错误集）、工作区红 `exit 101`（同 §6.3）、不退化三命令 `exit 0`；rehearsal 副本内 4 文件 SHA 与仓库**逐字一致**。

| 文件 | SHA256（前 16） | 状态 |
|---|---|---|
| `crates/sr-pipeline/tests/safe_commit.rs` | `AA5C6A9250CC4E8F`（全 `…73E2C009`） | **新增** |
| `crates/sr-pipeline/tests/license_and_build_audit.rs` | `B512E4CA4016D696` | **新增** |
| `tests/sr-itests/tests/probe.rs` | `E4F92E98F4BBE1BF` | **新增** |
| `tests/sr-itests/tests/recovery.rs` | `555C94943201DA5A` | **新增** |
| `tests/sr-itests/tests/multi_route.rs` | `56577577AE55945E` | 未动（M6 加固基线） |
| `tests/sr-itests/tests/route.rs` | `D21AB7A43AD9B3DD` | 未动 |
| `tests/sr-itests/tests/audio_isolation.rs` | `B33A566F1CCA2580` | 未动 |
| `tests/sr-itests/tests/audio_m6.rs` | `7D62C9ACCEAE4F1D` | 未动 |
| `tests/sr-itests/tests/common/mod.rs` | `1258527C8131992F` | 未动 |
| `crates/sr-pipeline/tests/output_naming.rs` | `6E1BCB9436F9F48B` | 未动 |
| `crates/sr-pipeline/tests/ffmpeg_args_snapshot.rs` | `F027F697280BFF40` | 未动 |
| `tests/sr-itests/Cargo.toml` | `5D339849C49ADAEF` | 未动（与 M7 一致） |
| `Cargo.lock` | `D6817AB08183611B` | 未动（与 M7 一致） |

生产 `crates/` 与 `src-tauri/` 零改动（本会话在仓库内仅触碰 4 个新测试文件与本报告；`%TEMP%` 副本与 scratch 不入仓库）。

### 6.8 红阶段自查修复（测试自身缺陷）

1. `recovery.rs` 三处 `find_session(&scan_sessions(...), …)` 临时值借位（E0716）→ 改为先绑定 `let sessions`（重构后编译仅剩预期 unresolved）。
2. S1 断言顺序缺陷：段文件先于 manifest 创建导致「目录仅含 manifest.json」断言错误 → 改为「恰含 manifest.json 与既有段（排序后比较）」，本意（无临时残留）不变。
3. 真实杀进程相位段数断言从「≥2 封口段」放宽为「≥1」（段长 5.0–6.1s 波动；§6.6-R3）。
4. 新增相位 R2 后补齐 `ChildGuard::wait_exit`/`kill_tree` helper。

## 7. 关键设计取舍

| # | 取舍 | 依据 |
|---|---|---|
| R1 | REQ-24 补面落在新 L1 文件（`safe_commit.rs`）而非扩展 M3 `output_naming.rs` | 约束「不改既有测试文件」；§4 名称的 sanitize/禁覆盖基础面已由 M3 承载，新文件只补 D12 未覆盖面并注记映射 |
| R2 | probe 契约以 `LayerStatus` 五态区分「拒绝/超时/取消/不可用/失败」，而非 bool 摘要 | REQ-28「三类分别定义、各自处理取消/超时」；bool 无法表达「超时≠权限拒绝」（测试逐态钉死） |
| R3 | 注入面=`probe_capabilities_injected(&ProbeInjections)`（`Some`=整层替身） | 测试确定性 + 预演/UI 复用；真实 smoke 独立存在（结构不变量） |
| R4 | recovery 会话目录/清单由录制路径自动维护（`start_route` 即建 `<output>/.screenrec_tmp/<id>`） | D12 默认约定；M3–M7 既有测试不受影响（隐藏目录 + remove_dir_all 清理） |
| R5 | 「活动文件」用独占打开探测（share_mode 0）而非 pid 存活 | REQ-22 原文「排除活动文件」；pid 判据在同进程失败后误拒可重试收尾（rehearsal 实证 §6.6-R2） |
| R6 | 清单损坏/缺失=拒绝自动收尾（`RecoveryFailed`）+保留原段+显式丢弃；陈旧清单=重扫收尾 | §3.5「E_RECOVERY_FAILED 时保留原段」；身份/目标名丢失时不猜测成品命名 |
| R7 | 尾段回收**可选**（回收成功并入/失败丢弃），不得当封口段用；短封口段（<5s）保留 | S5 §5.3/§8（两方案均可行）；「完成段永不丢弃」 |
| R8 | 磁盘满注入用 ACL 写拒绝（真实写失败路径）而非伪造错误变体 | 无管理员工具造真 ENOSPC；S5 §9a 同类路径；仍钉 E_DISK_FULL 语义 |
| R9 | 真实杀进程相位断言保守化（≥1 封口段/时长下界），精确多段逻辑由合成相位承担 | 段长受编码器/GOP 对齐影响（5.0–6.1s 实测波动） |
| R10 | 审计测试的 cargo-deny 缺失=BLOCKED（fail） | 任务明确「工具缺失按 BLOCKED 语义记录」+ 项目「SKIP 不算通过」纪律；两分支均实测 |
| R11 | rehearsal 副本用 PATH `cargo.exe` shim 驱动 `cargo deny check licenses` 成功分支 | scratch-only；真实工具缺失时工作区仍走 BLOCKED 分支（如实记录） |

## 8. 未决问题（返回 orchestrator；绿阶段须知）

1. **cargo-deny 未安装**：`license_and_build_audit` 在本机将 BLOCKED（fail）。绿阶段须 `cargo install cargo-deny`（新工具安装需用户确认）或由编排者裁定替代证据口径（如接受 deny.toml 静态核验+豁免运行）。两分支已 rehearsal 实测（§6.6）。
2. **恢复与录制的集成面**：本红假定 `start_route/start_routes/start_audio_only` 自动创建会话目录并维护 manifest/Job（§3.2-1）；这是 REQ-22「完整」的实义，但会改动 route.rs 内部行为（不破坏既有测试语义）。若编排者希望显式「会话 API」而非自动维护，需在绿阶段前定夺（契约已按自动维护拟定）。
3. **pid 身份的用途收窄**：`identity` 仍按 §3.4 记录（REQ-30 要求），但恢复可收尾性判据不用 pid（§3.2-3）；若编排者要求 pid 复核（pid+创建时间匹配）作为附加判据，需明确其与「同进程失败后可重试」的优先级。
4. **磁盘满注入≠真 ENOSPC**：ACL 写拒绝覆盖「CreateFile 失败」类写错误；真满盘（写中途 ENOSPC）仍属未验证（S5 §12-2 同注记）。如须真满盘，需要管理员权限工具（VHD/配额）或产品级故障注入钩子。
5. **尾段回收策略未钉死**（可选）：本红允许「回收或丢弃」（保底=封口段全收）；若产品要求 δ_play≤1s 承诺（S5 §11-REQ-21-4），绿阶段应实现 remux 回收并在报告中记账。
6. **WAV 恢复的成品语义**：本红钉「就地封口 + `repaired_wavs` 上报」；WAV 是否移动/重命名为成品（REQ-19 停止即成品形态）未涉及，属实现自由面。
7. **probe 三类激活面的真实实现深度**：本红只钉状态/归类/降级与不误判（超时≠权限）；`激活成功但无信号` 检测流程（REQ-28 末句/S3 语义）不在本红用例内（属音频内容级判定面）。
8. **单会话多路由的恢复口径**：契约按 `routes[]` 逐路收尾，但合成矩阵只覆盖单路由会话；多路由崩溃会话的并发收尾顺序/部分成功语义未测（M5 会话形态），绿阶段应明确（建议：逐路独立、失败隔离）。
9. **`unit_naming_sanitize_no_overwrite` 名称归一**：本红以两个描述性用例覆盖 §4 名称剩余面；若编排者要求 §4 名称逐字出现，可在绿阶段调整用例名（测试文件归属本红）。
10. **审计来源记录的锚点**：本红要求 docs/ 下出现 zip SHA256；若编排者希望固定文件路径（如 `docs/audit/ffmpeg-locked-build.md`）或额外校验源码获取说明措辞，需明确（当前为「docs/ 递归任一文本含 SHA256」）。
11. **SESSIONNAME/用户注入注记**：L2 需 `SESSIONNAME='Console'`；icacls 注入需 `USERNAME`（非交互 shell 可能缺失 → BLOCKED，非测试缺陷）。

## 9. 结论

- M8 红测试就位：4 个新测试文件、5 个用例（§4 点名 ×3：`it_probe_permission_matrix_injected` / `license_and_build_audit` / `it_crash_kill_matrix_recovery_state_machine` / `it_fault_injection_disk_full`，＋REQ-24 剩余面 L1 ×2），L2 全部 `#[ignore]` 门 + BLOCKED=fail + 文件内 GATE；L1 纯逻辑（审计用例按设计在工具缺失时 BLOCKED）。
- 新 API 契约（§3）按 spec §3.2/§3.3/§3.4 拟定：probe 注入面与五态归类、recovery 会话/清单/收尾/丢弃、`RouteError::DiskFull`、`naming` 会话临时目录与安全提交、`sidecar` 锁定构建审计。
- 红因正确：`cargo test --workspace`（及两个定向命令）**exit 101，仅 4×E0432＋1×E0599（全部为预期未解析符号/变体）、0 警告**（§6.2/§6.3/§6.7）；既有 L1 及既有 L2 目标编译全绿（§6.4）。
- 双层自检 + dress rehearsal：机制实跑（Job 孤儿前提/ACL 写失败/截断尾段）；副本内 4 文件原文对 scratch 实现**全绿**（含真实子进程杀矩阵与磁盘满注入），并据实回写两条契约要点（stderr 全量扫描、活动文件探测）；cargo-deny 缺失的 BLOCKED 分支亦实测。
- 未写任何生产代码；未 git 提交；报告数字全部来自本机实跑日志（路径已列）。测试文件哈希：`safe_commit.rs` `AA5C6A9250CC4E8F…`、`license_and_build_audit.rs` `B512E4CA4016D696…`、`probe.rs` `E4F92E98F4BBE1BF…`、`recovery.rs` `555C94943201DA5A…`。
