# M6 红测试报告（TDD 红阶段：全音轨/纯录音 REQ-15/16/19 + REQ-17 全音轨模式补齐部分）

- 日期: 2026-10-17
- 规格: specs/001-screen-recorder.md §2.4（REQ-15 系统音频 / REQ-16 麦克风 / REQ-19 纯录音全文；REQ-17 全音轨模式）、§3.1/§3.2/§3.3/§3.4/§3.5、§4（REQ-15/16/17/19 行）、§5 M6 行、§8 M0 冻结项（S3）
- Spike 依据: docs/spikes/s3/s3-audio-isolation.md §2（方法/阈值）、§3.2–§3.5（隔离/静音/并发/树语义）、§4（激活流程与线程/格式/时基）；docs/spikes/s1b（写超时/背压模型，本报告仅引用不影响 M6 用例）
- 先例: docs/milestones/m4-red-report.md（契约/门/双层自检方法论）、m4-green-report（现有 sr-audio 契约实现面貌与 §6 已知 flake 注记）、m5-red-report（内容级阈值标定方法）
- 角色: **executor-backup 承担 Test Writer（红阶段）**（GLM 通道额度耗尽）。本任务**只写测试**：生产 src 零改动（sr-audio/sr-pipeline 等全部未动）；未 git 提交。
- 环境实测: Windows 11 25H2，`Microsoft Windows [Version 10.0.26200.9711]`（UBR 9711，M5 同）；cargo/rustc **1.99.0**；sidecar/ffplay/ffprobe = 锁定 `tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin`（§8 冻结）。
- 运维注记（本会话实测）: 当前工具 shell 不继承 `SESSIONNAME`；L2 运行时须显式 `$env:SESSIONNAME='Console'`（否则 `require_interactive_session` 按 §4 记 BLOCKED——scratch 首次实跑即命中此分支，属设计行为）。
- 到场状态: M5 达成（编排者独立复跑全绿）；本次改动前基线 `cargo test --workspace` = **exit 0**（日志 `%TEMP%\opencode\m6_baseline_ws.log`）。仓库无 git 提交历史（全部 untracked，与 M1–M5 一致）。

## 1. 交付物

| 文件 | 性质 | 内容 |
|---|---|---|
| `tests/sr-itests/tests/audio_m6.rs` | 新增（51 KB） | L2 ×4：`it_audio_device_loopback_rms`（REQ-15）/ `it_mic_capture_present_or_skip`（REQ-16）/ `it_audio_only_wav_duration_and_repair`（REQ-19，含崩溃子进程再入与 WAV 修复）/ `it_audio_multitrack_ffprobe_fft_all_tracks`（REQ-17 全音轨模式，§4 点名用例的姊妹用例） |
| `docs/milestones/m6-red-report.md` | 新增 | 本报告 |

未改动：生产 src（sr-audio/sr-pipeline/sr-capture 等零改动）、specs/、docs/spikes/、既有测试（audio_isolation.rs / multi_route.rs / route.rs / common/mod.rs 逐字未动——本任务约束「不改既有测试」；新 helper 全部内聚在 audio_m6.rs，沿 M3/M4/M5 先例）、`tests/sr-itests/Cargo.toml`（**零新依赖**）。

## 2. REQ → 测试 → 文件映射（§4 命名优先）

| REQ | 测试名 | 层 | 断言要点 |
|---|---|---|---|
| REQ-15 系统音频 | `it_audio_device_loopback_rms`（§4 点名） | L2 | 默认渲染端点设备环回录 6s（新 API `capture_device_loopback`）：48k/2ch、时长钉齐 ±48 帧；**系统轨非静音 RMS ≥0.002**；A(440)+B(880) 同播时两声均在（各 ≥本底+20dB）且电平差 ≤10dB（S3 场景 b 对照；本机设备环回实测差 −0.4dB）；stats 打印 |
| REQ-16 麦克风 | `it_mic_capture_present_or_skip`（§4 点名） | L2 | ①枚举采集设备（新 API `list_capture_devices`）；无设备 → 反证「设备不存在/不可打开必须显式报 `MicUnavailable` 且 ≤5s，不得静默成功」后记 **BLOCKED**；②设备存在 → 默认设备录 6s（48k/2ch/±48 帧）+ 1200Hz 经默认输出播放（§4 手法）要求声学回路含该音调（≥本底+20dB）；捕获为数字静音级（RMS < 1e-4）⇒ 无声学回路 ⇒ **BLOCKED**（§4 环境缺失）；有信号但无目标音调 = FAIL |
| REQ-19 纯录音 | `it_audio_only_wav_duration_and_repair`（§4 点名） | L2 | 相位 A（正常停止）：App(440)+App(880) 双源 → `wav_paths` 与 sources **同序同长**、各独立 WAV；pcm_s16le/48k/2ch；时长≈录制段 ±1.0s；decode clean；RMS ≥0.005；各自内容隔离（目标 ≥−30dB、非目标 ≤−40dB 相对）；「停止即成品」（wait_finalized ≤15s 即 Ok）。相位 B（进程级崩溃）：本测试二进制再入「崩溃录制子进程」→ 2.5s 后 `taskkill /F` → 残留未封口 WAV → **注入 2 字节尾部余量**（确定性覆盖裁切分支）→ `repair_wav`：`trimmed_bytes==2`、文件长度反映裁切、帧对齐、幂等二跑（trimmed==0、文件不变）；修复后时长≈子进程存活段 ±1.5s、`repair.frames` 与 ffprobe ±0.1s、s16/48k/2ch、decode clean、非静音 |
| REQ-17 全音轨模式（补齐部分） | `it_audio_multitrack_ffprobe_fft_all_tracks`（§4 点名 `it_audio_multitrack_ffprobe_fft` 的**姊妹用例**，见 §8-1） | L2 | 音源 = [App(440), System, App(880)]（目标窗口为测试自建窗口、无天然应用音源）→ 成品 MKV：恰 1 视频（h264/640×360）+ **恰 3 条独立音轨**（aac/48k/2ch；未启用麦克风 ⇒ 无 mic 轨）；时长≈录制段 ±2s；**内容级分类（不依赖轨序）**：恰 1 系统轨（两声均在、差 ≤10dB）+ 恰 1 应用轨只含 440（880 ≤−40dB）+ 恰 1 追加应用轨只含 880（440 ≤−40dB）——轨组织与内容双证「各独立轨、互不混音」 |

## 3. API 契约清单（测试即契约；M4 契约的 M6 扩展面，供绿阶段实现）

### 3.1 `crates/sr-audio` —— `pub mod wasapi`（新增符号）

```rust
/// REQ-15：默认渲染端点设备环回，同步录 N 秒（S3 apploop `--device-loopback` 同构）。
/// 复用与 capture_process_loopback 同一采集核：48k/f32/立体声 autoconvert、时长钉齐
/// （±48 帧）、墙钟续流；默认端点缺失/激活失败 → LoopbackUnavailable（L2 → BLOCKED）。
pub fn capture_device_loopback(duration: Duration) -> Result<CapturedPcm, CaptureError>;

/// REQ-16：采集（麦克风）设备信息。
#[derive(Debug, Clone)]
pub struct AudioDeviceInfo {
    pub id: String,          // 请求捕获时回传（实现自定格式，须稳定）
    pub name: String,        // 友好名（报告/UI）
    pub is_default: bool,    // 默认采集端点标识
}

/// REQ-16：枚举采集设备。无设备 → Ok(空列表)（明确报告，不静默；实现若以 Err 表达
/// 亦须为环境缺失类，测试对枚举 Err 记 BLOCKED）。
pub fn list_capture_devices() -> Result<Vec<AudioDeviceInfo>, CaptureError>;

/// REQ-16：麦克风捕获（同步录 N 秒；device = AudioDeviceInfo.id）。
/// 设备不存在/不可打开 → CaptureError::MicUnavailable，**快速返回**（测试上限 5s），
/// 不得静默零包/静默成功（「无设备时明确不可用（SKIP 不得静默）」）。
pub fn capture_microphone(device: &str, duration: Duration) -> Result<CapturedPcm, CaptureError>;

// CaptureError 增补变体（M6）：
//   MicUnavailable { detail: String }   // 无设备/设备不可打开——明确不可用
//   （设备环回沿用既有 LoopbackUnavailable；其余既有变体不动）
```

### 3.2 `crates/sr-audio` —— `pub mod wav`（REQ-19；M1 占位骨架落地）

```rust
/// 崩溃修复结果。
#[derive(Debug, Clone)]
pub struct WavRepair {
    pub frames: u64,         // 修复后完整帧数（s16 立体声；frames/48000 ≈ 文件时长）
    pub trimmed_bytes: u64,  // 尾部不完整样本裁切字节数（0..3）
}

#[derive(Debug)]
pub enum WavError { /* 变体实现自定（测试仅 Debug 打印，不匹配变体） */ }

/// 头部重写（RIFF/data 长度封口）+ 尾部不完整样本裁切；**幂等**（二次调用无操作，
/// trimmed_bytes=0、文件不变）。崩溃残留 = 数据已追加而头部长度为占位的半成品。
pub fn repair_wav(path: &Path) -> Result<WavRepair, WavError>;
```

WAV 写侧（实现自由，测试仅通过产物断言）：流式写入——头部占位 + 数据追加，正常停止
时封口；48kHz/s16/立体声（管线固定格式的 s16 归档）；崩溃损失 ≤ 写缓冲（测试容差
1.5s 的余量内）。注：未封口文件的 ffprobe 时长可能仍按**文件大小**推断（scratch 实测），
故修复前状态仅作诊断打印；修复的必测语义 = 时长准确性 + 裁切记账 + 帧对齐 + 幂等。

### 3.3 `crates/sr-pipeline` —— `pub mod route`（REQ-19 新增；**既有 `start_route`/`start_audio_route`/`RouteConfig`/`AudioRouteConfig`/`MultiRouteConfig` 逐字不动**）

```rust
/// M6 纯录音配置（§3.4 的来源列表形态；无视频/WGC/编码器参与）。
#[derive(Clone, Debug)]
pub struct AudioOnlyConfig {
    pub sources: Vec<sr_audio::wasapi::AudioSourceSpec>,
    pub output_dir: PathBuf,
}

/// 纯录音句柄：request_stop 立即返回「已请求停止」；wait_finalized 返回成品
/// （**停止即成品**：WAV 无 concat/后台收尾；测试 15s 上限内即 Ok 视为满足）。
pub struct AudioOnlyHandle { /* … */ }
impl AudioOnlyHandle {
    pub fn request_stop(&self);
    pub fn wait_finalized(&self, timeout: Duration) -> Result<AudioOnlyFinalized, RouteError>;
}

/// 与 sources **同序同长**；每来源一条独立 WAV（48kHz s16 立体声）位于 output_dir 内
/// （可位于其子目录；测试递归发现）。文件名实现自由（测试不断言）。
#[derive(Clone, Debug)]
pub struct AudioOnlyFinalized { pub wav_paths: Vec<PathBuf> }

pub fn start_audio_only(cfg: AudioOnlyConfig) -> Result<AudioOnlyHandle, RouteError>;
// 错误映射：环回不可用 → RouteError::LoopbackUnavailable（L2 → BLOCKED）；其余 Err → 测试 FAIL。
```

### 3.4 `start_audio_route` 的 System/Mic 源（REQ-15/16 路由侧；**行为扩展，无新符号**）

- `AudioSourceSpec::System`：默认渲染端点设备环回为一条独立音轨（REQ-15；v1 仅默认端点）。
- `AudioSourceSpec::Mic { device }`：采集设备为一条独立音轨（REQ-16；无设备/不可打开 →
  路级明确报错——具体变体实现自定，本红未钉）。
- 全音轨轨组织 = 给定来源列表各一条独立轨、互不混音（REQ-17）；**不得改动既有 config
  结构体字段**（M3/M4/M5 测试以全字段字面量构造——增字段会破坏既有测试编译）。
- 实现建议（非强制）：复用 §3.1 同一采集核，将「进程环回/设备环回/麦克风」抽象为采集
  目标枚举；流式路由路径与内存 API 行为不分叉（M4 红 §8-4 同思路）。

### 3.5 契约要点（行为层）

- **BLOCKED 分流**：`LoopbackUnavailable`（设备环回/纯录音环回）→ BLOCKED；设备枚举 Err
  → BLOCKED；麦克风数字静音级无声学回路 → BLOCKED（§4 环境缺失，本机即此分支）；
  `EncoderUnavailable`/`ConsentDenied`（路由用例）→ BLOCKED；其余 Err = 断言失败。
- **系统轨判定阈值**：存在性 ≥本底+20dB、两声差 ≤10dB（S3 对照 0.62dB；本机设备环回
  实测 −0.4dB）；隔离 ≤−40dB（S3 预锁定，沿 M4；m4-green §6-1 的 ±5dB flake 注记沿用）。
- **全音轨分类不依赖轨序**：以 440/880 存在性二元组分类（系统轨含两声、应用轨各自只含
  自身），避免过度钉死轨顺序/命名/元数据。
- **WAV 契约**：`wav_paths` 与 sources 同序（测试以 440/880 内容交叉验证顺序）；时长口径
  相位 A ±1.0s（含启动余量）/ 相位 B ±1.5s（taskkill 生效延迟经后置测量消除，见 §7）；幂等。
- **崩溃子进程协议**（测试内部机制，非产品契约）：环境变量 `SR_M6_WAV_CHILD_DIR/_SRC_PID/
  _SRC_CREATED` 触发本测试二进制再入（`--exact … --ignored`）；marker 文件 `child.started`
  回传就绪时刻；失败写 `child.error` 并 exit(2)；父进程 taskkill 后递归发现残留 WAV。

## 4. 运行命令与门控

```text
# L2（显式执行；须交互式用户会话；$env:SESSIONNAME='Console' 需可见）
cargo test -p sr-itests --test audio_m6 -- --ignored --nocapture
# 单用例示例
cargo test -p sr-itests --test audio_m6 it_audio_device_loopback_rms -- --ignored --nocapture

# 全仓（红阶段 exit 101 属预期：仅 audio_m6 的新符号 unresolved）
cargo test --workspace
```

BLOCKED 触发点：非交互会话（SESSIONNAME 缺失/非 Console）；锁定 n8.1 构建缺失；
设备环回/纯录音环回激活失败；麦克风无设备或无声学耦合（本机现状）；路由用例另有
EncoderUnavailable（§9 T1）/ConsentDenied（WGC）。文件内 `GATE` Mutex 使 4 个 L2 用例
串行（M4 R5/M5 R6 先例；全音轨用例同时开 3 个采集客户端，叠加并发会超出 S3 已验证面）。

## 5. 依赖与锁定构建

- **零新增依赖**：audio_m6.rs 仅使用 sr-itests 既有 deps（sr-audio/sr-pipeline/rustfft 6.4.1/windows 0.62 的既有 features；本文件无直接 windows-API 调用）。`tests/sr-itests/Cargo.toml`、`Cargo.lock` 零改动。
- ffmpeg/ffplay/ffprobe：锁定 `tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin`（§8 冻结；经 `CARGO_MANIFEST_DIR` 相对定位后 canonicalize，M3/M4/M5 同款）。测试音 WAV 用 `sine=...,volume=0.25` 合成（M4 R2 防削波谐波落位）。

## 6. 红证据（实际执行；全部日志在 `%TEMP%\opencode\`）

### 6.1 改动前基线（对照）

| 命令 | 结果 |
|---|---|
| `cargo test --workspace` | **exit 0**（`m6_baseline_ws.log`） |

### 6.2 定向红（新测试目标）

```text
cargo test -p sr-itests --test audio_m6 --no-run        → exit 101（m6_red_v2_targeted.log）

error[E0432]: unresolved imports `sr_audio::wasapi::capture_device_loopback`,
             `sr_audio::wasapi::capture_microphone`, `sr_audio::wasapi::list_capture_devices`,
             `sr_audio::wasapi::AudioDeviceInfo`                      （audio_m6.rs:54）
error[E0432]: unresolved import `sr_audio::wav::repair_wav`       （audio_m6.rs:57）
error[E0432]: unresolved imports `sr_pipeline::route::start_audio_only`,
             `sr_pipeline::route::AudioOnlyConfig`, `sr_pipeline::route::AudioOnlyHandle` （audio_m6.rs:59）
error[E0599]: no variant named `MicUnavailable` found for enum `CaptureError`  （×2）
error: could not compile `sr-itests` (test "audio_m6") due to 5 previous errors
```

**红因确认**：恰 5 个 error，全部为「新测试引用的预期公共符号/枚举变体 unresolved（实现缺失）」；无其它类别错误；新测试代码 **0 警告**（初版曾因未标注 `devices` 类型产生 4 条 `!` 推断级联 E0599 + 1 条 E0608（另 5 条警告）——已通过 `Vec<AudioDeviceInfo>` 标注消除，属修测试自身缺陷，见 §7-R12）。`cargo test --workspace` 同错误集、**exit 101**（`m6_red_v2_ws.log`）。

### 6.3 既有绿不退化

| 命令 | 结果 |
|---|---|
| `cargo test --workspace --exclude sr-itests` | **exit 0**：screen_rec_lib 7、channel 1、pacer 1、ffmpeg_args 1、output_naming 1、silence_pad 1、drift 1 —— 共 13 passed（`m6_l1_regress.log`） |
| `cargo test -p sr-itests --test route --test audio_isolation --test multi_route --test enum_targets --test enum_audio --test shot --no-run` | Finished、**exit 0**（M1–M5 全部 L2 目标可编译；`m6_l2_compile_regress.log`） |

### 6.4 Scratch 第 2 层：stub-编译自检（仓库外副本 `%TEMP%\opencode\sr-m6-red-ws`，不进仓库）

对测试文件原文（SHA 比对与仓库一致）在「仓库副本 + M6 契约桩」下做全量 typecheck：
`cargo test -p sr-itests --test audio_m6 --no-run --offline` → **exit 0、0 警告、可执行产出**
（`m6_stub_final.log`；注释微调后的最终 SHA 由 §6.5 假绿构建/§6.8 复核同法覆盖）。
桩仅存在于副本（wasapi 增 `capture_*`/`AudioDeviceInfo`/`MicUnavailable`，
wav 模块落地，route 增 `AudioOnly*`）。

### 6.5 Scratch 第 2 层（续）：dress rehearsal（副本内 scratch-only「假绿」实现）

在副本内以 scratch-only 假实现（墙钟节拍写 s16 正弦的 `start_audio_only` + 真实语义的
`repair_wav`）运行**测试文件原文**：

| 轮次 | 结果 |
|---|---|
| 全文件 4 用例（`m6_fake_green_all.log`） | `it_audio_only_wav_duration_and_repair` **PASS**；`it_audio_device_loopback_rms` BLOCKED（桩）；`it_mic_capture_present_or_skip` BLOCKED（桩）；`it_audio_multitrack_ffprobe_fft_all_tracks` 如实在「System/Mic 音源路由属 REQ-15/16（M6）：M4 仅实现 App 隔离源（TargetInvalid）」处 FAIL——**预期红因**；GATE 串行 20.68s 无交错 |
| 最终文件单跑 WAV 用例（`m6_fake_green_wav_final.log`） | **PASS（13.82s）**：相位 A 双 WAV 5.205/5.203s vs 录制段 5.001s、停止→成品 0.00s；相位 B 残留 746100 字节（+2 注入）→ `repair trimmed_bytes=2`、文件 746102→746100、时长 3.886s vs 存活段 3.894s（**Δ=0.008s**，容差 1.5s）、二跑幂等无操作、RMS 0.25 |

### 6.6 Scratch 第 1 层：时序敏感 helper 实跑自检（仓库外 `%TEMP%\opencode\sr-m6-red-scratch\helper`，exit 0）

| 检查 | 实测（原文关键行） |
|---|---|
| `parent`（崩溃协议：再入/marker/taskkill/注入/修复/ffprobe/解码） | `PARENT CHECKS DONE`：残留 656684 字节；注入 2 字节 → `repair frames=164160 trimmed_bytes=2`；修复后时长 3.420s vs 存活段 2.602s → **发现 taskkill 生效延迟 ≈0.8s**（见 §7-R4）；解码 RMS 0.25（`m6_scratch_parent.log`） |
| `mix`（合成 3 音轨 MKV → 逐轨解码 → 分类（测试同代码）） | 音轨0：440=47.4/880=−75.1 ⇒ (true,false)；音轨1：41.3/41.8 ⇒ (true,true) 差 0.5dB；音轨2：−71.1/47.8 ⇒ (false,true)；分类 **(1,1,1)**（`m6_scratch_mix.log`） |
| 设备环回 ground truth（S3 apploop `--device-loopback`，真实端点） | 6.009s / 288446 帧（ratio 0.998）；440=47.3dB、880=47.7dB、本底 14.9dB ⇒ 存在性差 32.4dB、两声差 −0.4dB；RMS_all 0.1638 / RMS_mid 0.1675（`m6-scratch-devlb\*`）——标定 DEV_RMS_MIN=0.002（≈80× 余量）与 MIX_PARITY_DB=10 |
| 麦克风 ground truth（wasapi 直采，真实设备） | 采集集合 1 个；48k/f32/2ch autoconvert OK；录 5.978s / RMS **6.7e-5**（数字静音级）；1200Hz 播放中 1200 分量 = −31.7dB vs 本底 0.0 ⇒ 不存在（`m6-scratch-mic\mic.log`）——默认渲染/采集为同一蓝牙耳机，**无声学回路**（见 §6.7） |

### 6.7 设备状况（实际，2026-10-17；与 S3 时代不同——**重要环境变更**）

| 项 | 实测值 | 来源 |
|---|---|---|
| 默认渲染端点 | `耳机`（headset），原生 44100Hz / 2ch / int16 | apploop probe（`m6-scratch-devlb\probe.log`） |
| 默认采集端点 | `耳机 (Baseus Bowie W04 Plus)`，原生 16000Hz / 1ch | apploop probe + scratch mic 采集 |
| S3 报告对照 | S3 时代：默认输出 `LS24A600N` 48kHz；**无采集端点（0x80070490）** | s3-audio-isolation.md §1 |
| 声学耦合 | **不存在**：1200Hz 播放（耳机）时麦克风 RMS=6.7e-5、1200 分量不存在 | scratch mic 检查 |
| 推论 | `it_mic_capture_present_or_skip` 在本机将走「设备存在 → 数字静音级 → BLOCKED」分支（正路径留扬声器—麦克风耦合机器复跑）；设备环回用例不受影响（实测量级健康） | 本报告 §6.6 |

### 6.8 最终冻结复核

对最终文件（注释微调后）复跑：定向红 `exit 101`（同 5 error、0 警告，`m6_red_v2_targeted.log`）+
工作区红 `exit 101`（`m6_red_v2_ws.log`）+ 副本内最终 SHA 的 stub-编译/假绿构建 `exit 0 / 0 警告`
（`m6_fake_green_wav_final.log`）+ dress rehearsal WAV 用例 `PASS`（§6.5 末行）。**报告与本清单一致，无未列出的私有改动**（仓库 mtime 扫描：本会话仅触碰 audio_m6.rs 与本报告）。

## 7. 关键设计取舍

| # | 取舍 | 依据 |
|---|---|---|
| R1 | REQ-17 全音轨部分落为**姊妹用例** `…_all_tracks`（不扩改 audio_isolation.rs 既有隔离用例） | 任务约束「不改既有测试」；m4-red §8-1 明示「扩展该用例或新增姊妹用例」二选一 |
| R2 | 阈值：系统轨存在性 ≥本底+20dB、两声差 ≤10dB；隔离 ≤−40dB（沿用）；WAV RMS ≥0.005（M5 同款）；设备环回 RMS ≥0.002；麦克风数字静音界 1e-4 | S3 预锁定 + §6.6 本机实测（设备环回 −0.4dB/32.4dB；麦克风 6.7e-5）；−40dB 维持 spec/S3 语义，M4 flake 注记沿用（m4-green §6-1） |
| R3 | 崩溃修复用**真实子进程强杀**（本测试二进制再入），而非手工构造坏 WAV | L2 忠实度：验证「崩溃残留 → 可修复」整链；env+marker 协议由 §6.6 parent 自检验证 |
| R4 | **taskkill 生效延迟实测 ≈0.8–1.0s**（two independent scratch observations: helper Δ0.82s、假绿首轮 Δ1.01s）→ 存活段在 `taskkill` 生效完成后测量 | 消除把 kill 延迟算进 WAV 时长的系统偏差；修后假绿两轮 Δ=0.019s / 0.008s，容差 1.5s 余量 ≈75× |
| R5 | 尾部不完整样本采用**确定性注入**（按 size%4 补至余 2 字节） | taskkill 时点随机、可能恰好帧对齐（helper 实测 1920B/块恒对齐）；不注入则裁切分支可能从未被触达 |
| R6 | 麦克风「设备存在但数字静音级」→ BLOCKED（而非 FAIL） | §4 环境缺失语义（沿 M2 负坐标先例）；前提断言（格式/时长）先于该分支，且异常实现「静音但记账正常」无法与本机耳机环境区分——如实记录于 §8-2 |
| R7 | `wav_paths` 同序契约以内容交叉验证（源0=440、源1=880） | 钉死「与 sources 同序同长」的可测语义，不依赖文件名/轨序 |
| R8 | 全音轨分类不依赖轨序/元数据（存在性二元组） | §3.4 未定义轨顺序与命名；避免过度钉死实现细节 |
| R9 | 残留 WAV 递归发现（含子目录） | 容忍实现的会话临时子目录布局（REQ-24/D12 语义） |
| R10 | 修复**幂等**纳入断言（二跑 trimmed=0、文件不变） | REQ-19「停止即成品/崩溃恢复」族语义；防止修复非幂等导致重复损坏 |
| R11 | 零新增依赖；文件内 GATE 串行；helper 只增（内聚本文件） | M4/M5 先例；全音轨用例 3 采集客户端并列，串行避免超出 S3 已验证并发面 |
| R12 | 红阶段自查修复测试自身缺陷 2 处：①`devices` 未标注类型产生 4 条 `!` 推断级联 E0599 + 1 条 E0608（+5 条警告）（已标注 `Vec<AudioDeviceInfo>`，红因收敛到纯符号缺失）；②kill 时点前置于 taskkill（见 R4） | 保证「仅预期未解析符号」的红证据口径；两处均在 scratch 验证后回移 |

## 8. 未决问题（返回 orchestrator；绿阶段须知）

1. **§4 命名与既有测试的取舍**：§4/§5 点名的 `it_audio_multitrack_ffprobe_fft` 已被 M4 隔离部分占用（audio_isolation.rs）。本红在不改既有测试的约束下新增姊妹用例 `…_all_tracks`。若编排者要求单一 §4 名称承载两模式，需在后续轮次合并（属测试编排决策）。
2. **麦克风环境变了且无法自动区分环境/实现**：本机现有 Baseus 蓝牙耳机采集端点（S3 的「无麦克风」结论已过期），但耳机无声学回路；「捕获正常但 RMS 数字静音」分支记 BLOCKED 存在理论盲区（静默但记账正常的实现无法与本机环境区分）。已在 §3.5/§7-R6 记录；建议 M6 绿在本机以 BLOCKED 收尾 mic 正路径，另寻耦合机器复跑；如编排者要求更严，可协商改为 FAIL 或引入设备级判定（实现侧可提供 `capture_microphone` 的静音判别扩展）。
3. **REQ-17「追加应用按根去重；快照语义（REQ-4）」无对应 §4 测试**，本红未覆盖（身份/策略层，M4 红 §8-10 同域）；建议绿阶段 L1 规则单测或归 M8。
4. **路由侧 Mic 轨（`AudioSourceSpec::Mic`）未内容级覆盖**：需耦合环境（同 §8-2）；契约已记载（§3.4），绿阶段须实现 System/Mic 源（M4 显式 TargetInvalid 路径将被替换——注意保持 M3/M4/M5 测试字面量可编译）。
5. **3 客户端并列采集为新并发面**：全音轨用例 = 2 进程环回 + 1 设备环回；S3 仅验证 2 客户端。若绿阶段遇引擎上限（激活失败/流异常），属产品发现，须评估 REQ-17 全音轨承诺（app+system+mic+追加应用天然要求 N 客户端并列）。
6. **设备环回与 S3 的跨路径电平口径**：S3 时代设备路径比进程路径低 ≈5.4dB；本机现状设备路径 RMS≈0.17（几乎无衰减）。阈值已按「数字静音 vs 实测」量级标定（0.002），不依赖该历史差值；多端点/独占模式仍为 REQ-15 v1 明确不支持。
7. **`repair_wav` 的 WavError 变体未钉**（测试仅 Debug 打印）；`WavRepair.frames` 语义已钉（完整帧数）——绿阶段实现自由。
8. **`capture_device_loopback` 与设备环回路由源的复用建议（非强制）**：绿阶段若实现为「采集目标枚举 + 同核」可避免行为分叉（M4 红 §8-4 先例）；若选择独立实现，须保持 48k/2ch/时长钉齐一致。
9. **SESSIONNAME 运行注记**：本工具链 shell 不继承 SESSIONNAME；L2 实跑需 `$env:SESSIONNAME='Console'` 或交互式控制台终端，否则按 §4 BLOCKED（非测试缺陷）。
10. **时间预算未断言**（沿 M3/M4/M5 先例）：设备环回激活时延、纯录音启动时延未设断言；`wait_finalized` 15s/120s 超时隐含宽松上限。相位 B 的 2.5s 录制窗口与 1.5s 容差按 scratch 实测（§6.5/§7-R4）标定。
11. **M4 已知 flake 的关联**：全音轨用例的应用轨隔离断言（≤−40dB）与系统轨存在性（+20dB）沿用 M4 阈值体系；m4-green §6-1 的负载相关波动若复现，按该注记校准（不放宽语义），本红不预调。

## 9. 结论

- M6 红测试就位：4 个 L2 用例（§4 点名 ×3 + 全音轨姊妹用例），`#[ignore]` 门 + BLOCKED=fail + 文件内 GATE 串行；新 API 契约（§3）按 spec §3.2/§3.3 拟定并钉死行为语义。
- 红因正确：`cargo test --workspace`（及定向命令）**exit 101，仅 5 个预期未解析符号/变体错误、0 警告**（§6.2/§6.8）；既有 L1 全绿、M1–M5 L2 目标全部可编译（§6.3）。
- 双层自检 + dress rehearsal：stub 全量 typecheck 0 错 0 警；WAV 用例在 scratch 假绿下 **PASS**（含崩溃相位 Δ=0.008s）；崩溃协议与 3 轨分类经第 1 层实跑标定（§6.4–§6.6）。
- 未写任何生产代码；未 git 提交；报告数字全部来自本机实跑日志（路径已列）。
