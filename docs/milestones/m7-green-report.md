# M7-C 绿实现报告（TDD 绿阶段：合并/REQ-23 / AC-5）

- 日期: 2026-10-17（全部命令为本机实际执行；日志归档见 §8）
- 角色: **executor-backup 承担实现**（GLM 系 test-writer/executor 额度耗尽至 2026-10-22，编排者指派后备通道；沿 M5/M6 先例）
- 规格: specs/001-screen-recorder.md §2.5（REQ-23 全文）、§3.2（crates/sr-merge = 合并（异构多轨）/REQ-23）、§3.3（`merge_recordings(Vec<PathBuf>, out)`）、§3.4（E_MERGE_INCOMPATIBLE）、§3.5（合并输入拒绝行）、§4（REQ-23 行）、§5 M7 行、§8 D5/锁定 n8.1 release
- 红阶段基线: docs/milestones/m7-red-report.md（**§3 契约=绑定**；§8 未决 10 条；§9 结论）
- 环境: Windows 11 25H2，`Microsoft Windows [Version 10.0.26200.9711]`；cargo/rustc **1.99.0** msvc；ffmpeg/ffprobe = 锁定 `tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin`（§8 冻结）；L2 以 `$env:SESSIONNAME='Console'` 运行（红 §8-9 注记）。仓库无 git 提交历史；未 git 提交。

## 1. 范围与约束遵守

- **未修改任何测试文件**：`tests/sr-itests/tests/merge.rs` SHA256 = `B404C59BD1B4022E7C7FF05E3D9EDBF543CAF7C0E4337EB2332D2B834AAEDF0E`（受控哈希，会话前后一致）；其余测试文件未触碰（本会话仅 `crates/sr-merge` + 本报告/日志）。
- **未改** `specs/`、`docs/spikes/`；**未改其它 crate**（sr-audio/sr-capture/sr-pipeline/src-tauri 零改动）。
- **零依赖新增**：`crates/sr-merge/Cargo.toml` 未动（纯 std），`Cargo.lock` 哈希 `D6817AB08183611B…97A2CE4B` 与红阶段一致未变；`tests/sr-itests/Cargo.toml` 红阶段既定内容未动（`5D339849…1D8018`）。
- ffmpeg/ffprobe 定位沿用 sr-pipeline 同款「仓库内相对路径 + 锁定 n8.1」模式（未引入跨 crate 依赖，见 §5-6）。
- 无调试/临时内容残留：scratch 校验程序在仓库外（§7）；仓库内生产代码仅 `crates/sr-merge/src/{lib,merge,inputs}.rs`。

## 2. 变更文件清单

| 文件 | 变更 |
|---|---|
| `crates/sr-merge/src/lib.rs` | 骨架落地：`pub mod inputs` / `pub mod merge` 由 TODO 空壳改为真实模块声明 + 职责注释（M7） |
| `crates/sr-merge/src/inputs.rs`（新增） | REQ-23 输入前置校验 `check_input`：存在/常规文件/非空/Matroska（EBML 存续标记 `0x1A45DFA3`）；**异构（分辨率/帧率/时长/轨数）一律放行** |
| `crates/sr-merge/src/merge.rs`（新增） | `merge_recordings` / `merge_recordings_cancellable` / `MergeConfig` / `MergeOutcome` / `MergeError`（含取消变体）；sidecar 流拷贝（逐输入显式全轨 `-map i` + `-c copy` + `-f matroska`，无 `-shortest`、无 `-y`）；临时物 + 原子改名提交；RAII 临时物守卫；ffprobe 轨计数；stderr 排空 |
| `docs/milestones/m7-green-report.md` + `m7-green-*.log` ×11 | 本报告 + 原始日志归档（§8） |

## 3. 绿前红态复现（修改前，exit 101）

| 命令 | 结果 |
|---|---|
| `cargo test -p sr-itests --test merge --no-run` | **exit 101**：恰 1 error（`E0432`×4 符号：`merge_recordings`/`MergeConfig`/`MergeError`/`MergeOutcome`），无其它错误（`m7-green-red-targeted.log`） |
| `cargo test --workspace` | **exit 101**：同一错误集（`m7-green-red-ws.log`） |

与红报告 §6.2/§6.3 的最终红态逐条一致（红因=预期符号缺失）。

## 4. 复跑证据（冻结树；命令 + exit + 计数 + 耗时；全部本机实测）

| # | 命令 | 结果（exit / 通过数 / 耗时） | 日志（docs/milestones/） |
|---|---|---|---|
| 1 | `cargo check -p sr-merge --all-targets` | **exit 0**（0 warning 0 error） | `m7-green-check-sr-merge.log` |
| 2 | `cargo test --workspace`（L1） | **exit 0**；L1 **13 passed / 0 failed**（screen_rec_lib 7、drift_hysteresis 1、silence_pad_late_packet 1、pacer_cfr 1、channel_snapshot_rebind 1、ffmpeg_args_snapshot 1、output_naming 1）；L2 门 ignored：merge 1 / route 5 / multi_route 4 / audio_isolation 4 / audio_m6 4 / shot 4 / enum_targets 4 / enum_audio 1；**0 warning**（构建 3.42s） | `m7-green-ws.log` |
| 3 | `cargo test -p sr-itests --test merge -- --ignored --nocapture`（第 1 轮） | **exit 0；1 passed；0.89s**（命令墙钟 2.46s 含 cargo 开销） | `m7-green-l2-merge-first-run.log` |
| 4 | 同上（冻结复核轮） | **exit 0；1 passed；0.88s** | `m7-green-l2-merge.log` |
| 5 | 回归 `--test route … --test-threads=1` | **exit 0；5 passed；60.35s** | `m7-green-reg-route.log` |
| 6 | 回归 `--test multi_route …` | **exit 0；4 passed；37.82s** | `m7-green-reg-multi-route.log` |
| 7 | 回归 `--test audio_isolation …` | **exit 0；4 passed；46.70s** | `m7-green-reg-audio-isolation.log` |
| 8 | 回归 `--test audio_m6 …` | **3 passed / 1 BLOCKED（环境缺失）**；exit 101（BLOCKED=fail 设计门）；35.70s——mic 用例本机无采集设备（`list_capture_devices` 空），与 M6-C 绿报告 §5/§9-1 环境状态一致、未变化；其余 3 用例全绿 | `m7-green-reg-audio-m6.log` |

（#5–#8 各自串行执行，无并行混跑；运行前后 `Get-Process ffmpeg,ffplay` 均 0 残留。）

### 4.1 正向合并关键实测值（测试输出原文，`m7-green-l2-merge.log`）

- 输入 A（2 视频+1 音频，3.021s）：轨 #0 `video/mpeg4:640x360@30` 终点 3.000s；#1 `video/mpeg4:320x180@15` 终点 2.999s；#2 `audio/aac:48000Hz/2ch` 起点 −0.021s / 终点 3.008s。
- 输入 B（1 视频+2 音频，2.023s）：#0 `video/mpeg4:1280x720@25` 终点 2.000s；#1 `audio/aac:44100Hz/2ch` 起点 −0.023s / 终点 2.020s；#2 `audio/aac:48000Hz/1ch` 起点 −0.021s / 终点 2.005s。
- 输出：`video_tracks=3 audio_tracks=3`；6 轨与输入**一一对应且逐轨起点/终点完全一致**（容差 ±0.35s/起点 [−0.2,0.5] 断言内）；容器时长 **3.029s vs max 输入 3.021s**（Δ0.008 ≤0.5；非 `-shortest`、非顺序拼接）；整文件可解码；默认轨 ≤1/类型；两输入逐字节未变（测试断言）。负相位（非 MKV/缺失输入→`InvalidInput`、out==输入→`OutputUnavailable`、不提交、输入保留）全部通过。

## 5. 裁定与未决项处置（红 §8/任务裁定）

| # | 事项 | 处置（已实现/记录） |
|---|---|---|
| 5-1 | crate 归属（红 §8-1/§8-2） | **落 `crates/sr-merge::merge`**（spec §3.2 + 测试导入一致）；未引入任何依赖边（无需改 `tests/sr-itests/Cargo.toml`/`Cargo.lock`——红阶段既有 path 依赖已就位） |
| 5-2 | 单输入 <2（红 §8-4） | **`InvalidInput`**（`detail` 含「需要 ≥2 个输入」；`input`=首输入或空路径）——含**空输入列表**同分支；不静默成功（scratch 实测，§7③④） |
| 5-3 | out 已存在（红 §8-6） | **`OutputUnavailable`（不覆盖策略）**：前置检查 + 原子改名提交前二次检查缩短竞争窗；既有文件不被覆盖（scratch 实测 §7⑤） |
| 5-4 | 轨道命名/默认选择（红 §8-5） | **不改写任何轨道元数据**：`-c copy` 流拷贝保留源轨 codec/尺寸/采样率/标题；默认轨 disposition 随流拷贝——本机 ground truth + L2 实测输出恰 **1 默认视频轨 + 1 默认音频轨**（≤1/类型契约满足） |
| 5-5 | 取消形态（红 §8-3） | **已定义并实现**：`merge_recordings`（同步、无取消点）之外提供 `merge_recordings_cancellable(cfg, &AtomicBool)`——置位（≤50ms 轮询）→ 终止 ffmpeg（kill+wait）→ RAII 清理临时物 → 返回可判别错误 **`MergeError::Cancelled { detail }`**；输入/输出路径不留半成品。scratch 实测响应 0.01s、0 残留（§7①）。`MergeError` 在测试契约最小变体集（3 个）之上新增 `Cancelled`（不破坏测试编译——测试仅 `matches!` 三个既定变体） |
| 5-6 | ffmpeg/ffprobe 定位 | sr-pipeline 同款仓库内相对路径（`CARGO_MANIFEST_DIR/../../tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin/`）；零跨 crate 依赖（未动任何 Cargo.toml/Cargo.lock） |
| 5-7 | 失败不提交（红 §7-R7） | 同目录临时物 `.{name}.merge-tmp-{pid}-{seq}.mkv` → 非空 + ffprobe 轨计数校验 → `rename` 原子提交（同卷）；**RAII 守卫**删除任何未提交临时物（sidecar 失败/取消/改名失败路径；scratch 实测 0 残留） |
| 5-8 | 跨输入对齐/会话偏移（红 §8-7） | v1 不启用（复述 spec §6；实现无偏移参数，各输入按本地零点并入） |
| 5-9 | 磁盘满路径 | 未造满盘（红 §8-3 同注）；「失败保留输入 + 不提交」由非法输入/缺失输入/执行失败/取消四类路径覆盖 |
| 5-10 | 进度 Channel（红 §3.1） | 属 `src-tauri` 命令层接线（本同步 API 不承载；模块文档已注） |

## 6. 实现要点（模块结构）

- `inputs::check_input`：轻量预检（存在/常规文件/非空/EBML **`0x1A45DFA3`** 存续标记 4 字节）——不可接纳输入在进 sidecar 前即拒绝（`InvalidInput`），避免误报为执行失败；**异构布局不经任何拒绝路径**。
- `merge::merge_impl` 流程：输入数量+逐条校验 → 输出校验（同路径/已存在/父目录）→ 临时物（守卫）→ `ffmpeg -hide_banner -nostdin -loglevel error -i A -i B … -map 0 -map 1 … -c copy -f matroska <tmp>` → 等待（50ms 轮询；可取消）→ 非空+ffprobe 轨计数 → 二次存在检查 → 原子改名 → `MergeOutcome{out, video_tracks, audio_tracks}`。
- 轨道计数取自**产物本身**（ffprobe 逐流 `codec_type`），非输入求和——防止「实现谎报」（红 §6.6 变异 M2 的判别面）。
- stderr 持续排空（16KB 尾部诊断；sr-pipeline 同款模式），错误含退出码 + stderr 尾部。

## 7. 取消/失败路径 scratch 实证（仓库外 `%TEMP%\opencode\m7-green-helper`，真实调用本 crate；日志 `m7-green-path-checks-helper.log`）

| 用例 | 断言目的 | 实测输出 |
|---|---|---|
| ① 取消（标志预置位） | `Cancelled` + 0 残留 | `err=true elapsed=0.01s out_exists=false tmp_residue=0 input_same=true` |
| ② sidecar 失败（EBML 头合法但内容垃圾） | `SidecarFailed` + 清理 + 输入不变 | `err=true out_exists=false tmp_residue=0 input_same=true bad_same=true` |
| ③ 单输入 | `InvalidInput` | `err=true out_exists=false tmp_residue=0` |
| ④ 空输入列表 | `InvalidInput` | `err=true out_exists=false` |
| ⑤ out 已存在（非输入） | `OutputUnavailable` + 既有文件不覆盖 | `err=true content_intact=true tmp_residue=0` |
| ⑥ 正向（纯视频双输入） | `Ok` + 计轨 + 0 残留 | `out=true v=2 a=0 out_exists=true tmp_residue=0` |

（helper 为仓库外 scratch；其自身 1 条 unused-import 警告属 helper 代码，与本仓无关。仓库构建 0 warning 见 §4-#1/#2。）

## 8. 日志归档（docs/milestones/）

`m7-green-red-targeted.log`、`m7-green-red-ws.log`（红复现）、`m7-green-check-sr-merge.log`、`m7-green-ws.log`（L1）、`m7-green-l2-merge-first-run.log` / `m7-green-l2-merge.log`（M7 L2 两轮）、`m7-green-reg-route.log`、`m7-green-reg-multi-route.log`、`m7-green-reg-audio-isolation.log`、`m7-green-reg-audio-m6.log`（回归）、`m7-green-path-checks-helper.log`（§7 scratch 实证）。控制台中文在日志中偶现编码折损（cp936↔UTF-8），ASCII 关键值（exit/计数/时长/Δ/字节）逐字可核。

## 9. 未决问题（如实记录）

1. **EBML-合法但损坏的输入**（如截断 MKV）会通过轻量预检、由 ffmpeg 拒绝而落 `SidecarFailed` 而非 `InvalidInput`；红契约的强制面（非 MKV/缺失）已覆盖。如需「未被 MKV 深度接纳」的全输入 ffprobe 预检属后续增强（成本：每输入一次探测）。
2. **提交竞态**：`exists` 前置检查与 `rename` 之间仍有极窄窗口（已加二次检查；当前实现为 Windows `rename` 语义）。完全原子的 no-replace 提交（hard_link+delete 等）未实现。
3. **取消粒度 ≤50ms**（轮询周期）；取消仅覆盖 ffmpeg 等待期（进入本函数后）。UI 层取消的接线属 `src-tauri` 轮次。
4. **磁盘满**未实测（同上 §5-9）；`out` 要求父目录存在且可写（跨卷不同目录的临时物策略不适用——临时物强制与 out 同目录）。
5. **merge 用例仍按 §4 L2 通则校验交互会话**（红 §8-8）：本机按 `$env:SESSIONNAME='Console'` 运行；若编排者希望 CI/Session 0 可跑需 spec 层开例外（本实现本身无设备依赖）。
6. **输入重复路径**（同一文件出现两次）未钉行为——当前实现允许（逐输入 `-map`）；红未测。
7. **时间预算未断言**（沿红 §8-10）：本机合并调用 ≈0.9s 量级（含合成/探测开销由测试承担）。

## 10. 验收结论（对照任务验收项）

| 验收项 | 状态 | 依据 |
|---|---|---|
| L1 `cargo test --workspace` exit 0 | ✅ | §4-#2（13 passed / 0 failed / 0 warning） |
| M7 L2 `merge` 全绿 | ✅ | §4-#3/#4（两轮 1 passed，exit 0，≈0.9s） |
| 回归 `route` 5/5 | ✅ | §4-#5 |
| 回归 `multi_route` 4/4 | ✅ | §4-#6 |
| 回归 `audio_isolation` 4/4 | ✅ | §4-#7 |
| 回归 `audio_m6` | ✅（3 绿 + 1 mic 环境 BLOCKED 如实报告） | §4-#8（与 M6-C 既定环境状态一致，非本改动引入） |
| `merge.rs` 哈希复核不变 | ✅ | `B404C59BD1B4022E…AAEDF0E`（§1） |
| 测试零改动 / 未提交 / specs 未动 | ✅ | §1（哈希 + mtime 审计；仅 crates/sr-merge 与本报告/日志落盘） |
| 报告落盘且与实际一致 | ✅ | 本文件 + §8 原始日志 |

**结论**：M7 绿阶段达成——`crates/sr-merge` 按红报告 §3 契约（绑定）真实落盘：`merge_recordings` 流拷贝合并（显式全轨映射、多视频轨并存、异构不误拒、非 `-shortest`、本地零点、输入不可变、失败不提交）经 L2 全绿（逐轨起点/终点与源一致、时长 3.029 vs max 3.021、负相位三例）；取消形态、单输入、out 已存在等未决项按任务裁定实现并经 scratch 实证。建议编排者按 §4 命令独立复跑复核（预算：L1 ≈5–10s、merge L2 ≈2.5s、route ≈60s、multi_route ≈38s、audio_isolation ≈47s、audio_m6 ≈36s）。
