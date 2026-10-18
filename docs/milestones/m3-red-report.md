# M3 红测试报告（TDD 红阶段：单路视频管线 REQ-9..21/24 的 M3 任务范围）

- 日期: 2026-10-16（第二轮续写：用量限额重置后恢复；恢复时 route.rs 已完整落盘，本轮完成 scratch 自检、stub 全量类型检查、红证据与本报告）
- 规格: specs/001-screen-recorder.md §2.3（REQ-9..13）、§2.5（REQ-20/21/24）、§3.1/§3.2/§3.3（管道协议/ffmpeg 基准/接口）、§4、§5 M3 行、§8 M0 冻结项
- Spike 依据: docs/spikes/s1b/s1b-pipe-faults.md §7（8MiB/EOF 优雅停止/232/109→E_SIDECAR_EXIT/0 字节段排除/taskkill 退出码 1）、s1（画布/定速）
- 角色: Test Writer（红阶段）。本任务只写测试，未写任何生产实现（sr-capture/sr-pipeline 仍为 M1-A 占位骨架）。
- 环境实测: Windows 11 25H2，build **10.0.26200.9457**，SESSIONNAME=Console；rustc/cargo 1.99.0 msvc。
- 到场状态: M2 绿阶段已由后续执行者完成（`m2-green-report.md`、`crates/sr-shot/src/screenshot.rs` 已实现）——`tests/…/shot.rs` 现**可编译**为正确状态；本报告红集仅含 M3 新符号。

## 1. 交付物

| 文件 | 性质 | 内容 |
|---|---|---|
| `crates/sr-capture/tests/pacer_cfr.rs` | 新增 | L1 ×1：`unit_pacer_cfr_dup_drop`（帧槽节奏器纯逻辑） |
| `crates/sr-pipeline/tests/ffmpeg_args_snapshot.rs` | 新增 | L1 ×1：`unit_ffmpeg_args_snapshot`（编码命令关键要素） |
| `crates/sr-pipeline/tests/output_naming.rs` | 新增 | L1 ×1：`unit_output_naming`（sanitize/时间戳/碰撞分配） |
| `tests/sr-itests/tests/route.rs` | 新增 | L2 ×5：单路非黑帧 / MKV 有效性 / 分段收尾 / 强杀可播 / 光标开关 |
| `tests/sr-itests/Cargo.toml` | 追加 | `sr-pipeline` path 依赖 + windows feature `Win32_System_Diagnostics_ToolHelp`（强杀用例的 sidecar 子进程发现） |
| `docs/milestones/m3-red-report.md` | 新增 | 本报告 |

未改动：生产 src（sr-capture/sr-pipeline 仍为骨架）、specs/、既有测试（M1/M2 全部逐字未动；common/mod.rs 本轮零改动，新 helper 全部内聚在 route.rs）、docs/spikes/。未 git 提交。无新增外部 crate。

## 2. REQ → 测试 → 文件映射（任务给定名 vs §4 名的偏差见 §8 未决 1）

| REQ | 测试名 | 层 | 文件 | 断言要点 |
|---|---|---|---|---|
| REQ-9 | `it_single_route_frames_nonblank` | L2 | route.rs | 自建运动图案窗口（专用泵线程 33ms 定时器：移动亮条+秒级翻转块）录制 7s：成品 MKV h264、时长≈录制段（±2s）、3fps 采样帧全部非黑（亮度 std≥2）、运动（≥30% 相邻对差≥2 且最大≥10）、段≥1 且非 0 字节 |
| REQ-10 | `unit_pacer_cfr_dup_drop` | L1 | pacer_cfr.rs | 30fps 完美节拍→槽 0..N 顺次 Write；抖动定速不乱槽；同槽第二帧 Drop；已写槽的迟到帧 Drop；**乱序批量送达按时间戳指派槽**（t2 先到 W2、t1 后到仍 W1）；跨槽大跳不误丢（补帧属 REQ-25/26） |
| REQ-11 | `it_cursor_toggle_degrades` | L2 | route.rs | cursor=true/false 两配置录制均成立：成品存在、h264、时长≥3s、非黑且运动、两路径互异（降级语义需故障注入，见 §8 未决 3） |
| REQ-20 | `unit_ffmpeg_args_snapshot` | L1 | ffmpeg_args_snapshot.rs | §3.3 基准关键要素（存在性+语义相邻序列，不锁 argv 顺序）：-hide_banner/-nostdin；rawvideo/nv12/-s WxH/-framerate→管道；f32le/48k/ac2→管道；-map 0:v(+每音频 map)；-c:v 链名透传；-g=2*fps（30→60、25→50）；-force_key_frames expr:gte(t,n_forced*5)；aac 192k（无音频时收缩）；segment/-segment_time/-reset_timestamps/matroska/-segment_list；输出 pattern 收尾 |
| REQ-20/24 | `it_route_pipeline_mkv_valid` | L2 | route.rs | 固定画布 640x360/30fps（D6）；恰 1 视频轨 0 音频轨；关键帧最大间隔 ≤2.5s（-g 60 实测 2.0）；成品命名 `{yyyyMMdd_HHmmss}_{sanitize(目标名)}.mkv` 且在输出目录内 |
| REQ-20/21 | `it_segments_finalize_concat` | L2 | route.rs | ≥2 段出现后：录制中已闭合段独立可播（时长>0+整文件解码）；收尾后 ffconcat 清单存在；成品时长≈Σ段（±0.75s）≈录制段（±2s）；成品整文件可解码 |
| REQ-21（基础版） | `it_crash_kill_segments_playable` | L2 | route.rs | 录 ~17s（前两段确定闭合）→ Toolhelp32 发现 ffmpeg sidecar 子进程 → taskkill /T /F → `wait_finalized` 须 Err(SidecarExit)；闭合快照段仍可播、不从清单消失（完成段永不丢弃）；段清单条目全部非 0 字节（0 字节壳排除） |
| REQ-24 | `unit_output_naming` | L1 | output_naming.rs | sanitize：非法/控制字符不残留、安全字符原样、结尾点/空格处理、保留名（CON/NUL/COM1-9/LPT1-9，大小写不敏感）转义；format_timestamp：显式偏移注入的 yyyyMMdd_HHmmss（unix0/±偏移/闰年锚点/形状）；next_available：纯分配器、碰撞 _1/_2、永不返回已存在路径、目录状态纯函数 |

（任务文本把 args/pipeline 两用例标在 REQ-19 下；按 spec §4 实际映射为 REQ-20 行（`unit_ffmpeg_args_snapshot`），`it_route_pipeline_mkv_valid` 为任务新增端到端用例（§4 无对应名）——记账见 §8 未决 1/2。）

## 3. API 契约清单（测试即契约；模块名沿用 M1-A 骨架，`route` 为测试假定新增模块）

### 3.1 `crates/sr-capture` —— `pub mod pacer`（骨架已有模块名）

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum FrameAction {
    Write { slot: u64 },   // CFR 输出槽位 n（相对 T0）
    Drop,                  // 重复/迟到
}

pub struct FramePacer { /* … */ }
impl FramePacer {
    pub fn new(fps: u32, qpc_ticks_per_sec: i64) -> Self;
    /// 源时间戳（WGC SystemRelativeTime，QPC ticks）→ 槽决策。首帧建立 T0。
    /// 语义：槽位=时间戳纯函数（floor((t-T0)·fps·qpc⁻¹)）；该槽未写→Write，已写→Drop。
    pub fn on_frame(&mut self, source_qpc_ticks: i64) -> FrameAction;
}
```

### 3.2 `crates/sr-pipeline` —— `pub mod sidecar`（骨架已有）

```rust
pub struct VideoPipeSpec { pub width: u32, pub height: u32, pub fps: u32, pub pipe_path: String }
pub struct AudioPipeSpec { pub pipe_path: String }   // f32le/48k/2ch 固定（§3.3）

pub fn build_encode_args(
    video: &VideoPipeSpec,
    audio: &[AudioPipeSpec],
    video_encoder: &str,          // D4 链探测结果透传（如 "h264_qsv"）
    segment_secs: u32,            // 5
    segment_list: &Path,          // ffconcat 清单
    output_pattern: &Path,        // 段输出 pattern（如 out_%05d.mkv）
) -> Vec<String>;                 // 不含 exe 本体；-nostdin/-hide_banner 由构建方加入
```

### 3.3 `crates/sr-pipeline` —— `pub mod naming`（骨架已有）

```rust
pub fn sanitize_target_name(raw: &str) -> String;
//   钉死：`\/:*?"<>|` 与控制字符不残留；安全字符（含 Unicode）原样；结尾 '.'/' ' 处理；
//   保留名（CON/PRN/AUX/NUL/COM1-9/LPT1-9，忽略大小写）转义为非保留且非空。

pub fn format_timestamp(unix_ms: u64, utc_offset_minutes: i32) -> String;
//   yyyyMMdd_HHmmss（15 字符）。偏移显式注入（本地化确定性与可测性；实现侧取系统 TZ
//   后换算，测试不依赖宿主时区）。

pub fn next_available(dir: &Path, stem: &str, ext: &str) -> PathBuf;
//   纯分配器（不创建文件）：无碰撞→{stem}.{ext}；有→{stem}_1.{ext}、{stem}_2…（后缀
//   形状为测试假定）；返回时目标不存在（禁止覆盖）；目录状态纯函数。
```

### 3.4 `crates/sr-pipeline` —— `pub mod route`（**测试假定新增**；§3.2 骨架无此模块，归属见 §8 未决 4）

```rust
pub enum RouteTarget { Window { hwnd: u64 }, Monitor { index: u32 } }   // Region 按 §3.4 由实现补全
pub struct VideoCfg { pub canvas_w: u32, pub canvas_h: u32, pub fps: u32, pub cursor: bool }
pub struct RouteConfig { pub target: RouteTarget, pub video: VideoCfg, pub output_dir: PathBuf }

#[derive(Debug)]
pub enum RouteError {
    EncoderUnavailable, // E_ENCODER_UNAVAILABLE（§9 T1；测试→BLOCKED）
    ConsentDenied,      // E_CONSENT_DENIED（WGC 授权；测试→BLOCKED）
    SidecarExit,        // E_SIDECAR_EXIT（sidecar 非零退出/被终止，保留分段；测试强匹配）
    WaitTimeout,        // 收尾等待超时（本红测试未直接引用，契约记载）
    // …其余变体实现自定（测试不引用）
}

pub struct Finalized {
    pub final_path: PathBuf,     // 成品 MKV：{format_timestamp}_{sanitize(目标名)}.mkv，位于 output_dir 内
    pub segment_list: PathBuf,   // ffconcat 段清单（收尾输入）
    pub segments: Vec<PathBuf>,  // 参与收尾的段（全部 >0 字节）
}

pub struct RouteHandle { /* … */ }
impl RouteHandle {
    pub fn request_stop(&self);                                                  // 「已请求停止」：立即返回（T5）
    pub fn wait_finalized(&self, timeout: Duration) -> Result<Finalized, RouteError>;
    pub fn segments(&self) -> Vec<PathBuf>;                                      // 当前已写出段；0 字节壳不列入；
                                                                                  // Err 后仍可查询（清单视角）
}

pub fn start_route(cfg: RouteConfig) -> Result<RouteHandle, RouteError>;
//   Ok 语义：启动协议走到「输出开始」（管道建立→sidecar spawn→采集预热→编码器就绪→
//   公共 T0，§3.1）；管道缓冲 8MiB（S1b 冻结）；sidecar=锁定 ffmpeg.exe（n8.1 release 构建）。
```

### 3.5 契约要点（行为层）

- **错误策略**：start_route 的 EncoderUnavailable/ConsentDenied 在 L2 中按环境缺失处理（BLOCKED）；SidecarExit 是强杀用例的**强匹配**断言（不得借道其它变体/不得误报成功）。RouteError↔`sr_core::error::Error`（E_*，已存在）的映射属实现内部。
- **时间/单位**：pacer 的 QPC 频率显式注入（换算必须可确定性单测，REQ-25 的 M3 钩子）；format_timestamp 显式偏移。
- **收尾语义**：request_stop 立即返回；wait_finalized 等后台 concat（成品验证成功后清理原段属保留策略，M3 不预设）；EOF 是唯一优雅停止路径。
- **段清单**：segments() 反映清单视角——强杀后完成段永不消失；0 字节壳与未封口烂段不进入收尾（S1b/S5 冻结；烂段=E_SEGMENT_CORRUPT 从 concat 剔除）。
- **stderr 约定**：argv 含 -nostdin（基准要素）；stderr 持续排空为运行时行为（S1b §7），由 L2 停止/强杀路径间接覆盖。

## 4. 运行命令与门控

```text
# L1（随默认 cargo test 执行）
cargo test -p sr-capture --test pacer_cfr -- --nocapture
cargo test -p sr-pipeline --test ffmpeg_args_snapshot -- --nocapture
cargo test -p sr-pipeline --test output_naming -- --nocapture

# L2（显式执行；须交互式用户会话 + WGC + 可用编码器）
cargo test -p sr-itests --test route -- --ignored --nocapture
# 单用例示例
cargo test -p sr-itests --test route it_single_route_frames_nonblank -- --ignored --nocapture

# 全仓（红阶段 exit 101 属预期）
cargo test --workspace
```

BLOCKED 触发点：非交互会话；锁定 ffmpeg n8.1 构建缺失；start_route 报 EncoderUnavailable（§9 T1 环境无编码器）/ConsentDenied（WGC 被拒）。

## 5. 依赖与锁定构建

- `tests/sr-itests/Cargo.toml` 追加：`sr-pipeline`（path）；windows features 追加 `Win32_System_Diagnostics_ToolHelp`（Toolhelp32 父子枚举，仅 helper 使用）。**无新增外部 crate**。
- L1 测试位于各产品 crate 自身 `tests/`（自动链接本 crate，Cargo.toml 零改动）。
- **ffmpeg 使用**：spec §8 钉的 release 构建 `tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin`（探测命中；L2 全部 ffmpeg/ffprobe 调用经 `CARGO_MANIFEST_DIR` 相对定位后 canonicalize 为绝对路径）。实测版本 `n8.1.3-18-g46d899d7a0-20261015`；SHA256（外部 Get-FileHash 记录）：
  - `ffmpeg.exe` = `3B57117B384DF98F2B8D01876DF0A596A0B6EBF45F756B30F15D6F65AB8251C5`
  - `ffprobe.exe` = `5B9724EE7A115D5A8458D41BD2E0CE6206847E2B76105E822061A4421322CDE9`
  - （M1/M2 时代 L2 用过的 master 构建 `ffmpeg-master-latest-win64-lgpl` 仍在仓库，本里程碑起测试侧统一 n8.1，与 §8 冻结一致。）

## 6. 红灯证据（实际执行，2026-10-16）

### 6.1 `cargo test --workspace` → exit 101（全部为预期未解析符号；cargo 首批失败后未再调度 sr-itests 目标，故另跑定向命令补全）

```text
error[E0432]: unresolved imports `sr_capture::pacer::FrameAction`, `sr_capture::pacer::FramePacer`
  --> crates\sr-capture\tests\pacer_cfr.rs
error[E0432]: unresolved imports `sr_pipeline::sidecar::build_encode_args`, `sr_pipeline::sidecar::AudioPipeSpec`, `sr_pipeline::sidecar::VideoPipeSpec`
  --> crates\sr-pipeline\tests\ffmpeg_args_snapshot.rs
error[E0432]: unresolved imports `sr_pipeline::naming::format_timestamp`, `sr_pipeline::naming::next_available`, `sr_pipeline::naming::sanitize_target_name`
  --> crates\sr-pipeline\tests\output_naming.rs
error: could not compile `sr-capture` (test "pacer_cfr") due to 1 previous error
error: could not compile `sr-pipeline` (test "ffmpeg_args_snapshot") due to 1 previous error
error: could not compile `sr-pipeline` (test "output_naming") due to 1 previous error
```

### 6.2 定向命令（sr-itests；exit 101，同类 E0432）

```text
cargo test -p sr-itests --test route --no-run
error[E0432]: unresolved import `sr_pipeline::naming::sanitize_target_name`   (route.rs:48)
error[E0432]: unresolved import `sr_pipeline::route`                           (route.rs:49；含
              start_route/RouteConfig/RouteError/RouteHandle/RouteTarget/VideoCfg 六名)
error: could not compile `sr-itests` (test "route") due to 2 previous errors
```

**红因确认**：5 个 error 全部为「新测试引用的预期公共符号 unresolved（实现缺失）」，无任何其它类别错误。

### 6.3 既有绿不退化

| 命令 | 结果 |
|---|---|
| `cargo test -p sr-core --test channel_snapshot_rebind` | `1 passed; 0 failed`（L1） |
| `cargo test -p sr-itests --test enum_targets / enum_audio / shot --no-run` | 均 Finished、exit 0（shot 为 M2 绿阶段实现后的正确可编译状态） |

### 6.4 Win32/Helper 运行时自检（仓库外 scratch `%TEMP%\opencode\sr-m2-red-scratch\m3-scratch`，exit=0）

```text
[env] Microsoft Windows [Version 10.0.26200.9457]；SESSIONNAME=Ok("Console")
[motion] 784x461，两次采样平均像素差 = 23.08     ← 运动窗口（专用泵线程）内容确实随时间变化
[gen] 12s@testsrc2 → 3 段（5s 段长）
[probe] duration = 5；stream = "h264,640,360,30/1"；keyframes = [2.0, 4.0]   ← -g 60 恰好 2s 间隔
[decode] 3fps 采样 15 帧；null 解码 exit=0 stderr=""
[kill] Toolhelp32 发现 ffmpeg 子进程；taskkill /T /F → ExitStatus(1)（与 S1b 实测的强杀退出码一致）
ALL M3 HELPER CHECKS DONE
```

（scratch 过程记录：① 无 `-re` 的 lavfi dummy 会被全速拉空 <1.5s 早退致 Toolhelp32 扑空——真实 sidecar 被管道阻塞写按 30fps 定速必长存，dummy 加 `-re` 修正；② `taskkill /F` 后 `try_wait` 需轮询 ≤5s 等句柄回收。）

### 6.5 stub-编译自检（M1 §9.4 方法论；契约 stub 位于 scratch `tc-capture`/`tc-pipeline`，不进仓库）

对三个 L1 文件与 route.rs 原文件做「import 可解析之后」全量 typecheck/borrowcheck：`cargo test --offline --no-run` → 三个测试可执行全部产出、exit=0、0 警告。**该自检抓到并已修复 3 处测试自身缺陷**（先 scratch/stub 验证再修仓库，记录如下）：

| # | 缺陷 | 修法 |
|---|---|---|
| 1 | pacer_cfr.rs 三处断言消息含 `Write{2}`/`Write{1}`/`Write{5}`——被 Rust 格式串解析为不存在的位置参数（compile error） | 措辞改为「应 Write 到槽 N」 |
| 2 | route.rs `PostMessageW(..., None, None)`——WPARAM/LPARAM 非 Option（scratch 编译证据） | `WPARAM(0), LPARAM(0)` |
| 3 | route.rs `extension().map_or("", e.to_string_lossy())`——Cow 与 &str 类型不匹配 + 3 处未用导入 | 改 `map_or(false, |e| e.to_string_lossy() == "mkv")`；清理导入 |

## 7. 关键设计取舍

| # | 取舍 | 依据 |
|---|---|---|
| R1 | 运动窗口在**专用线程**跑消息泵（33ms WM_TIMER + InvalidateRect；移动亮条 37px/帧 + 秒级翻转象限块） | 主测试线程 sleep 驱动录制时无人泵消息→WM_TIMER 不触发→静帧；专用线程使「录制中」与「内容运动」解耦。scratch 以 PrintWindow 双采样验证（差 23.08） |
| R2 | 内容级判定=ffmpeg 采样解码（3fps/64x36 rgb24）亮度 std + 帧间差；阈值 2/2/10 | 构造性保证（图案 std≫2、差≫2）+ scratch 校准；黑帧/静帧两侧均差一个数量级；零新增 Rust 依赖 |
| R3 | 关键帧断言用 ffprobe `-skip_frame nokey`（pts 差 ≤2.5s；scratch 实测恰 2.0s） | 直接验证 -g 60 与 force_key_frames 的运行时效果，比 argv 断言强一层 |
| R4 | 强杀用例=Toolhelp32 父子枚举 + taskkill /T /F **sidecar 树**（非主进程自杀） | 测试进程内无法杀自身后继续断言；sidecar 树即 §3.5「sidecar 被终止」路径（S1b 实验 2 同手段、退出码一致）；主进程矩阵归 REQ-22 |
| R5 | 强杀前保证「前两段确定闭合」（≥2 段出现后再录 6s） | S1b：未封口段必烂（无时长）；只对确定闭合的快照断言可播，语义不削弱（0 字节壳排除另行断言） |
| R6 | `start_route` Ok=「输出开始」（启动协议完成） | 使「成品时长≈录制段」可用墙钟交叉验证；编码器/授权失败走 BLOCKED 分流（环境）而非实现缺陷 |
| R7 | 命名断言把 route 与 naming 模块契约挂钩（成品名含 sanitize(标题)） | REQ-24 命名是管线行为而非孤立函数；窗口标题即 Window 目标名（测试假定） |
| R8 | ffmpeg 参数断言「存在性+语义相邻」不锁 argv 全序 | §3.3 是骨架（S2/S5/S6 校准）；锁全序会把实现钉死在未冻结细节上 |

## 8. 未决问题（返回 orchestrator）

1. **测试名与 §4 的偏差**（任务给定名优先，已按任务执行）：`unit_pacer_cfr_dup_drop`（§4：`unit_pacer_source_timestamp_slots`）、`it_cursor_toggle_degrades`（§4：`it_cursor_flag_and_border_degrade`）、`it_segments_finalize_concat`（§4 REQ-21 行：`it_segments_finalize_concat_alltracks`）、`it_crash_kill_segments_playable`（§4 REQ-22 行：`it_crash_kill_matrix_recovery_state_machine`）、`unit_output_naming`（§4 REQ-24 行：`unit_naming_sanitize_no_overwrite`）。§4 全音轨版（alltracks，含音频）与完整杀矩阵/磁盘满注入属 M4+/M8 用例，本红未覆盖也不应覆盖。
2. **任务 REQ 编号勘误**：任务把 `unit_ffmpeg_args_snapshot`/`it_route_pipeline_mkv_valid` 标在 REQ-19（纯录音 WAV）下；按 spec §4 前者属 REQ-20 行。`it_route_pipeline_mkv_valid` 为任务新增端到端用例（§4 无对应名，最近邻 `it_pipe_startup_no_deadlock`）。§4 表建议补登或归并。
3. **REQ-11「能力不可用时降级不阻塞」未测**：需故障注入钩子（REQ-28 `it_probe_permission_matrix_injected`）；本红只钉开关语义（两配置均产出有效非黑成品）。去边框（borderless）能力同属该轮。
4. **`route` 模块归属为测试假定**（sr_pipeline::route）：§3.2/§3.4 意图 RouteConfig/TargetRef 落 sr-core::types（M1-A 骨架 types 占位）。实现若迁至 sr-core 需同步改 route.rs 两处 import（可协商，非阻塞；同 M1 的 ChannelHub 归属未决模式）。
5. **RouteTarget::Region 未被任何测试构造**：M3 用例只用 Window（运动窗口）；Region 路由（跨屏录制画布）属后续里程碑，实现按 §3.4 补全变体即可。
6. **`RouteError::WaitTimeout` 为契约记载、无测试引用**：绿阶段如改用别的超时表达（如 Option 返回），可删改该变体（报告同步）；其余三变体被测试强引用。
7. **sidecar 进程发现以映像名含 "ffmpeg" 为约定**：强杀用例的 Toolhelp32 匹配依赖 sidecar=锁定 ffmpeg.exe（§8 冻结）。若实现改名/经 shim 启动，需改为 RouteHandle 暴露 sidecar pid（契约扩张，另行协商）。
8. **时间预算未断言**：启动时延、stop 响应 ≤2s（T5）、收尾时长等未设断言（spec 有目标但属性能预算，S 系实测语境）；如需可加宽松上限（如收尾 ≤30s 已隐含在 wait_finalized 120s 超时中）。
9. **阈值标定环境**：内容阈值（std≥2/差≥2/10）与关键帧容差（2.5s）基于本机 scratch 与合成段实测；绿阶段若在极端主题/编码器下逼近阈值，凭实测证据调整并记录（不放宽断言语义）。
