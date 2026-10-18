# M4 红测试报告（TDD 红阶段：音频隔离 REQ-14/17/18/26 的 M4 任务范围）

- 日期: 2026-10-16
- 规格: specs/001-screen-recorder.md §2.4（REQ-14/17/18）、§2.6（REQ-26）、§2.7（REQ-28 部分）、§3.1/§3.2/§3.3（线程规则/模块职责/管道与基准命令）、§3.4/§3.5、§4、§5 M4 行、§8 M0 冻结项（S3 音频隔离结论）
- Spike 依据: docs/spikes/s3/s3-audio-isolation.md §2（方法/阈值）、§3.2–§3.5（隔离/静音/并发/树语义）、§4（激活流程与线程）、§3.6 extra（phantom PID 教训）；docs/spikes/s1b（写超时 3–5s→E_PIPE_STALL、阻塞写=背压）
- 先例: docs/milestones/m3-red-report.md（契约/门/双层自检方法论）、m3-green-report.md（现有 sr-pipeline route API 面貌）
- 角色: Test Writer（红阶段）。本任务只写测试，未写任何生产实现（sr-audio 仍为 M1-A 占位骨架）。
- 环境实测: Windows 11 25H2，build **10.0.26200.9457**，SESSIONNAME=Console；rustc/cargo 1.99.0 msvc（`%USERPROFILE%\.cargo\bin`，PATH 外以全路径调用）；sidecar/ffplay/ffprobe=锁定 `tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin`（§8 冻结）。
- 到场状态: M3 缺陷（output_naming 3 处期望值笔误）已被测试作者修复（本机验证 1 passed）；仓库无 git 提交历史（全部 untracked，与 M1–M3 一致）。

## 1. 交付物

| 文件 | 性质 | 内容 |
|---|---|---|
| `crates/sr-audio/tests/silence_pad_late_packet.rs` | 新增 | L1 ×1：`unit_silence_pad_late_packet_drop`（样本槽静音补齐/迟到裁切，REQ-18） |
| `crates/sr-audio/tests/drift_hysteresis.rs` | 新增 | L1 ×1：`unit_drift_hysteresis_limits`（漂移滞回带/速率限幅，REQ-26） |
| `tests/sr-itests/tests/audio_isolation.rs` | 新增 | L2 ×4：隔离 FFT / 有界身份无宿主泄漏 / 隔离模式音轨 ffprobe+FFT / 背压阶跃校正 |
| `tests/sr-itests/Cargo.toml` | 追加 | `sr-audio` path 依赖（契约目标）+ `rustfft 6.4.1`（§4 FFT 手法） |
| `Cargo.lock` | 更新 | 随 rustfft 新增传递闭包（num-complex/num-integer/num-traits/primal-check/strength_reduce） |
| `docs/milestones/m4-red-report.md` | 新增 | 本报告 |

未改动：生产 src（sr-audio/sr-pipeline 等全部未动）、specs/、docs/spikes/、既有测试（M1/M2/M3 逐字未动；`common/mod.rs` 零改动——新 helper 全部内聚在 audio_isolation.rs，沿 M3 先例）。未 git 提交。

## 2. REQ → 测试 → 文件映射（§4 命名优先）

| REQ | 测试名 | 层 | 文件 | 断言要点 |
|---|---|---|---|---|
| REQ-14（核心） | `it_audio_application_isolated_fft` | L2 | audio_isolation.rs | A(440)+B(880) 同播：并发两客户端（S3 场景 d）进程环回各自录制 6s；各自目标音存在（≥本底+30dB）、非目标 ≤−40dB（相对；S3 预锁定）；录 B 含 880 = 整机混音确含两声的自对照（S3 场景 b 合并）；REQ-18 端到端：48k/f32/立体声、帧数=请求时长×48000（容差 48 帧=1ms，静音补齐钉时长） |
| REQ-14（有界身份） | `it_audio_identity_bounded_no_host_leak` | L2 | audio_isolation.rs | ①父 cmd+子 ffplay 树（S3 场景 e）：以 cmd 为根 INCLUDE 树捕获含子进程 440、树外 880 ≤−40dB；②explorer.exe 为根 → `HostForbidden` 且 ≤2s 快速失败（不得变相录系统混音）；③phantom PID 999_999（奇数 ⇒ 必非合法 PID）→ `TargetLost` ≤2s（S3 extra：激活不校验 PID，静默零包教训）；④真实 PID+偏差 5min 创建时间 → `IdentityAmbiguous`（D9 PID 复用防护） |
| REQ-17（隔离模式部分） | `it_audio_multitrack_ffprobe_fft` | L2 | audio_isolation.rs | 窗口路由+目标应用音源：成品 MKV 恰 1 视频（h264/640×360）+ 恰 1 音轨（隔离默认，无 mic ⇒ 无 mic 轨）、aac/48k/立体声（§3.3 基准）；时长≈录制段（±2s）；音轨解码 FFT：440 存在、880 ≤−40dB（全音轨模式部分属 M6，见 §8-1） |
| REQ-18 | `unit_silence_pad_late_packet_drop` | L1 | silence_pad_late_packet.rs | 100ns→样本显式换算（10ms@48k=480 帧）；首包 T0；定速流顺次写入；断流空洞按样本槽补零（pad 计入水位）；补零区间后迟到整包丢弃（水位不回退，禁双写）；部分迟到裁前缀接续写入；SILENT 旗标同样占时间轴 |
| REQ-26（慢漂移决策核） | `unit_drift_hysteresis_limits` | L1 | drift_hysteresis.rs | 带内（<enter）Hold；越带进入校正（方向同号、非零、≤限幅）；±10s 偏差饱和于 ±500ppm；滞回（校正态在 (exit,enter) 维持、≤exit 退出、退出后不自启）；cfg 参数化收缩（enter 10/limit 100 生效）；增益不钉死（S6 定值前实现自由） |
| REQ-26（阶跃校正） | `it_backpressure_step_correction` | L2 | audio_isolation.rs | 录制中挂起 sidecar 全部线程 2s（< S1b 写超时下限 3s，不触 E_PIPE_STALL；纯背压「活但不读」）→ 恢复收尾：音轨 aac/48k/2ch、时长≈墙钟段（±0.75s；无校正亏空 ≈2s 必失败）；停滞前后 2s 目标音均在（≥本底+20dB）；整文件解码干净 |
| REQ-28 部分 | （隐式） | — | audio_isolation.rs | 进程环回试激活+「激活成功且有信号」路径被隔离用例覆盖；显式探测矩阵 `it_probe_permission_matrix_injected` 按 §5 M8 行属 M8（见 §8-8） |

## 3. API 契约清单（测试即契约；模块名沿用 M1-A 骨架 `wasapi`/`slots`/`resample`）

### 3.1 `crates/sr-audio` —— `pub mod wasapi`（骨架已有模块名）

```rust
/// §3.4 AudioSourceSpec（M4 测试仅构造 App；System/Mic 为契约记载，M6 用例构造）。
#[derive(Debug, Clone)]
pub enum AudioSourceSpec {
    App { pid: u32, created_at_unix_ms: u64 },
    System,                       // REQ-15（M6）
    Mic { device: String },       // REQ-16（M6）
}

#[derive(Debug)]
pub enum CaptureError {
    TargetLost { detail: String },          // E_TARGET_LOST：PID 不存在（激活前校验）
    IdentityAmbiguous { detail: String },   // E_IDENTITY_AMBIGUOUS：创建时间不符/PID 复用
    HostForbidden { detail: String },       // REQ-14：系统宿主根硬性禁止
    LoopbackUnavailable { detail: String }, // E_LOOPBACK_UNAVAILABLE（L2 → BLOCKED）
    Timeout,
    Failed { detail: String },
}

/// REQ-18/26 记账（绿阶段事件/UI 呈现可扩展字段；测试读三字段）。
#[derive(Debug, Clone, Default)]
pub struct CaptureStats {
    pub padded_frames: u64,       // 静音补齐帧数
    pub dropped_frames: u64,      // 迟到/背压裁切丢弃帧数
    pub discontinuities: u64,     // DATA_DISCONTINUITY 包数
}

#[derive(Debug, Clone)]
pub struct CapturedPcm {
    pub samples: Vec<f32>,        // 交错立体声 L,R,L,R,…
    pub sample_rate: u32,         // 48000（REQ-18 管线固定格式）
    pub channels: u16,            // 2
    pub stats: CaptureStats,
}

/// 进程环回捕获（同步「录 N 秒返回」；S3 apploop record 同构）。
pub fn capture_process_loopback(
    pid: u32,
    created_at_unix_ms: u64,
    duration: Duration,
) -> Result<CapturedPcm, CaptureError>;
```

行为语义（测试钉死）：
- **激活前身份前置校验**：PID 存在（否则 `TargetLost`）+ 创建时间吻合（否则 `IdentityAmbiguous`），失败**快速返回**（≤2s，不得录满 duration）——S3 §3.6 phantom 教训（激活不校验 PID，8s 静默零包）。
- **宿主禁入**：捕获根映像名为 explorer.exe / svchost.exe / ApplicationFrameHost.exe / RuntimeBroker.exe / sihost.exe → `HostForbidden`（REQ-14 硬性禁止；防宿主泄漏=变相录系统混音）。cmd.exe 等普通父进程合法（树根）。
- **INCLUDE_TARGET_PROCESS_TREE**（D9 默认根 = PID+后代树；S3 场景 e：父根含子进程音；EXCLUDE 语义不用于此）。
- **专用 MTA 线程**内完成 COM/客户端创建/使用/释放（S3 §4；wasapi !Send 不跨线程）；UI 线程禁用（RPC_E_CHANGED_MODE）。
- **格式**：48kHz/f32/立体声 `EventsShared{autoconvert:true}`（REQ-18；process loopback 无 mix format，须自建格式）。
- **时长钉齐**：返回帧数 = duration×48000（S3 §3.3 补静音把时长精确补到请求值；测试容差 48 帧）。
- **并发**：两个线程同时各调一次（双客户端并发，S3 场景 d 已验证 2 路）。
- 绿阶段实现路由流式（管道写）时，建议复用同一采集核，本函数作为其上内存 sink 薄封装（见 §8-4）。

### 3.2 `crates/sr-audio` —— `pub mod slots`（骨架已有）

```rust
/// 单个 WASAPI 包经样本槽编排后的写入计划（REQ-18）。
#[derive(Debug, Clone, PartialEq)]
pub struct PacketPlan {
    /// 写入前须补的静音帧数（时间轴空洞；0=连续）
    pub pad_frames: u64,
    /// 迟到裁切：包首部与已写区间重叠而丢弃的帧数（== frames ⇒ 整包丢弃）
    pub trimmed_frames: usize,
    /// 写入起始样本槽（相对 T0；迟到裁切时 = 当前水位，接续写入）
    pub start_sample: u64,
}

pub struct SampleSlotPacer { /* … */ }
impl SampleSlotPacer {
    pub fn new(sample_rate: u32) -> Self;                 // 48_000
    /// WASAPI 包：100ns 时间戳 + 帧数 + SILENT 旗标 → 写入计划。首包建立 T0。
    /// silent=true（真实静音）同样占时间轴（内容写零由写入方执行，不触发额外补齐）。
    pub fn on_packet(&mut self, ts_100ns: i64, frames: usize, silent: bool) -> PacketPlan;
    /// 已编排输出总帧数（含补齐；迟到裁切不回退）
    pub fn written_frames(&self) -> u64;
}
```

语义（REQ-18，测试逐条钉死）：
- 起始槽 = floor((ts−T0)·rate/10⁷)（**100ns→样本换算显式实现**，REQ-25；48k 下 10ms 恰 480 帧）。
- 空洞（起始槽>水位）→ 先补 `pad_frames` 零帧再写（静音补齐按样本槽）。
- 迟到（起始槽<水位，含落在补零区）→ 裁切重叠前缀，余量在**水位处接续**写入；整包落在已写区间 → 丢弃（水位不变）——**禁止同一时间段写两次**。
- AUDCLNT_BUFFERFLAGS 语义区分（真实静音/断流/设备错误）由 silent 参数与 pad/trim 决策共同表达（S3 §3.3：目标停止后流不断、内容为数字零、无 SILENT/DISCONTINUITY 旗标——补帧钉时长）。

### 3.3 `crates/sr-audio` —— `pub mod resample`（骨架已有）

```rust
/// REQ-26 参数化漂移控制配置（测量窗口/滤波为实现内部记账；三参数测试注入）。
#[derive(Debug, Clone, PartialEq)]
pub struct DriftCfg {
    pub enter_ms: f64,       // 进入校正的 |偏差| 阈值（锚点 40：阶跃阈值起点同源）
    pub exit_ms: f64,        // 退出阈值（< enter，滞回带）
    pub rate_limit_ppm: f64, // 速率限幅（锚点 500：±500ppm 实验起点，S6 定值）
}

#[derive(Debug, Clone, PartialEq)]
pub enum DriftAction {
    Hold,
    Correct { rate_ppm: f64 },   // 符号与偏差同号（正=音频落后须加速/丢）；≠0；|·|≤limit
}

pub struct DriftController { /* … */ }
impl DriftController {
    pub fn new(cfg: DriftCfg) -> Self;
    /// 输入内容偏差测量（正=音频内容落后基准：已写样本/48000 vs QPC/已写帧数）。
    pub fn on_measurement(&mut self, content_offset_ms: f64) -> DriftAction;
}
```

边界语义：|offset| < enter ⇒ Hold；≥ enter 进入；校正态在 (exit, enter) 维持、≤ exit 退出；巨大偏差饱和于限幅；增益（offset→rate 比例）不钉死。**与阶跃校正分开记账**（REQ-26）。

### 3.4 `crates/sr-pipeline` —— `pub mod route`（M4 扩展，测试假定新增；**既有 `start_route`/`RouteConfig` 逐字不动**）

```rust
/// M4 音频路由配置（§3.4 RouteConfig 的音轨扩展面；video 保持必填——
/// M4 用例均为窗口目标路由。audio= 隔离模式的源列表，App 一源一轨）。
pub struct AudioRouteConfig {
    pub target: RouteTarget,                            // 既有枚举（M3）
    pub video: VideoCfg,                                // 既有（M3）
    pub audio: Vec<sr_audio::wasapi::AudioSourceSpec>,  // 隔离模式：[App{目标}]
    pub output_dir: PathBuf,
}

/// 启动带音轨的路由。Ok = 输出开始（§3.1，同 start_route）；
/// 句柄/收尾语义复用 M3 RouteHandle（request_stop / wait_finalized / segments）。
pub fn start_audio_route(cfg: AudioRouteConfig) -> Result<RouteHandle, RouteError>;

// RouteError 增补变体（M4）：
//   LoopbackUnavailable { detail: String }   // E_LOOPBACK_UNAVAILABLE（§3.5：路级，不改录系统混音）
```

行为语义：隔离模式每 `App` 源一条独立音轨（互不混音，REQ-17）；音频管道 f32le/48k/2ch（§3.3），成品音轨 aac/48k/立体声；音源 PID 消失 → 该轨静音继续、路由不失败（REQ-14/S3 §3.3 引擎持续供零）；sr-pipeline 需新增 `sr-audio` path 依赖（绿阶段 Cargo.toml 变更）。

### 3.5 契约要点（行为层）

- **BLOCKED 分流**：`LoopbackUnavailable`（默认渲染端点/进程环回不可用）、`EncoderUnavailable`（§9 T1）、`ConsentDenied`（WGC）→ 测试按环境缺失记 BLOCKED（fail）；其余 Err 均为断言失败。
- **背压注入边界**：挂起 sidecar 2s 必须落在「纯背压」路径——绿实现的管道**写超时下限须 ≥3s**（S1b 冻结 3–5s→CancelIoEx→E_PIPE_STALL），否则用例误入 E_PIPE_STALL（契约约束，绿阶段须遵守）。
- **阶跃校正观测口径**：音轨时长≈墙钟 ±0.75s（无校正亏空 ≈2s）；实现以环形缓冲零丢失自然对齐同样合法（内容对齐即 REQ-26 目标）；校正**事件记账**接口未钉死（绿阶段自由，L3 soak 观测）。
- **宿主禁入的位置**：本红钉在 sr-audio 捕获层（根校验）；REQ-14 的「有界提升规则」（经验证规则扩大捕获根、共享范围告知）属 sr-enum 身份解析层（M1 identity.rs 预留 M4+ 扩展），不在本红断言范围（见 §8-10）。

## 4. 运行命令与门控

```text
# L1（随默认 cargo test 执行）
cargo test -p sr-audio --test silence_pad_late_packet -- --nocapture
cargo test -p sr-audio --test drift_hysteresis -- --nocapture

# L2（显式执行；须交互式用户会话 + 默认渲染端点 + WGC + 可用编码器 + 锁定 n8.1）
cargo test -p sr-itests --test audio_isolation -- --ignored --nocapture
# 单用例示例
cargo test -p sr-itests --test audio_isolation it_audio_application_isolated_fft -- --ignored --nocapture

# 全仓（红阶段 exit 101 属预期）
cargo test --workspace
```

BLOCKED 触发点：非交互会话；锁定 n8.1 构建缺失（ffbin canonicalize 失败）；默认渲染端点缺失/环回激活失败（LoopbackUnavailable）；route 用例另含 EncoderUnavailable（§9 T1）/ConsentDenied（WGC 被拒）。文件内 `GATE` Mutex 使 4 个 L2 用例串行（见 §7-R5）。

## 5. 依赖与锁定构建

- `tests/sr-itests/Cargo.toml` 追加：`sr-audio`（path，契约目标）；`rustfft = "6.4.1"`——依据：spec §4「各轨 FFT（realfft）断言」点名 FFT 手法，**S3 spike 实际分析器即 rustfft**（s3-audio-isolation.md §2「FFT（analyze 子命令，rustfft）」），且 6.4.1 已在本地离线缓存（realfft 不在，拉取需网络）；测试侧依赖、非生产代码。Cargo.lock 随之新增 5 个传递 crate。
- L1 测试位于 `crates/sr-audio/tests/`（自动链接本 crate；sr-audio/Cargo.toml 零改动——wasapi 0.26 由绿阶段引入）。
- ffmpeg/ffplay/ffprobe：全部走 `tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin`（§8 冻结的 release 构建；含 ffplay.exe，实测存在）。测试音 WAV 用 `sine=...,volume=0.25` 合成为 48k 立体声 f32（40s，单用例跨度 ≤12s）。

## 6. 红灯证据（实际执行，2026-10-16）

### 6.1 `cargo test --workspace` → exit 101（全部为预期未解析符号；cargo 首批失败后未再调度 sr-itests 目标，另跑定向命令补全）

```text
error[E0432]: unresolved imports `sr_audio::slots::PacketPlan`, `sr_audio::slots::SampleSlotPacer`
  --> crates\sr-audio\tests\silence_pad_late_packet.rs
error[E0432]: unresolved imports `sr_audio::resample::DriftAction`, `sr_audio::resample::DriftCfg`, `sr_audio::resample::DriftController`
  --> crates\sr-audio\tests\drift_hysteresis.rs
error: could not compile `sr-audio` (test "drift_hysteresis") due to 1 previous error
error: could not compile `sr-audio` (test "silence_pad_late_packet") due to 1 previous error
```

### 6.2 定向命令（sr-itests；exit 101，同类未解析）

```text
cargo test -p sr-itests --test audio_isolation --no-run
error[E0432]: unresolved imports `sr_audio::wasapi::capture_process_loopback`, `sr_audio::wasapi::AudioSourceSpec`,
               `sr_audio::wasapi::CaptureError`, `sr_audio::wasapi::CapturedPcm`   (audio_isolation.rs:60)
error[E0432]: unresolved imports `sr_pipeline::route::start_audio_route`, `sr_pipeline::route::AudioRouteConfig`
               (audio_isolation.rs:61)
error[E0599]: no variant named `LoopbackUnavailable` found for enum `RouteError`
               (audio_isolation.rs:383)
error: could not compile `sr-itests` (test "audio_isolation") due to 3 previous errors
```

**红因确认**：5 个 error 全部为「新测试引用的预期公共符号/枚举变体 unresolved（实现缺失）」，无任何其它类别错误；新测试代码 0 警告（stub 全量 typecheck 证实，§6.5）。

### 6.3 既有绿不退化

| 命令 | 结果 |
|---|---|
| `cargo test -p sr-core -p sr-capture -p sr-pipeline -p screen-rec` | **exit 0**：screen_rec_lib 7 passed；pacer_cfr 1 passed；channel_snapshot_rebind 1 passed；ffmpeg_args_snapshot 1 passed；output_naming 1 passed（M3 缺陷已由测试作者修复） |
| `cargo test -p sr-itests --test enum_targets --test enum_audio --test shot --test route --no-run` | Finished、exit 0（M1/M2/M3 全部 L2 目标可编译，无回归） |

### 6.4 helper 实跑自检（第 1 层；仓库外 scratch `%TEMP%\opencode\sr-m4-red-scratch\helper`，exit=0）

```text
[env] Microsoft Windows [Version 10.0.26200.9457]；SESSIONNAME=Ok("Console")
[fft] 边界混合：target=65.4 other=25.9 floor=0.0 rel=-39.6dB      ← −40dB 边界信号测得 −39.6（容差 ±2）
[fft] 纯 440：target=65.4 other=-92.8 floor=0.0 rel=-158.2dB      ← 与 S3 实测隔离底 −158dB 一致
[tone] ffplay 存活=true（wav=15360092 bytes）                      ← n8.1 ffmpeg 合成 + ffplay 播放 + 树杀
[aac] 解码 240640 帧（≈5.01s）；target=47.4 other=-81.6（存在性差 47.4dB）
[aac] stream=duration 不可解析（"N/A"）→ 回退 format=duration；时长 = 5.02s   ← probe_audio_duration 回退设计被证实必要
[bp] ping 挂起 6 线程；恢复后退出=0，总耗时 5.03s（正常 ≈3s + 冻结 2s）      ← SuspendThread 背压注入原语有效
[tree] cmd=61856 子进程=[118740]                                  ← children_of 树发现
[host] explorer.exe = Some(15732)                                 ← 宿主拒绝用例前置
ALL M4 HELPER CHECKS DONE（exit 0）
```

### 6.5 stub-编译自检（第 2 层，M3 §9.4 方法论；契约桩位于 scratch `ws\crates\sr-audio`（桩 lib.rs）+ 真实 sr-pipeline 副本追加桩（AudioRouteConfig/start_audio_route/LoopbackUnavailable），不进仓库）

对三个新测试文件做「import 可解析之后」全量 typecheck/borrowcheck：`cargo test --offline --no-run` → **exit 0、0 警告**，三个新测试可执行全部产出（silence_pad_late_packet / drift_hysteresis / audio_isolation），既有全部目标同步编译通过。**双层自检共抓到并已修复 5 处测试自身缺陷**（先 scratch/stub 验证再修仓库，全部在 scratch 证实后回移）：

| # | 缺陷 | 发现层 | 修法 |
|---|---|---|---|
| 1 | `Thread32FirstW/Thread32NextW` 不存在（windows 0.62 ToolHelp 线程枚举无 W 后缀） | 双层 | 改 `Thread32First/Thread32Next`（import + 调用点） |
| 2 | `player_b` 未使用警告（守卫绑定后无引用） | 第 2 层 | `_player_b` 绑定（Drop 守卫仍执行）+ println 引用同步 |
| 3 | scratch 缺 `CommandExt` import（raw_arg） | 第 1 层 | scratch 补 import（仓库文件本就函数内 use，无此缺陷） |
| 4 | scratch 借用/所有权三处（mut player、元组 move、Option move） | 第 1 层 | 仅 scratch 侧修正（仓库文件写法本就正确） |
| 5 | ffprobe aac `stream=duration` 返回 "N/A" 直接 parse 会 panic | 第 1 层 | `probe_audio_duration` 设计为回退容器时长（scratch 证实回退路径可用） |

## 7. 关键设计取舍

| # | 取舍 | 依据 |
|---|---|---|
| R1 | FFT 判定双阈值且全相对化：目标 ≥本底+30dB（隔离）/+20dB（存在性），非目标 ≤目标−40dB | S3 预锁定阈值；绝对电平依赖（ffplay 音量/端点增益）被消除；分析器 32768 点 Hann 帧平均谱 = S3 同参数，scratch 数值标定（−39.6/−158.2/+47.4dB） |
| R2 | 测试音音量 0.25（两声相加 0.5） | 满幅两音在共享模式混音会削波，440 的**二次谐波（削波失真）恰落位 880** 造成假泄漏；0.25 保持线性区（f32 WAV 全线性） |
| R3 | 双目标并发录制互为对照（免设备环回对照） | S3 场景 b 的设备环回对照依赖 REQ-15（M6）；「录 B 含 880」即证明整机混音确含两声；并发同时覆盖场景 d（双客户端） |
| R4 | 背压注入 = SuspendThread/ResumeThread 挂起 sidecar 全部线程 2s | 零契约侵入（无需故障注入钩子——那是 REQ-28/M8 的 `it_probe_permission_matrix_injected`）；2s < S1b 写超时下限 3s ⇒ 纯背压（「活但不读」不触发 E_PIPE_STALL）；scratch 以 ping 冻结实测验证原语 |
| R5 | 文件内 `GATE` Mutex 串行 4 用例 | S3 仅验证 2 路并发客户端；4 用例并行会同时起 6+ 音源/2 路由，超出已验证并发面；隔离语义本身不受串行影响（进程环回按目标过滤） |
| R6 | 身份校验断言「快速失败 ≤2s」 | S3 §3.6 phantom 教训（激活 1.2ms 成功 + 8s 静默零包无错误信号）转为可测契约：校验必须先于激活/录制 |
| R7 | `PacketPlan` 三元组 + 水位模型 | 以纯函数决策钉死 REQ-18（补齐/裁切/禁双写），对齐 M3 FramePacer 先例；迟到余量在水位接续（不留洞、不回退）；100ns 换算用 48k 整除锚点消除舍入噪声 |
| R8 | drift 只钉方向/限幅/滞回形状，不钉增益与滤波 | REQ-26 参数「全部参数化记账」+ S6 定值；锁增益会把实现钉死在未冻结细节 |
| R9 | 多轨/背压用例的视频目标用 common 的 `ProbeWindow`（静态窗） | M4 断言音轨形状与内容；视频内容级判定属 M3 已覆盖——不复制 MotionWindow（helper 内聚原则） |
| R10 | 隔离用例时长容差 48 帧（1ms） | S3 补静音模式实测精确到请求值（7.98s→8.000s）；容差仅容纳尾部包边界舍入 |
| R11 | `start_audio_route` 与既有 `start_route` 并立（不给 RouteConfig 增字段） | M3 route.rs 测试以全字段字面量构造 RouteConfig——增字段会破坏既有测试编译（本任务约束「不改既有测试」）；合并为单一 API 需重构 M3 测试编排，留 §8-5 协商 |
| R12 | phantom PID 取 999_999（奇数） | Windows PID 为 4 的倍数 ⇒ 奇数必不存在，跨机器确定性（S3 同值） |

## 8. 未决问题（返回 orchestrator）

1. **REQ-17 全音轨模式部分属 M6**：spec §5 M4 与 M6 两行都点名 `it_audio_multitrack_ffprobe_fft`（§4 注「隔离/全音轨两模式」）。本红按 M4 范围落**隔离模式部分**（恰 1 应用音轨）；M6 红阶段需扩展该用例（System/Mic/追加应用轨、互不混音、按根去重）或新增姊妹用例——届时修改属 M6 红作者权限。
2. **rustfft vs realfft**：spec §4 文本写 realfft，S3 实际用 rustfft（报告 §2 明示），本红沿用 rustfft 6.4.1（离线缓存；判定语义与 S3 校准一致）。若须严格 realfft，需网络拉取并改 2 行 import（分析器结构不变）。
3. **写超时下限契约**：`it_backpressure_step_correction` 依赖绿实现的管道写超时 ≥3s（S1b 冻结 3–5s）。若绿阶段选择 <2s 的写超时，用例会误入 E_PIPE_STALL——这不是测试缺陷，是实现违背 S1b 冻结项，绿阶段须以 ≥3s 落地。
4. **`capture_process_loopback` 同步形态与流式实现的关系**：本函数是红测试假定的一等 API（S3 apploop record 同构）。绿阶段实现路由流式管道写时，应复用同一采集核（身份校验/MTA 生命周期/补齐/裁切），本函数作为内存 sink 薄封装——避免两套行为分叉。若实现决定只提供流式句柄 API，需与本测试作者协商改测试或提供等价封装。
5. **`AudioSourceSpec` 与音频路由 API 归属**：测试假定 `sr_audio::wasapi::AudioSourceSpec` + `sr_pipeline::route::{AudioRouteConfig, start_audio_route}`。§3.4 数据模型意图 AudioSourceSpec 落 sr-core::types（M1 占位未实现）；若实现迁移需同步改 audio_isolation.rs 一处 import（同 M3 red §8-4 模式，可协商）。绿阶段如合并 start_route/start_audio_route 为单一 API，必须保持 M3 route.rs 测试零改动可编译（R11），编排方案另行协商。
6. **M6 范围项**：REQ-15 设备环回（`it_audio_device_loopback_rms`）、REQ-16 麦克风（`it_mic_capture_present_or_skip`）、REQ-19 纯录音 WAV（`it_audio_only_wav_duration_and_repair`）不在 M4 红；本机无麦克风（S3 §1），mic 断言届时在本机只能 BLOCKED。
7. **阶跃校正的事件记账接口未钉死**：L2 以内容对齐（时长≈墙钟）观测；「校正事件」的显式上报（REQ-26 记账、L3 soak 观测项）接口由绿阶段自由设计，后续如需 L2 断言再补测试。
8. **REQ-28 部分的覆盖口径**：M4 行的「28 部分」以隔离用例隐式覆盖进程环回试激活+有信号路径；显式探测矩阵（`it_probe_permission_matrix_injected`，含故障注入钩子）按 §5 M8 行属 M8。
9. **阈值标定环境**：−40dB（S3 预锁定）、+30/+20dB 存在性、48 帧时长容差基于本机 scratch/合成实测；绿阶段若在异常端点（独占模式/受保护内容）逼近阈值，凭实测证据调整并记录（不放宽断言语义；S3 §5 已列 DRM/独占为未测风险）。
10. **宿主禁入的双层位置**：本红把宿主拒绝钉在 sr-audio 捕获层（纵深防御）；REQ-14 完整「有界提升规则」（经验证规则、审计依据、共享范围 UI 告知）属 sr-enum 身份解析层（identity.rs 注释预留 M4+），spec §4 无对应测试名——建议绿阶段补 L1 单测覆盖规则逻辑（不阻塞 M4 验收，AC-2 不依赖）。
11. **`AudioRouteConfig.video` 必填**（非 §3.4 的 Option）：M4 用例均为窗口目标；纯音频路由（video=None）语义上更接近 REQ-19 纯录音（M6，WAV 路径）——若 M6 需要 MKV 形态的纯音频路由，届时扩展为 Option 并保持本红用例构造不变。
12. **时间预算未断言**（同 M3 red §8-8）：激活时延（S3 实测 1–6ms）、停止响应 ≤2s、收尾时长未设断言；wait_finalized 120/150s 超时隐含宽松上限。
