# M3-C 绿实现报告（TDD 绿阶段收尾：单路视频管线 REQ-9..11/20/21/24 的 M3 范围）

- 日期: 2026-10-16
- 规格: specs/001-screen-recorder.md §2.3（REQ-9..11/13）、§2.5（REQ-20/21/24）、§3.1/§3.3（启动协议/管道协议/ffmpeg 基准）、§3.4/§3.5、§4、§5 M3 行、§8 M0 冻结项
- 红阶段基线: docs/milestones/m3-red-report.md（§3 契约、§5 锁定构建、§6 红证据）
- Spike 依据: docs/spikes/s1/s1-pipeline-throughput.md §2/§4–§6（CPU 缩放路径/阶段成本/管道定稿）、docs/spikes/s1b/s1b-pipe-faults.md §2–§7（背压/取消/EOF/错误映射/8MiB）
- 环境: Windows 11 25H2，build **10.0.26200.9457**，SESSIONNAME=Console（交互式用户会话）；cargo/rustc **1.99.0 msvc**；sidecar=锁定 `tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin`（红报告 §5，SHA256 已留痕）。
- 会话说明: 本会话为 M3-C 绿实现（同一执行者此前的 M2-C 绿阶段已完成并留报）。

## 1. 范围与约束遵守

- **未修改任何测试文件**（pacer_cfr.rs / ffmpeg_args_snapshot.rs / output_naming.rs / route.rs / common 逐字未动——按 mtime 扫描核实，tests/ 全树仅存在红作者 3:36–3:39 的落盘时间戳）；未改 specs/、docs/spikes/；未 git 提交。
- 实现面 = `crates/sr-capture`（Cargo.toml + src×5）、`crates/sr-pipeline`（Cargo.toml + src×4）、workspace `Cargo.toml`（dev profile 热点优化，见 §4.1）、`Cargo.lock`（随依赖更新）。无其它文件触碰。
- **发现测试自身缺陷 1 处（3 个断言期望值错误），未私改，已按规约停在该用例并列入 §5**（仅测试作者可修）。

## 2. 变更文件盘点

| 文件 | 内容 |
|---|---|
| `crates/sr-capture/Cargo.toml` | 依赖：`windows-capture 2.0.1`（spec §1 锁定视频栈）、`fast_image_resize 5.5`（S1 定稿 CPU 缩放）、`windows 0.62`（管道 API：Foundation/Security/Storage_FileSystem/System_IO/System_Pipes） |
| `crates/sr-capture/src/pacer.rs` | 帧槽节奏器纯逻辑（REQ-10）：槽位=源时间戳纯函数、首帧 T0 门控、每槽至多一帧、乱序未写槽必 Write（HashSet 记账）、跨槽大跳不误丢 |
| `crates/sr-capture/src/canvas.rs` | 固定画布（D6）：fast_image_resize Bilinear 等比缩放 + studio-black(RGB16) 黑边 compose + BGRA→NV12（BT.601 limited，2×2 均值色度；S1 同款实现）；画布/缩放缓冲跨帧复用 |
| `crates/sr-capture/src/pipe.rs` | 命名管道视频写（§3.3/S1b 冻结）：`CreateNamedPipeW` 字节模式单实例 8MiB（`FILE_FLAG_FIRST_PIPE_INSTANCE` 防静默接管）；`ConnectNamedPipe` + watcher 线程外置超时 `CancelIoEx`（995=超时、535=抢连成功）；阻塞式整帧 `WriteFile`（背压）；错误分类 232/109→PeerGone、995→Cancelled；EOF=关闭句柄 |
| `crates/sr-capture/src/wgc.rs` | 单路 WGC 捕获（windows-capture 2.0.1，`start_free_threaded` 独立线程+frame pool）：回调内「源时间戳→帧槽决策→固定画布 NV12→更新最近帧槽」；光标开关直通 `CursorCaptureSettings`；无效目标→TargetInvalid、E_ACCESSDENIED→ConsentDenied |
| `crates/sr-capture/src/lib.rs` | 模块装配（canvas/pacer/pipe/wgc） |
| `crates/sr-pipeline/Cargo.toml` | 依赖：`sr-capture`（path）、`windows 0.62`（Foundation/System_SystemServices/System_Time/UI_WindowsAndMessaging） |
| `crates/sr-pipeline/src/naming.rs` | REQ-24 三函数：sanitize（非法字符/控制字符替换、结尾点空格修剪、保留名转义）、format_timestamp（`yyyyMMdd_HHmmss`，显式偏移，Hinnant civil 算法）、next_available（纯分配器 `_1/_2` 序号禁覆盖） |
| `crates/sr-pipeline/src/sidecar.rs` | `build_encode_args`（§3.3 基准骨架逐要素，输出 pattern 收尾）、锁定 n8.1 ffmpeg 定位、D4 编码器链**实际初始化**探测（nvenc→qsv→amf→h264_mf，进程内缓存）、sidecar spawn + stderr 持续排空线程（尾部留存诊断） |
| `crates/sr-pipeline/src/route.rs` | 单路路由编排（见 §3）；`RouteTarget`（Region=最小占位变体，返回 TargetInvalid）、`VideoCfg`、`RouteConfig`、`RouteError`（钉 4 变体+自定义）、`Finalized`、`RouteHandle`、`SlotSender` |
| `Cargo.toml`（workspace） | `[profile.dev.package]` 热点优化（§4.1；语义不变） |
| `Cargo.lock` | 新增 `windows-capture 2.0.1`、`fast_image_resize 5.5.0` 及传递闭包（rayon/crossbeam/either/parking_lot/thiserror/document-features/litrs） |

## 3. 路由编排（route.rs 设计要点）

- **启动协议（§3.1，消除互等）**：D4 编码器探测 → 建管（server 先监听）→ spawn sidecar（stderr 管道+排空线程）→ 外置超时连接（5s）→ WGC 会话启动 ⇒ **Ok=「输出开始」**；任一步失败即回滚（kill child / close pipe）并返回对应错误变体。
- **CFR 槽发送器（SlotSender）**：独立线程按 1/fps 节拍逐槽写管道——取「最近帧槽」新鲜帧，源欠产时重复上一帧（§3.3「连续 NV12 帧…CFR 由帧槽节奏器保证」的最小实现；首帧前启动间隙跳槽；写阻塞即背压，不反压 DWM 回调）。纯节奏器契约不受影响（L1 `unit_pacer_cfr_dup_drop` 全绿）。
- **停止协议**：`request_stop` 置位立即返回；worker（50ms 轮询）停捕获 → 停发送器 → **EOF**（唯一优雅停止）→ 等 sidecar 5s（超时进程内强杀）→ 后台收尾。
- **错误归因（§3.5）**：sidecar 非零退出/被终止（无论是否请求停止）→ `SidecarExit`（保留分段）；捕获自然结束（目标关闭）→ 按目标丢失语义优雅收尾；EOF 后非零退出同样归 `SidecarExit`。
- **收尾（REQ-21）**：扫描本路由段（`{prefix}%05d.mkv`，**0 字节壳排除**，字典序=数值序）→ 写收尾 ffconcat（相对条目）→ `ffmpeg -f concat -safe 0 -i … -map 0 -c copy -f matroska {成品}`（60s 上限）→ 成品存在且非 0 字节 ⇒ `Finalized`。
- **segments()**：目录扫描的清单视角（0 字节壳不列入；Err 后仍可查；完成段永不消失）。
- **命名（REQ-24）**：成品 = `{format_timestamp(now, 本地偏移)}_{sanitize(窗口标题/显示器名)}.mkv`，`next_available` 纯分配防覆盖；段 pattern/清单/管道名带进程内路由序号防碰撞。

## 4. 复跑证据（本机实际执行，2026-10-16）

| # | 命令 | 结果 |
|---|---|---|
| 1 | `cargo clean -p sr-capture -p sr-pipeline` + `cargo check -p sr-capture -p sr-pipeline` | **exit 0，0 warning 0 error** |
| 2 | `cargo test -p sr-capture --test pacer_cfr -- --nocapture` | **`1 passed; 0 failed`** |
| 3 | `cargo test -p sr-pipeline --test ffmpeg_args_snapshot -- --nocapture` | **`1 passed; 0 failed`** |
| 4 | `cargo test -p sr-pipeline --test output_naming -- --nocapture` | **FAILED（测试缺陷，见 §5）**：panic 于 `output_naming.rs:74`（实现返回 `"19700102_000000"`，数学正确） |
| 5 | `cargo test -p sr-itests --test route -- --ignored --nocapture` | **`5 passed; 0 failed`（19.55s）**，逐用例：`it_single_route_frames_nonblank` ok（时长 7.400s vs 录制段 7.000s；22 采样帧运动对 21/21、最大差 61.85）；`it_route_pipeline_mkv_valid` ok（640x360/30fps/h264、1 视频轨 0 音频轨、关键帧 [0.0, 2.0, 4.0, 6.1] 最大间隔 2.1s ≤2.5、命名含 `{ts}_{sanitize(标题)}`）；`it_segments_finalize_concat` ok（录制中 seg0 独立可播 5.1s；收尾后成品 12.733s = Σ段 12.733s（±0）≈ 录制段 12.599s；整文件解码干净）；`it_crash_kill_segments_playable` ok（taskkill /T /F 后 `wait_finalized` → **Err(SidecarExit)**；闭合快照 2 段仍在清单且独立可播；3 个清单条目全部非 0 字节）；`it_cursor_toggle_degrades` ok（cursor=true/false 两路均 h264、时长 ≥3s、非黑且运动、成品路径互异） |
| 6 | `cargo test --workspace` | **exit 101，唯一失败=§5 测试缺陷**；L1 全绿：`screen_rec_lib` 7 passed、`channel_snapshot_rebind` 1 passed、`pacer_cfr` 1 passed、`ffmpeg_args_snapshot` 1 passed；L2 按门 ignored：enum_audio 1 / enum_targets 4 / **route 5** / shot 4（全部可编译） |
| 7 | M1/M2 L2 回归复跑：`cargo test -p sr-itests --test enum_targets --test enum_audio -- --ignored` | **exit 0：4+1 全 passed**（无回归） |
| 8 | M2 L2 回归复跑：`cargo test -p sr-itests --test shot -- --ignored` | **3 passed + `it_screenshot_mixed_dpi_negative_coords` BLOCKED fail**（本机无负方向显示器，M2 绿报告 §4.2 既定机制；无回归） |
| 9 | scratch 复算（仓库外 `%TEMP%\opencode\m3-naming-scratch`，path 依赖 sr-pipeline） | **1 passed**：output_naming.rs 全部断言按数学正确期望复算通过（含 74/76/77 的修正值），证明缺陷仅限测试文件 3 处期望值 |

日志：`%TEMP%\opencode\m3_l1.log`、`m3_route_t1f.log`、`m3_route_t2.log`、`m3_route_t3.log`、`m3_route_t4.log`、`m3_route_t5.log`、`m3_route_all.log`、`m3_ws_test.log`、`m3_enum_regress.log`、`m3_shot_regress.log`、`m3_check.log`。

### 4.1 dev profile 热点优化（必要修正，非性能装饰）

首轮 L2 实测成品时长仅 1.5s/7.0s：探针定位为 **dev profile opt-level=0 下逐帧数值路径（BGRA→NV12 + 缩放）≈150ms/帧 → 实际交付 6.4fps**（S1 的 ~6.6ms/帧为 release 实测）。L2 用例断言实时性（时长≈录制段、段闭合节拍），故对两个热点数值包启用编译优化（语义不变）：`[profile.dev.package.sr-capture] opt-level=2`、`[profile.dev.package.fast_image_resize] opt-level=3`。修正后 CFR 连续交付 ≈30fps，`it_single_route_frames_nonblank` 5.13s→7.40s（±2s 容差内）。

### 4.2 CFR 槽发送器（范围判定说明）

实现「按 1/fps 节拍逐槽发送最近帧（新鲜或重复）」的 `SlotSender`：依据 = §3.3「视频：**连续** NV12 定长帧…CFR 由帧槽节奏器保证」。没有它时交付率跟随源内容更新率（本机测试窗口自绘 ≈22fps ⇒ 时长≈0.73×录制段），`it_segments_finalize_concat` 的 ±2s 断言必失败、强杀用例的「前两段已闭合」时点存在竞态。该实现不改变纯节奏器契约（槽决策仍为纯函数、L1 全绿），亦不等同 REQ-25/26 的 epoch 映射/漂移补帧机制（M4+），仅为 §3.3 连续流语义的最小落实。

## 5. 测试缺陷报告（按规约未私改，待测试作者修正）

`crates/sr-pipeline/tests/output_naming.rs` 的 `format_timestamp` 断言存在 **3 处期望值错误**，且互相矛盾、无任何一致实现可同时满足：

| 行 | 测试期望 | 数学正确值 | 依据 |
|---|---|---|---|
| 74 | `format_timestamp(86_400_000, 0) == "19710101_000000"` | **`"19700102_000000"`** | 86,400,000ms = +1 **天**（1970-01-01T00:00:00Z + 1 天 = 1970-01-02）；"1971-01-01" 需 +365 天 |
| 76 | `format_timestamp(T2025, 330) == "20250101_0530"`（13 字符） | **`"20250101_053000"`** | 05:30:00 按 `HHmmss` = "053000"；与第 72 行 `"19700101_000000"`（15 字符 HHmmss 格式）直接矛盾 |
| 77 | `format_timestamp(T2025, -60) == "20241231_2300"`（13 字符） | **`"20241231_230000"`** | 23:00:00 同上；亦与第 72/75 行及第 80 行 `s.len()==15` 形状断言矛盾 |

- 契约面（红报告 §3.3「`yyyyMMdd_HHmmss`（15 字符）」+ 测试头部注释 + 第 72/73/75/80 行）一致钉 15 字符格式，故第 74/76/77 行为测试文件笔误。
- 矛盾不可调和证明：76 行要求 05:30:00→`"0530"`（分钟非零仍裁秒）而 72 行要求 00:00:00→`"000000"`；若按「分钟为零才保留 HHmm」解释，77 行 23:00:00→`"2300"` 与 73 行 08:00:00→`"080000"` 又矛盾。
- **本会话处置**：实现按契约（15 字符）交付；`cargo test --workspace` 中该用例如实 FAILED（停在 74 行）；仓库外 scratch 以修正期望复算 **全部断言通过**（含 sanitize/next_available/形状断言），证明缺陷边界恰为上述 3 行。**需测试作者改 3 个字符串字面量即可转绿**：`"19700102_000000"`、`"20250101_053000"`、`"20241231_230000"`。

## 6. 契约偏差与实现说明（如实记录）

1. **pacer 槽位域**：红报告 §3.1 文字为 `floor((t-T0)·fps·qpc⁻¹)`，但绑定测试（pacer_cfr.rs 第 3 组）要求首帧（t=2 槽）Write 到**槽 2**，即槽位为**源时间域绝对槽** `floor(t·fps·qpc⁻¹)` + 首帧 T0 门控（t<T0 → Drop）。实现按测试（测试即契约），与报告文字的偏差在此留痕。
2. **Monitor 索引映射**：`RouteTarget::Monitor{index}` 沿 sr_enum 0 起稠密索引，windows-capture `Monitor::from_index` 为 1 起 ⇒ 实现 `from_index(index+1)`。M3 route 用例仅构造 Window 目标，该映射未经 L2 实测（红报告 §8-5 同类未决）。
3. **`RouteTarget::Region`**：按任务「最小实现（可编译+合理错误）」落为单位变体，`start_route` 返回 `RouteError::TargetInvalid`（M3 未实现，注释明示）；M4+ 按 §3.4 补全。
4. **依赖新增**：`windows-capture 2.0.1`（spec §1 锁定栈、任务指名）、`fast_image_resize 5.5`（S1 定稿 CPU 路径）——均为任务/规格明确授权。
5. **ffmpeg 定位**：`CARGO_MANIFEST_DIR` 相对仓库内锁定构建（与红报告 §5 测试侧同源）；打包形态的路径覆盖属后续 Settings 工作。
6. **错误变体**：钉死的 4 变体中 `SidecarExit` 带诊断字段（花括号模式兼容）；`EncoderUnavailable`/`ConsentDenied`/`WaitTimeout` 为单元变体（同样匹配 `{ .. }` 模式）；另有 TargetInvalid/CaptureFailed/StartFailed/FinalizeFailed/Internal 实现自定变体。
7. **音频**：M3 路由为视频单路（`build_encode_args` 空音频收缩路径已被 L1 用例 B 覆盖）；音频管道/全轨收尾属 M4+。

## 7. 未决问题

1. **`unit_output_naming` 转绿依赖测试作者修复 §5 的 3 个字面量**（`cargo test --workspace` 当前 exit 101 的唯一原因）。
2. **目标关闭语义**：捕获自然结束现按「优雅 EOF → 正常收尾 → Ok(Finalized)」处理（保留分段）；E_TARGET_LOST 的显式错误信号与 UI 呈现属 REQ-13/M4。
3. **写停滞 watcher**（S1b §7.3：对端「活但不读」3–5s → CancelIoEx → E_PIPE_STALL）未实现；对端死亡路径由内核即时解除（232/109，强杀用例实测通过）。M3 测试未覆盖停滞注入。
4. **Job 对象绑定 sidecar**（REQ-22/D8）未做：主进程死亡时 sidecar 残留——主进程杀矩阵属 REQ-22/M8 用例范围。
5. **逐帧成本**：windows-capture 库内每帧新建 staging 纹理；如需进一步降低 CPU（当前 dev 优化后实时余量充足）属 M4+ 优化项。
6. **REQ-28 光标/边框降级故障注入**、**Region 路由**、**全音轨 concat（alltracks）**、**音频管道小缓冲**：红报告 §8 既定后续里程碑范围。

## 8. 结论

M3 绿阶段达成：`pacer_cfr` / `ffmpeg_args_snapshot` L1 全绿；`route` L2 **5/5 全绿**（非黑帧/命名与关键帧/分段收尾/强杀 SidecarExit/光标开关）；M1/M2 全部既有测试无回归（enum 4+1、shot 3+既定 BLOCKED、channel 1、settings 7）；`cargo test --workspace` 唯一红项为 **output_naming 测试文件自身的 3 处期望值笔误**（§5，已留证据与修法，未私改）。证据全部落盘本报告。

## 9. 缺陷闭环（§5 笔误由测试作者修正，2026-10-16）

- 修正（仅 3 个字符串字面量，其余断言/测试结构/生产代码零改动）：`crates/sr-pipeline/tests/output_naming.rs:74` `"19710101_000000"`→`"19700102_000000"`（86,400,000ms=+1 天非 +1 年）；`:76` `"20250101_0530"`→`"20250101_053000"`；`:77` `"20241231_2300"`→`"20241231_230000"`（后两处补齐 HHmmss 秒位=15 字符）。
- 确认为真修复而非迎合实现：落改前以 .NET DateTime（与被测实现无关）独立复算三值，与 §5「数学正确值」列逐字一致。
- 验证（实际执行）：`cargo test -p sr-pipeline --test output_naming` → `1 passed; 0 failed`；`cargo test --workspace` → **exit 0**（L1 全绿：screen_rec_lib 7 / channel 1 / pacer 1 / args 1 / naming 1；L2 按门 ignored 共 14：enum_audio 1 / enum_targets 4 / route 5 / shot 4，0 failed）。
- 位置说明：闭环注记落本文件（缺陷报告与跟踪在 §5/§7-1/§8，就近闭环）；m3-red-report 为红阶段基线记录，未改动。
