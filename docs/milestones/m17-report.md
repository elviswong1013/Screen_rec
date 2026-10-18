# M17（P3 MP4 输出）实现报告

- 日期：2026-10-18
- 基线：`8847f0f`（M16，acc-10 ACCEPTED）；工作树含已批准的 spec v5 文本（§3.3 / §4 P3 行），本里程碑未提交 git。
- 规格：`specs/002-recording-modes.md` §3.3（REQ-16..20，v5）+ §4；PRD AC-6..8。
- 约束遵守：未改 PRD/spec 语义（仅更新 §8 M17 状态）；未重建安装包；未触碰 `SR_DIAG_AV`；未提交 git。

## 1. TDD 红 → 绿

| 阶段 | 证据日志 | 红（实现前） |
|---|---|---|
| L1 设置 | `docs/milestones/m17-red-l1-settings.log` | 坏类型（number）导致整份设置解析失败；磁盘坏值把 `output_dir` 重置为默认 |
| L1 收尾事务 | `docs/milestones/m17-red-l1-pipeline.log` | `run_finalize_transaction` = "not implemented"；成功/取消路径失败（失败矩阵断言在转绿前补强为错误归因断言） |
| L1 命令快照 | `docs/milestones/m17-red-l1-commands.log` | Settings 快照提供者调用 0 次（要求恰 1 次） |
| L2 路由/合并 | `docs/milestones/m17-red-l2-route.log`、`m17-red-l2-merge.log` | 预留/成品为 `.mkv`（应为 `.mp4`）；MP4 输入被 EBML 校验拒绝 |
| UI runner | `docs/milestones/m17-red-ui.log` | 无 `#set-container`；app.js/api.js 未接线容器；mock 三路径硬编码 `.mkv` |

转绿日志：`m17-green-l1-settings.log`、`m17-green-l1-pipeline.log`、`m17-l2-container-final.log`、`m17-ui.log`。

## 2. 实现摘要（REQ → 改动）

### REQ-16 设置项
- `sr-pipeline::naming::OutputContainer { Mkv, Mp4 }`（`extension()`/`as_str()`/`parse()`），作为路由/恢复/设置共享类型。
- `Settings.output_container`：具名默认 `mkv`；`serialize_with` + `deserialize_with` 字段级容错（数字/布尔/null/对象/数组/未知字符串/浮点只降级本字段，其余字段保留；磁盘加载同入口）。
- 设置页新增「成品格式」下拉 + 说明（内部始终 MKV 分段/崩溃安全；MP4 为收尾流拷贝；多音轨 MP4 播放器通常选一轨播放）。
- `ui/api.js` real/mock 接线；mock 默认 `mkv`、`setSettings` 归一化；**正常收尾 / 提前终态 / 恢复三条路径**按 `output_container` 返回扩展名（正常路径启动时冻结）。

### REQ-17 收尾事务
- 启动冻结：`start_routes_with_options_rate_and_container(..., out_rate, container)` 贯穿 `start_route_impl`；`Shared.container` 冻结；`reserve_final_path(dir, stem, ext)` 从启动即按容器扩展名预留。旧入口默认 MKV，`out_rate` 保留（命令层/托盘共用 `start_routes_impl`）。
- 统一事务（`finalize.rs::run_finalize_transaction`）：
  1. 空间预检（MP4 新增预算硬预检；MKV 仅记录预算、维持既有不拒绝；查询失败仅日志降级）；
  2. ffconcat 清单（视频轨锚点校准不变）→ concat 到会话目录 `.<final>.concat.mkv.tmp`；
  3. 基础校验（存在/非零/可解析/时长>0/**可选期望轨数=实际去重后 sources**）；
  4. MP4：`ffmpeg -hide_banner -nostdin -i <tmp> -map 0 -c copy -movflags +faststart -f mp4 <out>.tmp`；
  5. MP4 校验（容器族 mov/mp4/m4a/3gp/3g2/mj2、轨数、时长>0）→ `safe_commit` 最终 MP4；
  6. 失败矩阵（spawn/非零/超时/MP4 校验/提交冲突）：保留原段、无成品、清理临时物；`safe_commit` 契约未改。
- remux 独立超时 180s（concat 60s 不变）、stderr 排空、`spawn_sidecar` Job 生命周期、取消可中断。
- 会话级占用：`SessionOccupancy`（进程内注册表 + 会话目录 `.finalize.lock` 独占句柄）；正常收尾与恢复全程持有（含 remux/`.tmp` 阶段）；活动探测优先看占用；Drop 立即释放（失败可立即重试）。
- 批量回滚：仅回滚侧 `cancel_finalize_for_rollback()` + `request_stop`，等待收尾结束后才删除成品/段；通用 `wait_finalized` 超时语义不变。
- 磁盘预检、期望轨数、MP4 校验均与恢复路径共用同一事务。

### REQ-18 恢复、幂等与合并
- `finalize_session` 命令执行时**一次**读取 `output_dir/keep_segments/output_container`（`finalize_session_settings_snapshot`，L1 断言 provider 恰调用一次且三字段同源）；不再扫描时读取。
- `finalize_session_with_options_and_container`：已有有效 `final_path` → 基础校验后复用（不因设置变化重生另一容器）；未完成路由按容器产出；concat/remux 均 `-c copy`（保留既有采样率，不用当前设置重编码）。
- 合并：`sr-merge::inputs` 由 EBML 强制改为容器探测（EBML 或 ISO BMFF `ftyp`）；MP4+MP4、MP4+MKV 均可参与，输出仍 `-f matroska`。
- UI 合并页说明兼容性/多轨 MP4 限制；默认合并路径保持 `.mkv`（合并产物固定 MKV）。

### REQ-19 成品可播
- 测试断言 `format_name` 容器**族**（禁止 `=="mp4"`）、逐轨率/声道/FFT、整文件解码、时长容差。
- `it_sync_dual_container_mp4` 分析 MP4 本身（复用 `sync_dual.rs` 内容级同步口径），MKV 48k 基线保留。
- 高采样率不静默降采样/转码（无相关代码路径）；L3 裁决项见 §5。

### REQ-20 回归
- 所有既有入口默认 MKV（`start_routes_with_options_and_rate`、`finalize_session_with_options` 等委托默认容器），mkv 默认路径零行为变化。

## 3. 测试结果

### L1（`cargo test --workspace`）
- 全绿（日志 `docs/milestones/m17-l1-all.log`；0 failed）。
- 新增：`unit_settings_output_container_*`（4）、`unit_remux_args_*`、`unit_finalize_space_estimate_*`、`unit_finalize_container_failures_{spawn,nonzero_exit,timeout,verify,commit_conflict}`、`unit_finalize_container_success_commits_mp4`、`unit_finalize_cancel_before_commit_does_not_publish`、`unit_commands_recovery_settings_snapshot_once`、`recovery_finalize_failure_releases_occupancy_for_immediate_retry`（失败后占用立即释放、同进程可立即重试、原段不清理）。
- `cargo fmt --all -- --check` 通过；`cargo build --workspace` 无警告。

### L2（串行，`SESSIONNAME=Console`，运行期 `SetThreadExecutionState`，独占机器）
| 套件 | 结果 | 日志 |
|---|---|---|
| container（P3 全行：MP4 短录/四档率流拷贝/多音轨/端点失效/并发预留/提交冲突/回滚/等待超时/恢复容器/幂等复用/remux 占用/合并） | **11/11 pass** | `m17-l2-container-final.log` |
| sync_dual `it_sync_dual_container_mp4`（610s 双路，MP4） | **pass**：route-A max 102.6ms、route-B max 106.3ms；斜率 0.075/0.074 ms/s；边界 2628/2978 样本 | `m17-l2-sync-dual-mp4.log` |
| sync_dual `it_sync_dual_routes_content_10min`（MKV 48k 基线） | **pass**：max 97.8/97.2ms；斜率 0.075/0.077 ms/s | `m17-l2-sync-dual-mkv48k.log` |
| sample_rate（P1 四档率 route + WAV） | **7/7 pass**（44.1k/88.2k/96k 成品率+FFT；48k 基线在既有套件） | `m17-l2-regression-sample-rate.log` |
| route（P1/P2 路由回归） | **5/5 pass** | `m17-l2-regression-route.log` |
| recovery（崩溃矩阵/磁盘满/坏成品/44.1k/96k） | **5/5 pass** | `m17-l2-regression-recovery.log` |
| m11_route_commit | **3/3 pass** | `m17-l2-regression-m11-route-commit.log` |
| m11_audio_only_recovery | **2/2 pass** | `m17-l2-regression-m11-audio-recovery.log` |
| merge（异构多轨 MKV） | **1/1 pass** | `m17-l2-regression-merge.log` |
| endpoint_audio（P2 端点链路） | **6 pass / 1 BLOCKED**：`it_route_display_endpoint_plus_mic_tracks` 因本机无采集设备 BLOCKED（不静默 skip；与 M16 报告一致） | `m17-l2-regression-endpoint.log` |
| render_endpoints（P2 端点/显示器元数据） | **2/2 pass**（3 个活动端点；两台显示器 ContainerId 均命中端点） | `m17-l2-regression-render-endpoints.log` |

### UI runner（两套）
- `node tools/ui/settings-sample-rate.mjs`：**全通过**（含容器下拉/说明/保存重载 + mock 正常/提前终态/恢复三路径 `.mp4`），日志 `m17-ui.log`。
- `node tools/ui/audio-sources-builder.mjs`：**全通过**（REQ-9..14 既有行为），日志 `m17-ui-audio-sources.log`。

## 4. L2 关键断言摘录
- `it_route_container_mp4`：`.mp4`、容器族、h264 640×360、AAC 48k/2ch、660Hz FFT 差 61.6dB、整文件解码、时长差 <2s、keep=false 清理、无 `.tmp`。
- `it_route_container_mp4_multitrack_endpoint_system`：端点≠系统 → 2 条 AAC 48k/2ch；端点 880Hz FFT 差 162.7dB。
- `it_route_container_mp4_four_rates`：44.1k/48k/88.2k/96k 各录 4s → 均 `.mp4`、容器族、AAC 率=所选四档、2ch、整文件解码（四档流拷贝夹具）。
- `it_route_container_mp4_commit_collision`：既有占位文件字节不变、`FinalizeFailed`、原段保留、临时物清理。
- `it_batch_rollback_no_late_commit`：回滚取消计数递增；返回后等待 9s 无迟到成品/无临时物；耗时 12.8s（含 2×6s 注入慢启动，未等待完整 8s remux）。
- `it_recovery_container_mp4`：四档率（44.1k/48k/88.2k/96k）恢复产物均 MP4、音轨率保持源值；改设置为 MKV 后再收尾仍复用原 MP4、无第二容器。
- `it_recovery_active_during_remux`：remux 暂停期间 `scan_sessions.active == true`；第二次收尾 `Active`；首次完成后成功。
- `it_merge_mp4_inputs`：MP4+MP4、MP4+MKV → `matroska`，输入字节不变。

## 5. L3 人工验收（待办，不计入 L2 通过）
1. **系统播放器播放 MP4**：用设置成品格式 MP4 录制 ~10s（48k），在 Windows 自带播放器（Movies & TV / Media Player）播放，记录 OS build（本机 `10.0.26200.9711`）、播放器名称/版本、画面/声音/同步结论。
2. **88.2k/96k MP4 播放兼容裁决**：Windows AAC Decoder 官方不支持 >48kHz。若系统播放器无法播放，记录为已知限制并提交用户裁决；**不得为通过验收静默降采样或转码**。
3. **多音轨 MP4**：端点+系统 2 轨 MP4 在系统播放器中通常只选一轨播放（不自动混音）——记录实际选中轨与用户提示是否足够。
4. **端点真实拔插/禁用**：录制中拔掉 HDMI/DP 端点，确认该路 MP4 优雅收尾、`E_TARGET_LOST` 可见、其它路继续。
5. **真实高采样率端点/设备**：本机端点 48k；96k 捕获行为仍以单测/合成覆盖，真实 96k 端机留 L3。

## 6. BLOCKED / 未决
- **BLOCKED**：`it_route_display_endpoint_plus_mic_tracks`（端点+麦克风 2 轨）——本机无采集设备；与 M16 相同，不算通过。
- 未决：L3 第 1/2/3 项需人工执行并记录；acceptor 复审待跑。
- 已规避但需知晓：多音轨 MP4 的播放器选轨行为依播放器实现而异，产品仅在设置页/合并页如实说明，不做混音。

## 7. M17c 修复（acc-11 阻断项）

- 输入：acc-11 复审阻断 P1-1（回滚删外部文件）、P1-2（取消/提交未串行化 + 后台结束未确认）、P1-3（恢复幂等复核口径不足）、P2-4（磁盘预检重复计入已占盘原段）；UI 覆盖录制中改容器。
- 约束：不改 PRD/spec 语义；未 git；未重建安装包；未触碰 `SR_DIAG_AV`。

### 7.1 改动摘要

| 项 | 文件 | 改动 |
|---|---|---|
| P1-1 | `crates/sr-pipeline/src/route.rs` | 批量回滚失败分支**不再删除** `reserved_final_path()`（取消/失败的收尾没有提交任何成品；预留名可能已被外部文件占用）。仅 `Finalized`（`safe_commit` 成功后才返回）分支删除本次事务成品。新增 `set_route_started_hook_for_tests` 供回归在预留名注入外部占位文件。 |
| P1-2 | `naming.rs` / `finalize.rs` / `route.rs` | 新增 `CommitGate`：取消登记与「校验 + 原子 rename」同锁串行化；提交已完成后到达的取消返回「太晚」，未提交的取消必使提交返回 `Cancelled`。新增 `FinalizeControl`（原子取消标志 + 提交门）并贯穿 `MediaRunner`/`wait_media_child`。批量回滚在 `wait_finalized` 后 `join_worker()`，确定性确认 worker/sidecar/媒体子进程已回收（超时也只记录并继续等待，不再提前返回）。新增 `FinalizeTestPhase`/`FinalizePhaseBarrier` L2 阶段屏障（remux 子进程已 spawn / commit boundary）。 |
| P1-3 | `recovery.rs` / `finalize.rs` | 恢复幂等复用改为与收尾同口径：`MediaExpectation::from_streams(清单 streams)` + 既有成品扩展名推断容器族 + `verify_final_media_with_container`（容器族/轨数/时长）。缺轨、错容器、未知扩展名均拒绝并保留原段/原成品。 |
| P2-4 | `finalize.rs` | 新增 `finalize_incremental_space_needed`（MKV=1S、MP4=2S）与 `check_finalize_space`；预检只比较**新增空间**（总峰值仍记录在日志/错误中）。可用空间恰等于新增预算允许，少 1 字节拒绝；MKV 不新增硬拒绝。 |
| UI | `tools/ui/settings-sample-rate.mjs` | 新增「录制中以 MKV 启动 → 中途把设置改为 MP4 → 收尾仍返回 `.mkv`（启动快照）」检查。Mock 已在 `startRoutes` 冻结 `entry.container`。 |

### 7.2 红 → 绿证据

| 回归 | 红（临时恢复旧行为） | 绿 |
|---|---|---|
| P1-1 外部占位 | `m17c-red-p1-1.log`：`it_batch_rollback_preserves_external_reserved_placeholder` FAILED（外部文件被删） | 同测试 PASS（`m17c-l2-container-debug.log` / `-release.log`） |
| P1-2 remux 中取消 | `m17c-red-p1-2.log`：`it_cancel_during_remux_no_commit` FAILED（返回 `Ok(Finalized)`，迟到成品出现） | PASS |
| P1-2 提交边界取消 | `m17c-red-p1-2.log`：`it_cancel_at_commit_boundary_no_commit` FAILED（取消后仍提交） | PASS |
| P1-2 提交门线性化 | —（新增单测） | `unit_commit_gate_cancel_wins_before_commit`、`unit_commit_gate_commit_wins_then_cancel_is_too_late` PASS |
| P1-3 缺轨 | `m17c-red-p1-3.log`：`recovery_finalize_rejects_final_missing_declared_track_and_keeps_segment` FAILED（被判成功） | PASS（`m17c-red-green-pipeline.log`） |
| P1-3 错容器 | `m17c-red-p1-3.log`：`recovery_finalize_rejects_wrong_container_family_and_keeps_segment` FAILED（被判成功） | PASS |
| P1-3 跨设置复用 | —（正例） | `recovery_finalize_reuses_valid_existing_container_across_setting_change` PASS |
| P2-4 边界 | —（新增 API） | `unit_finalize_incremental_space_budget_excludes_existing_segments`、`unit_finalize_space_boundary_equal_budget_passes_one_byte_short_fails` PASS |
| UI 快照 | —（新增检查） | `PASS 录制中改成品格式 → 收尾仍按启动快照 .mkv`（`m17c-ui-settings.log`） |

### 7.3 验证命令与结果

| 命令 | 结果 | 日志 |
|---|---|---|
| `cargo fmt --all -- --check` | pass | `m17c-fmt.log` |
| `cargo test --workspace` | 全绿（58 个 test-result 段均 ok；0 failed） | `m17c-l1-workspace.log` |
| `cargo test -p sr-itests --test container -- --ignored --nocapture --test-threads=1`（debug） | **14/14 pass**（新增 3 个 L2 用例） | `m17c-l2-container-debug.log` |
| `cargo test --release -p sr-itests --test container -- --ignored --nocapture --test-threads=1`（release） | **14/14 pass**（首跑，端点齐备）；最终重跑 **12 pass + 2 BLOCKED**：`it_route_container_mp4_multitrack_endpoint_system`、`it_route_container_mp4_endpoint_lost` 因运行期第二渲染端点消失（仅剩默认端点；`render_endpoints` 同报 BLOCKED）。两用例在同一最终代码的 debug 跑次中均 PASS，非产品回归。 | `m17c-l2-container-release.log` |
| `cargo test -p sr-itests --test route -- --ignored --nocapture --test-threads=1` | **5/5 pass** | `m17c-l2-route.log` |
| `cargo test -p sr-itests --test recovery -- --ignored --nocapture --test-threads=1` | **5/5 pass** | `m17c-l2-recovery.log` |
| `node tools/ui/settings-sample-rate.mjs` | 全通过（含新增快照检查） | `m17c-ui-settings.log` |
| `node tools/ui/audio-sources-builder.mjs` | 全通过 | `m17c-ui-audio-sources.log` |

### 7.4 未决 / 风险

- P1-2 的取消路径只在**批量回滚**与新增测试入口启用；通用 `wait_finalized` 超时仍不取消录制（既有契约不变）。
- release 最终重跑时本机仅剩 1 个活动渲染端点（`U27G4WS`/`LS24A600N` 端点转为未连接），两个端点用例按 §4 BLOCKED（不静默 skip）；同一最终代码的 debug 跑次 14/14 通过，新增取消/回滚用例在 release 跑次全部通过。
- 回滚的确定性确认依赖 worker 退出：取消会杀死 concat/remux 子进程；ffprobe 校验进程无取消句柄，但本地执行且有界。若 worker 异常 panic，回滚保留预留名（不删外部文件），并以 `WaitTimeout` 归因。
- L3 人工项（系统播放器/高采样率/多音轨/真实拔插）仍按 §5 待办，不因本次修复改变。

## 8. M17d 修复（acc-12 两项 P2）

- 输入：acc-12 P2-A（UI 快照测试假通过）、P2-B（MKV 残留硬预检）。
- 约束：未改 PRD/spec 语义；未 git；未重建安装包；未触碰 `SR_DIAG_AV`。

### 8.1 P2-A：UI 快照测试假通过

**问题**：`settings-sample-rate.mjs` §11 导航到 `?scenario=early_terminal` 后未恢复；§13 在该场景下启动：`ui/api.js` 的提前终态分支在 `startRoutes` 返回前发出 `route_finalized` 并删除 route，runner 的 Promise 在「修改为 MP4」之前就以 MKV 路径 resolve，后续 stop 无 route 可停。旧的 `.mkv` 断言只证明启动快照的输出被提前终态路径返回，不能证明「录制中改设置后收尾仍用启动快照」（§7.2 的 UI 快照 PASS 不成立）。

**修复**（`tools/ui/settings-sample-rate.mjs`）：
- §13 开始前显式 `navigate(base)` 回到正常场景；
- 新增 `runContainerSnapshot()`：启动前设置容器 → 等待 `route_state=recording` → 在改设置前采集 `{id,state,finalized}` → 改容器 → 停止；
- `route_finalized` 必须携带本次启动的 route ID；断言改设置时 route 仍在录制（state=recording 且未收到终态）；
- 正向 MKV→MP4 断言 `.mkv`；新增反向 MP4→MKV 断言 `.mp4`。双分支可识别「收尾读取当前设置」的错误实现：若实现改错，两分支分别得到 `.mp4`/`.mkv`，必失败。

**红 → 绿证据**：
- 变异（`ui/api.js` 收尾改用当前 `containerExt()`）：`m17d-ui-red-snapshot.log` — 正向/反向均 FAIL（实际 `.mp4`/`.mkv`），录制状态与 route ID 断言 PASS。
- 恢复启动快照实现：`m17d-ui-settings.log` — 全通过（含新断言）；`m17d-ui-audio-sources.log` — 全通过。

### 8.2 P2-B：MKV 残留硬预检

**问题**：`finalize.rs::run_finalize_transaction` 对两种容器无条件 `check_finalize_space(...)?`；空间不足时 MKV 也被硬拒绝，偏离 `8847f0f` 既有 MKV 收尾行为（原实现无空间预检）。

**修复**（`crates/sr-pipeline/src/finalize.rs`）：
- 新增 `finalize_space_policy`：MP4 保持硬预检（新增预算不足 → 拒绝并保留原段）；MKV 只输出预算日志（新增/可用/总峰值；不足时注明「按既有行为继续（不硬拒绝）」），不新增拒绝条件；
- 查询失败（`None`）继续降级为日志；
- 新增 `#[cfg(test)]` 线程本地可用空间注入与日志捕获（生产路径不读取）。

**红 → 绿证据**：
- 红（`m17d-red-p2b.log`）：`unit_finalize_mkv_insufficient_space_still_commits_with_budget_log` FAILED（`磁盘空间不足 ... 新增空间预算 16`）；MP4 用例已通过。
- 绿（`m17d-green-p2b.log`）：两用例 PASS；MKV 日志含「新增预算 16 / 总峰值 32」。

### 8.3 验证命令与结果

| 命令 | 结果 | 日志 |
|---|---|---|
| `node tools/ui/settings-sample-rate.mjs` | 全通过（含正向/反向快照） | `m17d-ui-settings.log` |
| `node tools/ui/audio-sources-builder.mjs` | 全通过 | `m17d-ui-audio-sources.log` |
| `cargo test -p sr-itests --test container -- --ignored --nocapture --test-threads=1`（debug，`SESSIONNAME=Console`） | **12 pass / 2 BLOCKED**：`it_route_container_mp4_multitrack_endpoint_system`、`it_route_container_mp4_endpoint_lost`（本机仅 1 个活动渲染端点；与 §7.4 release 跑次同因） | `m17d-container-debug.log` |
| `cargo fmt --all` + `cargo fmt --all -- --check` | pass（exit 0） | `m17d-fmt.log` |
| `cargo test --workspace` | 全绿（0 failed；36 个 sr-pipeline 单测含新增 2 个） | `m17d-workspace.log` |
| `git diff --check` | exit 0（仅 LF/CRLF 提示） | `m17d-diff-check.log` |

### 8.4 未决 / 风险

- container 两例端点用例因环境仅剩 1 个活动渲染端点 BLOCKED（非产品回归）；其余 12 例（含 MP4 收尾/回滚/恢复/合并）PASS。
- P2-A 的 recording 观测基于 mock 事件（`route_state=recording` + 未收到 `route_finalized`），未新增 mock 路由内省 API。
- P2-B 的真实磁盘满行为仍由既有 concat 写失败归因；MKV 不再提前拒绝，MP4 硬预检保持。

