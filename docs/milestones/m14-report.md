# M14 报告：成品成功后自动清理原段 + `keep_segments` 设置

- 日期：2026-10-18
- 范围：`crates/sr-pipeline/src/route.rs`、`src-tauri/src/settings.rs`、
  `src-tauri/src/commands.rs`、`ui/app.js`、`ui/api.js`、`specs/001-screen-recorder.md`、
  `docs/design/ui-spec.md`；测试更新
  `tests/sr-itests/tests/{route,m11_route_commit,sync}.rs`。
- 上游：acc-5 验收后增量（M14）；不涉及 PRD、`specs/002-recording-modes.md`、
  `docs/prd/002-*`。
- 约束遵守：未改 PRD；未执行任何 `git` 命令；未重建安装包；未改 `SR_DIAG_AV` 机制；
  未触碰另一条线的规格/PRD。

## 0. 结论

| 项 | 结果 | 证据 |
|---|---|---|
| 默认清理 | 正常停止 + 成品验证成功后递归删除 `<output_dir>/.screenrec_tmp/<session_id>` | §2；§4 L2；§5 真实应用层 |
| 保留开关 | 新设置 `keep_segments: bool`（默认 false）；true 时保留原段 | §2；§4 `it_route_keep_segments_preserves_session_tmp`；§5 |
| 删除失败 | 保留会话目录 + stderr 日志；不把路由判失败 | §2（`cleanup_session_after_finalize`）；代码路径 review |
| 失败/崩溃路径 | 不变：原段保留，recovery 扫描/收尾/丢弃未改 | §4 recovery/route/multi_route 回归 |
| L1 | `cargo test --workspace` exit 0；新增 2 个确定性单测红→绿 | §3；`m14-workspace-L1.log` |
| L2 | route 5/5、recovery 2/2、m11_route_commit 3/3、m11_audio_only_recovery 2/2、multi_route 4/4 | §4；`m14-verify-*.log` |
| 真实验证 | 应用命令层默认 → 会话目录消失；keep=true → 保留 | §5；`m14-e2e.log` |

## 1. 需求映射

| 需求 | 实现 | 验证 |
|---|---|---|
| 1. 成功后删除会话目录（递归） | `finalize()` 在 `safe_commit` 成功后调用 `cleanup_session_after_finalize(&session_dir, keep_segments)`；删除复用 `recovery::discard_session`（`remove_dir_all`） | L1 单测；m11_route_commit 默认清理断言；route 单路用例；真实应用层 |
| 1. 删除失败保留 + 日志，不影响路由成功 | helper 对 `discard_session` 的 `Err` 仅 `eprintln!`，返回 `()`；`finalize` 已返回 `Ok(Finalized)` | 代码路径 review（见 §6） |
| 2. `keep_segments` 默认 false、持久化、UI 复选框 | `Settings.keep_segments`（`#[serde(default)]`）；`Default` false；`commands::start_routes_impl` 透传；设置页复选框「保留原始分段（调试/排查用）」 | settings 单测（默认/旧配置兼容/往返）；真实应用层 `set_settings` 后读取 JSON |
| 3. 失败/崩溃路径不变 | 清理调用位于提交成功之后；`recovery.rs`、`finalize_session`、UI 恢复/清理流程未改 | recovery 2/2、route 强杀用例、multi_route 故障隔离 |
| 4. 规格同步 | §2.5 REQ-21 条目与 §3.1 停止协议改为「成品验证成功后自动清理原段（默认；设置『保留原始分段』开启时保留）；失败不删原段」；§5 新增 M14 行 | `specs/001-screen-recorder.md:77/151/293` |

## 2. 实现（diff 摘要）

### `crates/sr-pipeline/src/route.rs`

- `Shared` 新增 `keep_segments: bool`；`start_route_impl` 增加同参数并写入 `Shared`。
- 新增公共 API：
  - `start_route_with_options(cfg, keep_segments)`；
  - `start_routes_with_options(cfgs, encoder, keep_segments)`。
  - 既有 `start_route` / `start_audio_route` / `start_routes` / `start_routes_with_encoder`
    保持签名不变，统一委托并传 `false`（默认清理）。
- `finalize()` 成功路径（`safe_commit` 之后、写回 manifest 之后）调用
  `cleanup_session_after_finalize(session_dir, keep_segments)`。
- 新增 `cleanup_session_after_finalize`：
  - `keep_segments == true` 或目录已不存在 → 直接返回；
  - 否则 `recovery::discard_session` 递归删除；
  - 删除失败 → `eprintln!` 记录「成品验证成功，但清理原段失败（保留会话目录 …）」，
    不改变 `finalize` 的 `Ok` 结果。
- `Finalized` 文档注明：默认成功后 `segment_list`/`segments` 路径随会话目录清理，
  `keep_segments=true` 时保留；避免调用方误判。
- 新增 `#[cfg(test)] mod tests`：两个 L1 用例覆盖默认删除与 keep=true 保留。

### `src-tauri/src/settings.rs`

- `Settings` 新增 `pub keep_segments: bool`，`#[serde(default)]`（旧配置缺字段 →
  false）；`Settings::default()` 显式 false。
- 测试更新：默认值、完整往返、旧/不完整配置补齐。

### `src-tauri/src/commands.rs`

- `start_routes_impl` 读取一次设置快照，将 `settings.keep_segments` 传给
  `route::start_routes_with_options`；编码器逻辑不变。托盘「开始」共用该路径。

### `ui/app.js` / `ui/api.js`

- 设置页新增复选框 `#set-keep-segments`：「保留原始分段（调试/排查用）」；
  保存时写入 `keep_segments`。
- 提示文案：默认关闭，成功后自动清理；失败时始终保留原段。
- mock 默认设置补 `keep_segments: false`（浏览器演示同样读写该字段）。

### `specs/001-screen-recorder.md`

- §2.5 REQ-21（第 77 行）：改为「成品验证成功后自动清理原段（默认；设置
  『保留原始分段』开启时保留）；失败不删原段」。
- §3.1 停止协议（第 151 行）：改为「成品校验→自动清理原段（默认；设置
  『保留原始分段』开启时保留；失败不删原段）」。
- §5 新增 M14 行（第 293 行）。

## 3. 测试设计（层级选择）与红→绿

**层级选择**：清理行为本质是文件系统分支（删除/保留/失败日志），用
`crates/sr-pipeline/src/route.rs` 内的 L1 单测最便宜且确定（无 WGC/编码器/交互会话，
毫秒级）；然后用 `m11_route_commit` 的 L2 用例验证真实 `finalize` 调用链（默认清理
复用既有并发成功用例的成品断言，不额外录制；keep=true 单独一条短录制约 4s）。
这比在 route 套件新增两条真实录制更省，同时保留端到端证据。

**红**（实现前，见 `docs/milestones/m14-red.log`）：

```text
$ cargo test -p sr-pipeline --lib
error[E0425]: cannot find function `cleanup_session_after_finalize` in this scope
  --> crates\sr-pipeline\src\route.rs:1866:9
error[E0425]: cannot find function `cleanup_session_after_finalize` in this scope
  --> crates\sr-pipeline\src\route.rs:1878:9
```

L2 测试同时引用尚不存在的 `start_route_with_options` / `start_routes_with_options`，
实现前亦为编译失败。

**绿**：

```text
$ cargo test -p sr-pipeline --lib
test route::tests::finalize_cleanup_removes_session_dir_by_default ... ok
test route::tests::finalize_cleanup_keeps_session_dir_when_requested ... ok
test result: ok. 5 passed; 0 failed
```

L2 新增/更新断言：

- `m11_route_commit::it_route_same_second_concurrent_safe_commit`：
  默认成功后 `.screenrec_tmp` 下无会话目录，且每个 `Finalized.segment_list` 的父目录
  不存在；原有成品/解码/无 `.tmp` 断言保持。
- `m11_route_commit::it_route_keep_segments_preserves_session_tmp`（新增）：
  `start_routes_with_options(..., keep=true)` → 成品存在、会话目录/清单/原段存在且非空。
- `route::it_single_route_frames_nonblank`：默认成功路径改为断言会话目录及原段路径
  已不存在。
- `route::it_segments_finalize_concat`：因用例需检查 ffconcat 与逐段时长，显式使用
  `keep_segments=true`；原段参数/时长/可解码断言保持不变。
- `sync::it_window_resize_canvas_stable`：同样因逐段检查而显式 `keep_segments=true`。
- 失败/崩溃断言保持：`route::it_crash_kill_segments_playable`、
  `m11_route_commit::it_route_commit_collision_keeps_existing_and_segments`、
  `recovery::it_fault_injection_disk_full`、`multi_route` 故障路均仍断言原段保留。

## 4. 回归结果（L1 + L2，串行）

环境：Windows 11 25H2 build 26200.9711，`SESSIONNAME=Console`，独占机器；运行期用
`SetThreadExecutionState(ES_CONTINUOUS|ES_SYSTEM_REQUIRED|ES_DISPLAY_REQUIRED)` 保持唤醒。

| 套件 | 命令 | 结果 | 日志 |
|---|---|---|---|
| workspace L1 | `cargo test --workspace` | exit 0，无失败 | `m14-workspace-L1.log` |
| m11_route_commit | `cargo test -p sr-itests --test m11_route_commit -- --ignored --nocapture --test-threads=1` | **3 passed / 0 failed（13.85s）** | `m14-verify-m11_route_commit.log` |
| route | 同上 `--test route` | **5 passed / 0 failed（60.95s）** | `m14-verify-route.log` |
| recovery | 同上 `--test recovery` | **2 passed / 0 failed（48.99s）** | `m14-verify-recovery.log` |
| m11_audio_only_recovery | 同上 | **2 passed / 0 failed（5.85s）** | `m14-verify-m11_audio_only_recovery.log` |
| multi_route | 同上 | **4 passed / 0 failed（38.82s）** | `m14-verify-multi_route.log` |
| audio_isolation（附加，覆盖音频路由默认清理） | 同上 `--test audio_isolation` | **4 passed / 0 failed（49.15s）** | `m14-verify-audio_isolation.log` |
| sync resize（附加，改动测试） | `... --test sync it_window_resize_canvas_stable ...` | **1 passed（12.05s）** | `m14-verify-sync-resize.log` |

关键失败路径证据：

- `recovery::it_fault_injection_disk_full`：注入会话目录写拒绝 → `E_DISK_FULL`；
  解除后恢复收尾成功，**封口段仍在磁盘**（日志：`[diskfull] 失败后保留封口段 1 个`）。
- `route::it_crash_kill_segments_playable`：强杀 sidecar → `E_SIDECAR_EXIT`；
  强杀前闭合段仍存在、可独立解码。
- `multi_route` 故障隔离：故障路 `SidecarExit` 且段保留，幸存路正常收尾。
- `m11_route_commit` 提交冲突：既有文件不被覆盖、无半成品、段保留。

静态检查：`cargo fmt --all -- --check` exit 0；`node --check ui/app.js`、
`node --check ui/api.js` 均 exit 0。

## 5. 真实应用层验证（命令层，非仅测试 API）

方法：构建 debug 应用，临时在 `tauri.conf.json` 的窗口配置加入
`--remote-debugging-port=9333`（仅本次验证），通过 WebView2 CDP 直接调用真实 Tauri
命令：`get_settings` → `set_settings` → `start_routes` → `stop_route`。验证后已恢复
`tauri.conf.json` 并重新构建 debug 应用；恢复前后 SHA256 均为
`95080F11734FA35F3BF28B3B51B01A4D8EA28A1AF742B4FD306FDDE040F819FC`。

| 场景 | 设置 | 录制 | 成品 | `.screenrec_tmp/<session>` |
|---|---|---|---|---|
| 默认 | `keep_segments=false` | Monitor 0，640x360@30，约 6.5s | `…\out-default\20261018_144027_Monitor 0.mkv`（717,390 B） | **不存在（0 个会话目录）** |
| 保留 | `keep_segments=true` | 同上 | `…\out-keep\20261018_144035_Monitor 0.mkv`（717,284 B） | **保留** `1792305635129_1`：manifest.json 635 B、2 个段（578,688 + 139,004 B）、最终 ffconcat |

- `set_settings` 后直接读取 `%LOCALAPPDATA%\com.screenrec.desktop\settings.json`，
  确认 `keep_segments` 与 `output_dir` 已持久化（两次均断言）。
- 真实用户设置文件在验证前备份、验证后恢复，恢复前后 SHA256 均为
  `7895B72457F4C666387D5D47BC7FF0FC6EC18CB8AA32C3C7D227E59E820400BE`。
- 为满足单实例插件，验证前关闭了一个**空闲的旧 release 实例**（pid 98508；无 ffmpeg
  子进程、最后输出时间为 13:10，距操作约 1.5h）；未影响任何录制。
- 证据日志：`docs/milestones/m14-e2e.log`。

## 6. 未决 / 范围说明

1. **纯录音（`start_audio_only`）未纳入自动清理**：本轮需求锚点为视频录制路
   `route.rs`/原段/分段，`AudioOnlyHandle` 在成功后仍保留 WAV 会话目录（与 M13 行为
   一致）。若编排者希望纯录音也按 `keep_segments` 清理，需要单独确认后增量实现。
2. **删除失败未做故障注入测试**：实现为 best-effort（`discard_session` 失败仅日志、
   返回 `()`，`finalize` 仍 `Ok`），但未用锁定句柄/ACL 构造确定性失败用例。当前通过
   代码路径 review 保证；如需运行时证据可补锁定文件用例。
3. 本轮未重建 NSIS 安装包（按编排者约束），真实应用层验证使用 debug 构建。

## 7. M14b 增量（2026-10-18）：纯录音纳入同一保留策略

用户批准的语义不区分视频/纯录音；本增量把 `start_audio_only`（纯录音 WAV）纳入
M14 同一套清理/保留语义。

- **语义**：纯录音**正常停止且 WAV 安全提交成功后**，默认递归删除其
  `.screenrec_tmp/<session_id>` 会话目录；`keep_segments=true` 时保留。删除失败仅
  stderr 日志、不改变 `wait_finalized` 的成功归因（复用
  `cleanup_session_after_finalize` / `recovery::discard_session`）。失败/崩溃路径不变
  （原段保留，recovery 扫描/收尾/丢弃未改）。
- **注意**：纯录音的 WAV 即成品，安全提交后已移出会话目录；`keep_segments=true`
  保留的是会话目录（manifest 等），不会复制一份成品 WAV。
- **实现**：
  - `route.rs` 新增 `start_audio_only_with_options(cfg, keep_segments)`；既有
    `start_audio_only(cfg)` 保持签名并委托传 `false`（默认清理）。
  - `commit_audio_only(..., keep_segments)` 在全部 WAV 安全提交、manifest 置 `final`
    后调用 `cleanup_session_after_finalize`。
  - `commands::start_audio_only` 读取设置快照并把 `settings.keep_segments` 透传。
- **L1**（`cargo test -p sr-pipeline --lib`）：新增
  `route::tests::audio_only_commit_cleanup_removes_session_dir_by_default`、
  `route::tests::audio_only_commit_cleanup_keeps_session_dir_when_requested`；本包 7/7
  通过（含 M14 既有 2 条）。
- **workspace L1**：`cargo test --workspace` exit 0（`m14b-workspace-L1.log`）。
- **L2 回归**（环境同 §4：Windows 11 25H2 build 26200.9711；串行、
  `SESSIONNAME=Console`、保持唤醒；均 0 failed）：
  `m11_audio_only_recovery` **2/2**（崩溃路径仍保留会话/可恢复）、`route` **5/5**、
  `recovery` **2/2**；日志 `m14b-verify-*.log`。
- 未改 PRD / `specs/002-*` / `docs/prd/002-*` / `SR_DIAG_AV`；未执行 `git`；未重建
  安装包。
- §6 未决 1（纯录音未纳入清理）由本节闭环。

## 8. M14c 增量（2026-10-18）：恢复收尾纳入同一保留策略

L3 实测发现恢复路径（`recover_sessions` → `finalize_session`）成功后原段仍残留
（会话 `1792310265895_0` 恢复出成品后目录仍在）；本增量把恢复收尾纳入 M14/M14b 的
`keep_segments` 策略。

- **语义**：恢复收尾**全部路由成功且 MKV 成品校验通过后**，默认递归删除
  `<output_dir>/.screenrec_tmp/<session_id>`；`keep_segments=true` 时保留原段。
  删除失败仅 stderr 日志、不改变收尾成功归因（成品已提交）。失败/活动拒绝/清单损坏
  路径语义不变（不删除原段）。
- **WAV 边界**：WAV 恢复的成品是就地封口的会话内 WAV（不像正常纯录音会先安全提交到
  输出根）；若按默认策略删目录会丢录音，故 WAV 路由（`repaired_wavs` 非空）不触发
  清理，保持 M11 恢复语义。
- **实现**：
  - `recovery.rs` 新增 `finalize_session_with_options(session_dir, keep_segments)`；
    既有 `finalize_session` 保持签名并委托 `false`（默认清理）。
  - 全部路由收尾成功后调用 `cleanup_session_after_finalize`（`discard_session` 失败
    仅日志，函数返回 `()`）；默认清理后再次收尾返回 `NotFound`（需幂等重试/核对原段
    时用 `keep_segments=true`）。
  - `commands::finalize_session` 读取设置快照并把 `settings.keep_segments` 透传。
- **L1**（`cargo test -p sr-pipeline --lib`）：新增
  `recovery::tests::recovery_finalize_cleanup_removes_session_dir_by_default`、
  `..._keeps_session_dir_when_requested`、`..._retains_wav_only_session`；本包
  **11/11** 通过（含 M14/M14b/route 既有 7 条）。
- **红→绿（L1）**：API 引入前编译红（`finalize_session_with_options` 不存在）；首次
  运行 10 passed / 1 failed——`retains_wav_only_session` 因测试夹具 `WavWriter`
  写句柄未关闭，会话被活动探测判 `Active`；`drop(writer)` 后 **11/11** 绿
  （`m14c-red-green-pipeline-lib.log`）。
- **workspace L1**：`cargo test --workspace` exit 0（`m14c-workspace-L1.log`）。
- **L2 回归**（环境同 §4：Windows 11 25H2 build 26200.9711；串行、
  `SESSIONNAME=Console`、`SetThreadExecutionState` 保持唤醒；均 0 failed）：
  `recovery` **4/4**（新增 44.1k/96k 恢复子集；S3 断言默认清理会话目录；S2/R/R2/
  disk-full 用 `keep=true` 保留原段核对）、`m11_audio_only_recovery` **2/2**
  （WAV 恢复仍保留会话/可读）、`route` **5/5**；日志 `m14c-verify-*.log`。
- 未改 PRD / `specs/002-*` 需求文本 / `SR_DIAG_AV`；未执行 `git`；未重建安装包。
- 未决：删除失败仍为 best-effort（沿用 M14 §6 未决 2，无确定性故障注入）。

## 9. M15c/acc-7 增量（2026-10-18）：恢复清理前复核成品

acc-7 验收发现 M14c 恢复路径的幂等分支只以 `final_path.exists()` 判成功
（`recovery.rs:735-741` 原实现）：若清单已记成品但文件被截断/零字节，收尾会返回成功，
随后默认清理（`keep_segments=false`）递归删除会话目录，**原段随损坏成品一起丢失**。

- **修复（生产口径）**：
  - `crates/sr-pipeline/src/finalize.rs` 新增 `pub(crate) verify_final_media(ffprobe, p)`：
    文件存在且可读 + 长度 > 0 + ffprobe 可解析 + 容器时长 > 0；错误串包含路径与原因。
  - `recovery.rs` 幂等分支在复用 `final_path` 前调用该函数；失败返回
    `RecoveryError::RecoveryFailed`（detail：`清单已记成品但校验失败（route …）：…；
    原段保留、拒绝清理`），早退使清理不会执行，会话目录/原段保留，可人工处置损坏
    成品后重新恢复。
  - `safe_commit` 的提交前校验闭包改用同一 `verify_final_media`（成品提交与恢复复用
    完全同口径，含 `duration > 0`）。
  - WAV-only 保留边界不变（`repaired_wavs` 非空仍不清理）。
- **测试夹具修正**：原单元夹具用 `b"final-bytes"` 冒充成品，真实校验下必然失败；
  改为用锁定 ffmpeg 生成 0.2s 合法 MKV（`write_valid_final`），生产校验无测试绕过。
- **红→绿**：新增确定性单元回归
  `recovery::tests::recovery_finalize_rejects_corrupt_recorded_final_and_keeps_session`
  （清单记零字节 `final_path` → 必须报错、会话/原段保留、损坏成品不改写）。
  修复前 **红**：收尾返回 `Ok(RecoveryOutcome)` 并删除会话目录
  （`m15c-red-recovery.log`）；修复后 **绿**：1/1（`m15c-green-recovery-unit.log`）。
- **L2 回归**：新增 `it_recovery_corrupt_recorded_final_keeps_session`（同一场景，
  合成段 + 锁定 ffmpeg），随 `recovery` 套件 **5 passed / 0 failed**（49.68s；
  `SESSIONNAME=Console`、`--test-threads=1`、`SetThreadExecutionState` 保持唤醒）；
  日志 `m15c-verify-recovery.log`。route 抽查 `it_single_route_frames_nonblank`
  **1/1**（8.66s，`m15c-verify-route.log`）；workspace L1 `cargo test --workspace`
  exit 0（`m15c-workspace-L1.log`）。
- 未改 PRD / `specs/*` 需求语义 / `SR_DIAG_AV`；未执行 `git`；未重建安装包。
