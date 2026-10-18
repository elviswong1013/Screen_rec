# M6-C 绿实现报告（TDD 绿阶段：全音轨/纯录音 REQ-15/16/17/19）

- 日期: 2026-10-17（全部命令为本机实际执行；日志归档见 §8）
- 角色: **executor-backup 承担实现**（GLM 系 test-writer/executor 额度耗尽至 2026-10-22，编排者指派后备通道；同 M5 先例）
- 规格: specs/001-screen-recorder.md §2.4（REQ-15/16/17/19）、§3.1/§3.2/§3.3/§3.4/§3.5、§4（REQ-15/16/17/19 行）、§5 M6 行、§8 M0 冻结项（S3）
- 红阶段基线: docs/milestones/m6-red-report.md（**§3 契约=绑定**；§6.7 设备状况；§8 未决）
- 先例: docs/milestones/m4-green-report.md、m5-green-report.md（既有 sr_audio/sr_pipeline API 现状与验收纪律）
- 环境: Windows 11 25H2，`Microsoft Windows [Version 10.0.26200.9711]`；cargo/rustc **1.99.0** msvc；sidecar/ffplay/ffprobe = 锁定 `tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin`（§8 冻结）；L2 以 `$env:SESSIONNAME='Console'` 运行（红 §8-9 注记）。
- 会话说明: 仓库无 git 提交历史（全部 untracked，与 M1–M5 一致）；未 git 提交。

## 1. 范围与约束遵守

- **未修改任何测试文件**（编排者已记录哈希）：`audio_m6.rs` SHA256 `7D62C9AC…4D1B10`（会话前后一致）、`audio_isolation.rs` `B33A566F…769627`、`multi_route.rs` `BFD099C9…8CFEDE`、`route.rs` `D21AB7A4…E031D`、`common/mod.rs` `1258527C…77CBA8`——全部逐字未动。
- **未改** `specs/`、`docs/spikes/`、`tests/sr-itests/Cargo.toml`、`Cargo.lock`（**零新依赖**：wasapi 0.26 / windows 0.62 / rustfft 均为既有锁定依赖）。
- 生产改动仅 `crates/sr-audio`（wasapi/wav/slots/lib）+ `crates/sr-pipeline`（route/lib），逐文件见 §2。
- 无调试/临时内容残留：会话中的 TEMP DIAG 诊断代码（§7）已全部移除（`grep 'diag' crates/sr-audio/src/wasapi.rs` 零命中）。

## 2. 变更文件清单

| 文件 | 变更 |
|---|---|
| `crates/sr-audio/src/wasapi.rs` | M6 采集面：`AudioDeviceInfo`/`CaptureTarget`（Process/DeviceLoopback/Microphone 同一采集核）、`CaptureError::MicUnavailable`、`capture_device_loopback`（默认渲染端点环回，REQ-15）、`capture_microphone`（REQ-16）、`list_capture_devices`（专用 MTA 线程枚举，REQ-16）、`LoopbackStream::start_target`（流式三目标）；`run_capture_core` 按目标激活/错误分流（进程环回语义逐字保持——回归佐证） |
| `crates/sr-audio/src/wav.rs`（新增） | REQ-19：`WavWriter`（44B 头部占位 + s16 流式追加 + `finalize` 封口）、`repair_wav`（头部重写 + 尾部不完整样本裁切 + 幂等）、`WavRepair`/`WavError`；RIFF 4GiB 上限明确报错 |
| `crates/sr-audio/src/slots.rs` | **缺陷修复（阻绿，见 §7）**：`SampleSlotPacer::pad_to_ts` 改为单调（水位只前进不回退） |
| `crates/sr-audio/src/lib.rs` | `wav` 模块由 TODO 骨架落地为真实模块；模块头注释同步 M6 |
| `crates/sr-pipeline/src/route.rs` | M6：`start_audio_only`/`AudioOnlyConfig`/`AudioOnlyHandle`/`AudioOnlyFinalized`（REQ-19，停止即成品）；`start_route_impl` 放开 `System`/`Mic` 源为各一条独立轨（REQ-15/16/17 全音轨）；`start_audio_stream`/`capture_start_error` 辅助 |
| `crates/sr-pipeline/src/lib.rs` | `route` 模块文档注释补 M6 一行 |
| `docs/milestones/m6-green-report.md` + `m6-green-*.log` ×11 | 本报告 + 原始日志归档（§8） |

M3/M4/M5 既有 API（`start_route`/`start_audio_route`/`start_routes`/`RouteConfig`/`AudioRouteConfig`/`MultiRouteConfig`/`capture_process_loopback`/`LoopbackStream::start` 等）字面量零改动——回归三套全绿佐证。

## 3. 绿前红态复现（修改前，exit 101）

命令：`cargo test --workspace` → **exit 101**，恰 5 个 error、与红报告 §6.2/§6.8 逐条一致、无其它类别错误（日志 `m6-green-red-repro.log`）：

```text
error[E0432]: unresolved imports `sr_audio::wasapi::capture_device_loopback`, `capture_microphone`,
             `list_capture_devices`, `AudioDeviceInfo`                     （audio_m6.rs:54）
error[E0432]: unresolved import `sr_audio::wav::repair_wav`               （audio_m6.rs:57）
error[E0432]: unresolved imports `sr_pipeline::route::start_audio_only`, `AudioOnlyConfig`,
             `AudioOnlyHandle`                                             （audio_m6.rs:59）
error[E0599]: no variant named `MicUnavailable` found for enum `CaptureError`（×2）
error: could not compile `sr-itests` (test "audio_m6") due to 5 previous errors
```

## 4. 复跑证据（冻结树；命令 + exit + 计数 + 耗时；全部本机实测）

| # | 命令 | 结果（exit / 通过数 / 耗时） | 日志（docs/milestones/） |
|---|---|---|---|
| 1 | `cargo test --workspace` | **exit 0**；L1 **13 passed / 0 failed**（screen_rec_lib 7、drift_hysteresis 1、silence_pad_late_packet 1、pacer_cfr 1、channel_snapshot_rebind 1、ffmpeg_args_snapshot 1、output_naming 1）；L2 门 ignored：audio_isolation 4 / audio_m6 4 / enum_audio 1 / enum_targets 4 / multi_route 4 / route 5 / shot 4；**0 warning**（构建 4.67s） | `m6-green-final-ws.log` |
| 2 | `cargo test -p sr-itests --test audio_m6 -- --ignored --nocapture --test-threads=1` | **exit 101**：**3 passed / 1 BLOCKED**（唯一失败=mic 环境缺失，§5；BLOCKED=fail 属设计门）35.59s | `m6-green-final-audio-m6.log` |
| 3 | `… audio_m6 it_audio_device_loopback_rms …` | **exit 0；1 passed**（9.16s） | `m6-green-final-case-devlb.log` |
| 4 | `… audio_m6 it_audio_only_wav_duration_and_repair …` | **exit 0；1 passed**（12.72s） | `m6-green-final-case-wav.log` |
| 5 | `… audio_m6 it_audio_multitrack_ffprobe_fft_all_tracks …` | **exit 0；1 passed**（13.71s） | `m6-green-final-case-alltracks.log` |
| 6 | 回归 `… --test audio_isolation -- --ignored --nocapture --test-threads=1` | **exit 0；4 passed**（47.09s） | `m6-green-final-reg-audio-isolation.log` |
| 7 | 回归 `… --test multi_route …` | **exit 0；4 passed**（37.86s） | `m6-green-final-reg-multi-route.log` |
| 8 | 回归 `… --test route …` | **exit 0；5 passed**（60.50s） | `m6-green-final-reg-route.log` |

（#6–#8 各自串行执行；运行结束后 `Get-Process ffmpeg,ffplay` 抽查 0 残留。）

## 5. audio_m6 逐用例结果（冻结树）

| 用例 | 结果 | 关键实测值 |
|---|---|---|
| `it_audio_device_loopback_rms`（REQ-15） | **PASS**（9.16s） | 采集耗时 6.01s；stats `padded=0 dropped=0 discontinuities=1`；去首尾 RMS=**0.0124**（阈 ≥0.002）；FFT 440=**39.4dB**、880=**39.8dB**、本底 0.0dB ⇒ 存在性 39.4/39.8 ≥20dB、两声差 **0.4dB** ≤10dB；48k/2ch、时长恰 288000 帧（钉齐断言） |
| `it_mic_capture_present_or_skip`（REQ-16） | **BLOCKED（环境缺失，非通过；如实报告，不伪绿）** | 本会话两个设计分支均被命中：① 首轮设备枚举 1 个（`耳機 (Baseus Bowie W04 Plus)`，默认；红 §6.7 设备）→ 采集 48k/f32/2ch 正常、去首尾 RMS=**1.2e-5** < 1e-4 数字静音级（无声学回路）→ BLOCKED（静音分支）；② 最终轮 `list_capture_devices()`=**0 个**（独立复核：apploop `probe` 默认采集端点 = `0x80070490 元素找不到`）→ 反证断言成立（`capture_microphone("sr-m6-no-such-device")` **0ms** 返回 `MicUnavailable`，非静默）→ BLOCKED（无设备分支）。正路径（设备存在 + 1200Hz 声学耦合）留耦合机器复跑（红 §8-2） |
| `it_audio_only_wav_duration_and_repair`（REQ-19） | **PASS**（12.72s） | **相位 A**：停止→成品 **0.02s**（停止即成品）；WAV×2 = span 同序：wav-440 5.200s / wav-880 5.200s vs 录制段 5.000s（Δ0.2 ≤1.0）；`pcm_s16le/48k/2ch`；RMS 0.0217/0.0213（≥0.005）；隔离 440 目标 47.1dB vs 880 泄漏 **−45.1dB**、880 目标 47.3dB vs 440 泄漏 **−42.6dB**（≤−40）。**相位 B**（真实子进程强杀）：残留 687680B → 注入 2B 尾部余量 → `repair trimmed_bytes=2`、文件 687682→687680、帧对齐、**幂等二跑无操作**（断言内）；时长 3.581s vs 存活段 3.608s（Δ0.027 ≤1.5）；`repair.frames` 折算 3.581s vs ffprobe 3.581（±0.1）；decode clean、RMS 0.0209 |
| `it_audio_multitrack_ffprobe_fft_all_tracks`（REQ-17 全音轨） | **PASS**（13.71s） | 成品 MKV：恰 1 视频（h264/640×360）+ **恰 3 条独立音轨**（aac/48k/2ch）；时长 8.852s vs 录制段 8.000s（Δ0.852 ≤2.0）；**内容级分类（不依赖轨序）**：音轨0 = 只含 440（880 泄漏 **−42.6dB**）、音轨1 = 系统轨两声均含（42.4/42.8dB，差 **0.4dB** ≤10）、音轨2 = 只含 880（440 泄漏 **−43.6dB**）⇒ **(system,app440,app880) = (1,1,1)**「各独立轨、互不混音」成立 |

## 6. 契约实现要点（按红报告 §3）

- **同一采集核三目标**（红 §3.1/§3.4 建议落实）：`CaptureTarget::{Process, DeviceLoopback, Microphone}`；设备环回 = 默认渲染端点 `Device::get_iaudioclient()`（client 方向 Render）+ `initialize_client(…, Direction::Capture, EventsShared{autoconvert})`——wasapi 0.26 对「Render 端点 + Capture 流」自动置 `AUDCLNT_STREAMFLAGS_LOOPBACK`（S3 apploop `--device-loopback` 同构）；48k/f32/2ch autoconvert 不变。错误分流：设备环回 → `LoopbackUnavailable`；麦克风（设备不存在/不可打开/初始化失败）→ `MicUnavailable`（快速返回）；进程环回维持 M4 语义逐字不动。
- **`list_capture_devices`**：专用线程内 MTA 枚举（§3.1 线程规则，wasapi 类型 !Send）；`is_default` = 与默认采集端点 id 比对；无设备 → `Ok(空列表)`。
- **`wav`**：写侧 = 头部占位（RIFF/data 长度 0）+ s16 流式追加（未封口=崩溃残留形态，写线程每包落盘）；`repair_wav` = chunk 扫描定位 `fmt`/`data`（block_align 从 fmt 读，缺省 4）→ **实际文件长度**为准计算数据段 → 尾部 `%block_align` 裁切（测试注入 2 字节 → trimmed=2）→ RIFF/data 长度回填 → 幂等（已封口且帧对齐 ⇒ 零改动）；`WavRepair.frames` = 完整帧数（s16 立体声）。
- **`start_audio_only`**：每源一条采集流（同步握手=激活成功才 Ok，失败即停已启动流）+ 一个 WAV 写线程（独占流）+ 协调线程；`request_stop` 仅置停止位立即返回；写线程见停止位 → 停采集 → 封口；协调线程 join 全部写线程后置 `AudioOnlyFinalized { wav_paths }`（与 sources 同序同长，`next_available` 纯分配禁覆盖）——**停止即成品**（无 concat/后台收尾）。文件命名 `{yyyyMMdd_HHmmss}_audio{i}.wav`（实现自由面，测试不断言）。
- **`start_audio_route` 的 System/Mic**：每源一条独立音轨（各占一条管道 + 写线程），System=设备环回流、Mic=麦克风流；M3/M4/M5 config 结构体字段零改动（全字段字面量兼容）。

## 7. 中间缺陷发现与修复：`SampleSlotPacer::pad_to_ts` 非单调（阻绿项）

- **现象**（首次全文件 L2 跑，日志 `m6-green-defect-audio-m6-first-run.log`）：devlb 880 存在性 17.9dB < 20dB 失败（440=22.3 但整体电平稀释）；alltracks 时长 **11.156s vs 8.000s** 失败；`padded=6,607,597`（6s 采集内补出 ≈137s 静音）。
- **根因链**（诊断日志 `m6-green-defect-diag-devlb.log`）：设备环回包时间戳**领先**同刻 QPC ≈10.7ms（逐包实测 `ts-qpc=+10.7ms`）→ 墙钟续流 `pad_to_ts(qpc)` 的 target 间歇小于水位 → 原实现 `pad = target.saturating_sub(watermark); watermark = target;` **把水位回退**（pad 虽为 0）→ 下一包按「回退量」再次补零（逐包 pad≈990 帧、`padded=594,356`）→ 时间轴被逐包拉伸（实测 1.4–3×，随负载波动），音频被静音段穿插稀释、成品时长膨胀。**违反 REQ-18「禁止同一时间段写两次」的墙钟版本**，且与该方法自身文档「已越过水位时为 0」相悖。
- **修复**（`slots.rs`，一行语义修正）：`watermark = self.watermark.max(target)`——水位**只前进不回退**。L1 未覆盖该方法（`silence_pad_late_packet` 只钉 `on_packet` 契约面，修复后仍全绿）；修复后设备环回 `padded=0`、时长/nominal 全部达标，三套回归全绿。
- **影响面诚实注记**：`pad_to_ts` 为 M4 墙钟续流路径共用（进程环回场景包 ts 落后 QPC，原缺陷表现为偶发重复补零）；本会话回归 `audio_isolation`（含 bp 时长 Δ=0.694 ≤0.75、泄漏 −44.0/−45.3dB）、`multi_route`、`route` 全绿，无回归。M4-green §6-1 的 FFT flake 与本缺陷的因果关系**未单独验证**，不作修复声明。
- 会话中的临时诊断代码（TEMP DIAG）已全部移除，终态无残留。

## 8. 日志归档（docs/milestones/，均为 %TEMP%\opencode 原始日志副本）

`m6-green-red-repro.log`（红复现）、`m6-green-final-ws.log`、`m6-green-final-audio-m6.log`、`m6-green-final-case-devlb.log`、`m6-green-final-case-wav.log`、`m6-green-final-case-alltracks.log`、`m6-green-final-reg-audio-isolation.log`、`m6-green-final-reg-multi-route.log`、`m6-green-final-reg-route.log`、`m6-green-defect-audio-m6-first-run.log`（修复前失败轮）、`m6-green-defect-diag-devlb.log`（根因诊断）。控制台中文在日志中偶现编码折损（cp936↔UTF-8），ASCII 关键值（exit/计数/阈值/dB/Δ/字节数）逐字可核。

## 9. 未决问题（如实记录）

1. **mic 正路径未在本机验证**：两个 BLOCKED 分支（无设备 / 数字静音级无声学回路）均属 §4 环境缺失设计分流；需扬声器—麦克风耦合机器复跑（红 §8-2）。
2. **设备环回绝对电平口径受环境漂移影响**：本会话默认渲染端点 = 显示器 `LS24A600N`（48kHz；红 §6.7 时代为蓝牙耳机）——devlb RMS=0.0124 仍达阈值 6× 余量、两声差 0.4dB；不调阈值，环境差异如实记录。
3. **§4 命名决策**沿红 §8-1（姊妹用例 `…_all_tracks` 承载全音轨模式，未合并进 `it_audio_multitrack_ffprobe_fft`）；REQ-17「追加应用按根去重/快照语义」仍无 §4 覆盖（红 §8-3）。
4. **路由侧 Mic 轨内容级验证**仍需耦合环境（红 §8-4）；路级 `MicUnavailable` 的专用 RouteError 变体本红未钉——实现取 `CaptureFailed{detail}`（明确报错），如后续需专用变体属契约扩展。
5. **3 客户端并列**（2 进程环回 + 1 设备环回）经 alltracks 实测成立（本机新并发面无异常）；红 §8-5 的风险在实际硬件上的余量未矩阵化。
6. **`pad_to_ts` 修复的 M4 flake 侧样本**：预算所限未专门重跑 M4 全量 flake 样本轮；M4-green §6-1/bp 边际注记维持原状（本会话回归轮单轮全绿）。
7. **`WavError` 变体**（Io/NotWav/TooLarge）实现自定，测试仅 Debug 打印（红 §8-7）；RIFF 超限策略为「明确失败」（分段策略属后续轮次）。
8. **SESSIONNAME 运行注记**沿用（红 §8-9）：非交互 shell 须 `$env:SESSIONNAME='Console'`，否则按 §4 BLOCKED（非测试缺陷）。

## 10. 验收结论（对照任务验收项）

| 验收项 | 状态 | 依据 |
|---|---|---|
| 红态复现（exit 101，仅预期 5 error） | ✅ | §3（`m6-green-red-repro.log`） |
| L1 `cargo test --workspace` exit 0 | ✅ | §4-#1（13 passed / 0 failed / 0 warning） |
| `audio_m6` 非环境受限用例全绿（3/3） | ✅ | §4-#2（3 passed）+ #3–#5（逐用例 exit 0） |
| mic 用例按设计 BLOCKED 并说明（不伪绿） | ✅ | §5（两个环境分支；负路径反证通过） |
| 回归 `audio_isolation` 4/4 | ✅ | §4-#6 |
| 回归 `multi_route` 4/4 | ✅ | §4-#7 |
| 回归 `route` 5/5 | ✅ | §4-#8 |
| 测试文件零改动（哈希核验） | ✅ | §1（会话前后 SHA256 一致） |
| specs/、docs/spikes/ 未动；未 git 提交；零新依赖 | ✅ | §1/§2（mtime 与依赖审计） |
| `m6-green-report.md` 落盘且与实际一致 | ✅ | 本文件 + §8 原始日志 |

**结论**：M6 绿阶段达成——`wasapi` 三目标采集（REQ-15/16）、`wav` 写与崩溃修复（REQ-19）、`route::start_audio_only` 纯录音（停止即成品）与 System/Mic 全音轨（REQ-17）均按红报告 §3 契约真实落盘；除 mic 用例因本机环境缺失按设计 BLOCKED（两种分支均命中并说明）外，audio_m6 全绿、回归三套全绿、L1 exit 0 且 0 warning。过程中发现并修复一处阻绿实现缺陷（`pad_to_ts` 非单调，§7）。建议编排者按 §4 命令独立复跑核验（预算：L1 ≈10s、audio_m6 ≈36s、audio_isolation ≈47s、multi_route ≈38s、route ≈61s）。

---

## 11. 2026-10-17 抖动加固：multi_route 三路用例标记消失 flake（测试侧；仅 `multi_route.rs`）

- 范围：消解 `it_three_routes_stop_middle` 偶发「某一路标记占比 0.0000 < 0.02」抖动（M6 遗留）。**仅改** `tests/sr-itests/tests/multi_route.rs`（+ 本小节与归档日志）；未动生产代码、`specs/`、其它测试文件；零新依赖；未 git 提交。
- 会话注记：L2 于交互式会话运行（`query session`：`console`/ID 1/Active，进程 `SessionId=1`），命令显式 `$env:SESSIONNAME='Console'`（沿用红 §8-9 / §9-8 注记）。

### 11.1 失败特征与根因实证

- 历史两次失败（`m6-verify-multi.log` 路 2、`m6-verify-multi-three-single.log` 路 0）：该路 `本路#N 最小占比 0.0000`；同路时长/亮度/运动/解码断言全部正常；其它路最大占比 0.0000（无串扰）。
- 本轮加固过程（置顶已生效、双缓冲未加时）**全量复跑再次命中 1 次**（exit 101；路 0，32 个采样帧中第 23 帧占比 0.0000）。受害产物留存并逐帧复核（`%TEMP%\sr_m5_red_multi_three-0_94356`）：
  - 帧 23：亮度 std=47.95（正常帧 ≈75），客户区整块纯底色——帧内唯一颜色 (55,55,55)（H.264 编码后值；绘制底色 (64,64,64)），**标记块与运动亮条同时缺失**；
  - 帧 22/24 正常（std≈77.0/75.7；本路标记占比 0.1337/0.1094，含运动亮条 (248,254,255)）；
  - 受害帧 PNG 归档：`docs/milestones/m6-hardened-badframe-route0-frame23.png`（诊断证据，非生成产物；先例 docs/spikes/s3/raw/*.spectrum.png）。
  - 结论：**GDI 分步刷新中间态被捕获**——整帧重绘在「底色已刷、标记/亮条未画」的瞬间被采到一帧，而非窗口遮挡。遮挡假设对应历史「整录段/多帧 0.0000」形态（受害路随机的另一类），本轮已一并由置顶消除。

### 11.2 改动（逐项）

1. **置顶不可遮挡**：`MarkerWindow` 创建后（pump 线程内、回传 hwnd 前）`SetWindowPos(hwnd, HWND_TOPMOST, SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE)`；`WM_TIMER`（33ms）心跳重申同款调用，防外部置顶窗口/浮层；3 槽位几何与用例语义逐字不变。
2. **WM_PAINT 双缓冲（超出原指令的最小补充，见 §11.5-1）**：新增 `PaintBuf` + 线程局部缓存（每窗口泵线程独占）+ `Drop` 释放；底色/标记/亮条先画入内存 DC，最后一次性 `BitBlt(..., SRCCOPY)` 提交——原子呈现，杜绝中间态入画；客户区尺寸变化自动重建缓冲；`WM_DESTROY` 释放 GDI 对象。
3. **逐帧日志（仅日志增强）**：`assert_frames_route_identified` 在断言前逐帧打印三路 `marker_fraction`；未改任何断言、阈值、用例名、`#[ignore]` 门与 BLOCKED 语义。

### 11.3 验证（全串行；本机实测；归档 docs/milestones/）

| # | 命令 | 结果 | 日志 |
|---|---|---|---|
| 1 | `cargo test -p sr-itests --test multi_route it_three_routes_stop_middle -- --ignored --nocapture`（严格串行连跑 5 次） | **exit 0 ×5；1 passed ×5**；13.03 / 13.09 / 13.06 / 13.09 / 13.16s | `m6-hardened-three-run1..5.log` |
| 2 | `cargo test -p sr-itests --test multi_route -- --ignored --nocapture` | **exit 0；4 passed / 0 failed / 0 ignored**；37.80s | `m6-hardened-multi.log` |
| 3 | 同 #2（补充轮，非归档必需） | **exit 0；4 passed**；37.85s | `%TEMP%\opencode\m6-hardened-multi-extra.log` |

- **逐帧独立核验**：6 份归档日志共 **570** 条「路由×帧」记录，按 `marker_fraction` 同款箱式规则复算，**本路标记占比 0.0000 的帧 = 0**；补充轮（smoke + #3）另 **237** 条，同样 0 异常。
- `Get-Process ffmpeg,ffplay` 抽查：0 残留。
- 诚实注记：双缓冲前的一次全量复跑为失败轮（exit 101；路 0 单帧 0.0000，3 passed/1 failed，37.00s，细节见 §11.1），该轮日志位置随后由 #2 绿轮覆盖；失败帧数据与产物先于覆盖留存，已逐条记录于本节与归档 PNG。

### 11.4 哈希

| 文件 | SHA256 | 状态 |
|---|---|---|
| `tests/sr-itests/tests/multi_route.rs` | `56577577AE55945E4F9C2EADF6505D7F49E74CE1CDAF45CD8FDDC2DECBE4BEE1`（前 16：`56577577AE55945E`） | **本轮变更**（旧基线 `BFD099C9…CFEDE`，§1 记录） |
| `tests/sr-itests/tests/audio_m6.rs` | `7D62C9ACCEAE4F1D8883D47A23B6933B4ACCA55085B20F775E756EA5254D1B10` | 未变（§1 `7D62C9AC…D1B10`） |
| `tests/sr-itests/tests/audio_isolation.rs` | `B33A566F1CCA258033B3A88A8C6B1AD87B8E7B2AB8504EB3C3E1A7954C769627` | 未变（§1 `B33A566F…69627`） |
| `tests/sr-itests/tests/route.rs` | `D21AB7A43AD9B3DD5BCD37E9CB56063F22FF446E8F7A6C481FCFA0D5ECBE031D` | 未变（§1 `D21AB7A4…E031D`） |
| `tests/sr-itests/tests/common/mod.rs` | `1258527C8131992FA1B00D00A86238D6E45A5D49ADCDFE68E76758526177CBA8` | 未变（§1 `1258527C…7CBA8`） |
| `enum_audio.rs` / `enum_targets.rs` / `shot.rs` | `9D75F46D93A8EDED…` / `FE1DF55DE21EBB4B…` / `7F1C581BA8D53A8C…` | 未变（未列于 §1，本轮零改动） |

- 生产代码（`crates/`、`src-tauri/`）、`specs/`、依赖清单零改动；未 git 提交。
- 说明：§1「未修改任何测试文件」为 M6 音频绿会话记录；本轮仅 `multi_route.rs` 因本加固变更，其余测试文件逐字不动。

### 11.5 未决 / 需编排者知悉

1. **超出原指令的最小补充**：原指定改动（置顶 + 日志）后全量复跑仍命中一次「单帧纯底色」捕获中间态（§11.1 实证），不追加双缓冲则验收不可稳定通过，故在**同一测试文件内**追加 WM_PAINT 双缓冲（无断言/阈值变化）。如编排者不认可该超出，可仅回退 §11.2-2（置顶与日志增强可保留）。
2. 抖动为低频事件：现有证据上限 = 5×单用例 + 2×全量（归档 570 + 补充 237 条逐帧记录，0 异常）；建议编排者按 §11.3 命令独立复跑复核。
3. 「受害路随机」的两种形态（整录段/多帧消失 vs 单帧消失）均有对应加固（置顶 + 双缓冲）；单帧中间态为绘制/捕获竞态，未观察到系统级可复现触发条件。
