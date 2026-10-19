# M18（P4 区域视频录制）实现报告

- 日期：2026-10-19
- 规格：`specs/002-recording-modes.md` §3.4（REQ-21..24，v5 已批准）+ §4 测试计划
- 前置：P1/P2/P3 已实现（acc-8/10/11-13）；基线提交 `3bb8c36`（M17）
- 状态：**实现完成；M18b acceptor 修复后复审 ACCEPTED（2026-10-19，源码验收）；已随 FFmpeg LGPL 整改内部测试包重建（未正式发布）**

## 1. 实现摘要（按 REQ）

### REQ-21 区域目标与选择流程

- `open_region_overlay(mode, request_id)`（async，缺省 `screenshot` 兼容旧调用）+
  `get_region_context() -> {mode, request_id}`（首次创建与复用一致）。
- 复用窗口：`open` 递增代次并 `eval` 触发 `__regionContextChanged()`；页面重新拉取
  最新上下文（异模式以最新代次为准）。
- `confirm_region_record(request_id, rect)`：代次复核 + 一次性交付；成功路径
  隐藏 overlay → `DwmFlush` + ≥150ms（复用 `wait_for_composition`）→ 经
  `region_target_added` 交付主窗 → 关闭覆盖层；失败恢复可见并显示错误。
- `cancel_region_record(request_id)` 使迟到确认失效；`close_region_overlay` 清理上下文。
- record 路径绝不进截图命令：`take_region_screenshot` 在 record 上下文显式拒绝。
- `src-tauri/capabilities/default.json` 最小 ACL：
  `core:event:allow-listen` / `core:event:allow-unlisten`；真实 Tauri E2E 已验证事件交付。
- UI：录制页「框选区域录制」入口；主窗启动时先建 `region_target_added` 监听并确认就绪，
  按钮 handler 再 `await open`；区域目标显示吸附后尺寸；确认后进入标准启动入口。
- 尺寸：输出 = 选区物理像素向下取偶；overlay 实时显示吸附后最终尺寸；<64×64 拒绝。

### REQ-22 裁剪链与 DTO

- `TargetIn::Region { display_device: String, rect }`：`display_device` 优先稳定
  `device_path`，不可得回退 GDI `szDevice`；两者皆缺 → 录制入口 `E_TARGET_LOST`。
  `display_device` 带 `#[serde(default)]`，旧截图 JSON（仅 `rect`）反序列化兼容，
  截图入口忽略该字段。
- `MonitorEntry` / `MonitorOut` 新增 `gdi_device_name`（不使用友好名代替设备键）。
- 管线 `RouteTarget::Region { display_device, rect }`；启动时按稳定设备键解析单显示器，
  校验矩形完全位于单屏内、非零、吸附后 ≥64×64。
- WGC `Frame::buffer_crop(start_x, start_y, end_x, end_y)` 帧内裁剪；调用前
  `RegionCrop::validate_for_frame` 校验 `0 ≤ start < end ≤ frame`（锁定库 2.0.1
  不检查终点越界）。裁剪后 `RegionConverter` 直接同尺寸 BGRA→NV12，无缩放/letterbox。
- 区域路 `video.canvas_*` 不生效：编码器/管道/成品均使用区域实际尺寸。
- 编码器探测仍 256×256；区域路启动时以真实尺寸 `probe_encoder_size` 初始化验证；
  不支持 → `RegionSizeUnsupported`（E_ENCODER_UNAVAILABLE），不缩放/补边。
- 桌面→帧内坐标 = 桌面 rect − 显示器原点（含负原点/非零偏移）。

### REQ-23 音轨矩阵

- 区域隔离 = 仅所选应用，各自独立轨；命令层拒绝 `system`/`mic`/`render_endpoint`
  （不隐式设备混音）。
- 全音轨 = 系统 + 麦克风 + 追加应用（+ 可选端点）；端点仅全音轨可选；
  `endpoint == system` 仍按实际 ID 去重（P2 语义）。
- 窗口/纯录音端点拒绝矩阵保持不变；区域零音源 → 命令层 `E_INVALID_AUDIO_PLAN`
  + 管线侧兜底拒绝。
- `buildAudioPlan` 为区域拆独立分支 `buildRegionAudioPlan`（不复用显示器分支）。

### REQ-24 失效、恢复与合并

- 生产者闩锁：WGC `on_closed`、分辨率变化、crop 越界 → 先同步写
  `CaptureLossLatch`（首因），worker 判定 `capture_ended` 后**重读**再收尾。
- 显示器监视：路由 worker 每 1s 轮询稳定设备键的拓扑/几何；移除/几何变化 →
  `E_TARGET_LOST` + `StopNotice`（复用 P2），优雅停路并保留已录内容。
- 无新帧/静止桌面不判失效（无超时逻辑；L2 静止 8s 断言 `stop_notice=None`）。
- 恢复：标准路由 + 快照入口；L2 强杀录制子进程 → `scan_sessions` →
  `finalize_session_with_options_and_container` → 尺寸/全部流/解码正确。
- 合并：全轨流拷贝支持异构尺寸（区域 402×300 + 320×240 → MKV 两条视频流）。

## 2. 文件变更

| 文件 | 变更 |
|---|---|
| `crates/sr-capture/src/region.rs`（新） | `RegionCrop` 边界校验、`CaptureLossLatch`、同尺寸 NV12 转换器 + L1 单测 |
| `crates/sr-capture/src/wgc.rs` | 区域 WGC 回调：`buffer_crop`、分辨率/越界闩锁、`start_region_stream` |
| `crates/sr-capture/src/canvas.rs` / `lib.rs` | `bgra_to_nv12` 复用可见性；导出 region 模块 |
| `crates/sr-enum/src/monitors.rs` | `MonitorEntry.gdi_device_name` |
| `crates/sr-pipeline/src/route.rs` | `RegionRect`/区域目标解析/帧内坐标公式、真实尺寸编码、区域监视/闩锁、测试注入钩子 |
| `crates/sr-pipeline/src/sidecar.rs` | `probe_encoder_size`（真实区域尺寸初始化） |
| `src-tauri/src/commands.rs` | Region DTO + `gdi_device_name` + 音源拒绝矩阵 + L1 测试 |
| `src-tauri/src/region.rs` | overlay 上下文/代次、confirm/cancel、region_target_added、隐藏时序、L1 测试 |
| `src-tauri/src/lib.rs` | 管理 overlay 状态、注册新命令 |
| `src-tauri/capabilities/default.json`（新） | 最小 event listen/unlisten ACL |
| `ui/api.js` / `ui/app.js` / `ui/index.html` | real/mock 接线、区域入口/独立音源分支、监听就绪顺序 |
| `ui/region.js` / `ui/region-geometry.js` | 模式化 overlay、record 确认/取消、吸附尺寸显示/纯函数 |
| `tests/sr-itests/tests/region_video.rs`（新） | §4 区域 L2 套件（10 用例） |
| `tools/ui/region-record.mjs`（新） | 区域覆盖层行为 runner |
| `tools/e2e/region-e2e.ps1` / `RegionE2E.psm1` | `-Scenario record` + 顶栏按钮/红按钮/音调/帧断言辅助 |
| `specs/002-recording-modes.md` | §1/§7/§8 M18 状态更新 |

> 说明：工作树中的 `README.md` 改动在本任务开始后出现，非本任务产生/回退，保持原样。

## 3. TDD 红→绿证据

- **红**：新测试对 M17 基线（无 M18 API）编译失败，缺失项与预期一致：
  `RegionRect`、`set_region_latch_hook_for_tests`、`set_region_size_probe_override_for_tests`、
  `gdi_device_name`、`RouteTarget::Region` 的 `display_device/rect` 字段、
  `RouteError::RegionSizeUnsupported`、`RouteHandle::force_capture_ended_for_tests` /
  `latch_region_loss_for_tests`。日志：`docs/milestones/m18-red.log`。
- **绿**：上述测试全部通过（L1 + L2 + E2E），过程中未削弱既有断言。

## 4. 验证结果

### 4.1 L1

```text
cargo fmt --all
cargo test --workspace
```

- 全部 crate 单测/既有 L1：**全绿**（含新增 region/wgc/route/commands/region.rs 单测）。
- 日志：`docs/milestones/m18-l1.log`。

### 4.2 L2（`SESSIONNAME=Console`，`--ignored --nocapture --test-threads=1`）

| 套件 | 结果 | 日志 |
|---|---|---|
| `region_video`（10 用例） | **10 passed / 0 failed** | `docs/milestones/m18-l2-region.log` |
| `route` | 5 passed / 0 failed | `docs/milestones/m18-reg-route.log` |
| `recovery` | 5 passed / 0 failed | `docs/milestones/m18-reg-recovery.log` |
| `container` | 14 passed / 0 failed | `docs/milestones/m18-reg-container.log` |
| `sync_dual`（48k/44.1k/96k，`SR_M11_DUAL_SECS=90` 抽查） | 3 passed / 0 failed | `docs/milestones/m18-reg-sync-dual.log` |

`region_video` 覆盖：48k/96k 尺寸与裁剪内容、MP4、尺寸拒绝、闩锁交错（成功/收尾失败）、
失效保留其它路、强杀恢复、区域+窗口并发、异构尺寸合并、音轨矩阵（隔离无系统泄漏 FFT 负例 /
全音轨系统+应用+端点 / 端点==系统去重 / 零音源拒绝）。分辨率变化/crop 越界由
`validate_region_frame` 的 L1 单测覆盖，WGC 回调调用同一函数后同步闩锁并停采集。

**BLOCKED（如实记录，不算通过）**：本机无采集（麦克风）设备 → `audio_m6` 正路径与
`region_video` 的麦克风分项 BLOCKED；真实 96k 捕获端点留 L3。日志中已打印
`[region-audio] BLOCKED 子项`。

### 4.3 E2E（真实 GUI）

| 场景 | 结果 | 日志 |
|---|---|---|
| E2E `region-e2e.ps1 -Scenario record` | **d0–d15 全 PASS**（含真实 UI 隔离双应用） | `docs/milestones/m18-e2e-record.log`、`docs/milestones/m18-e2e/` |
| `region-e2e.ps1 -Scenario all`（截图回归） | **a0–c1 全 PASS** | `docs/milestones/m18-e2e-shot.log`、`docs/milestones/m18-e2e-shot/` |

record 断言：真实鼠标拖拽 → 覆盖层隐藏 → 开录 → 停止 → 成品尺寸=选区物理像素
（吸附取偶，非整屏）→ MKV/时长 → 全音轨 1 轨 → 首帧/5s 帧纯绿目标未被遮罩压暗
（无覆盖层入镜）→ 660Hz 选中音调存在且 880Hz 未选音调低 28.2dB。
截图回归断言：PNG 尺寸/内容与覆盖前底层逐像素一致（meanDiff=0）、Esc 无残留、
失败路径覆盖层保持可见并显示错误。

> E2E 的音调判据使用锁定 ffmpeg 的频率选择性带通 + `volumedetect`（真实成品文件）；
> 规格要求的正式 FFT 内容断言由 L2 `it_region_video_audio_modes`（rustfft，32768 点）
> 承担（隔离负例 -115.5dB、全音轨逐轨命中）。

### 4.4 UI runners

| Runner | 结果 | 日志 |
|---|---|---|
| `node tools/ui/region-record.mjs` | 全 PASS（**mock-only**：真实 region.html + 假 Tauri 网关/静态断言；不声称真实 GUI） | `docs/milestones/m18-ui-region.log` |
| `node tools/ui/settings-sample-rate.mjs` | 全 PASS | `docs/milestones/m18-ui-settings.log` |
| `node tools/ui/audio-sources-builder.mjs` | 全 PASS | `docs/milestones/m18-ui-audio-sources.log` |

`region-record.mjs` 覆盖：150% DPI 物理换算/向下取偶/单屏包含；record 确认
`request_id + raw rect`、实时显示吸附尺寸、record 不调用截图命令、Esc→cancel、
迟到确认/越界错误可见、复用异模式重新拉取上下文；real/mock 接线与
「主窗先 listen 再 open」静态断言。

## 5. 未决 / 备注

- **acceptor 已复审 ACCEPTED（2026-10-19，源码验收）**；已随 FFmpeg LGPL 整改内部测试包重建，安装/自检见 `ffmpeg-lgpl-package-report.md`。
- **AC-8 系统播放器人工记录**仍待用户（M17 遗留，不在 M18 范围）。
- E2E 成品尺寸读取覆盖层确认后的 UI 最终尺寸并精确比较（398×120）；真实光标定位
  容差只用于输入，不进入产品尺寸断言。
- 麦克风/真实 96k 捕获端点环境缺失，按规格记 BLOCKED；未用 SKIP 冒充通过。

## 6. M18b acceptor 修复（2026-10-19，已复审 ACCEPTED）

> 本节对应 acceptor 的 P1/P2/P3 阻断项；实际重跑证据以本节日志为准。
> acceptor 于 2026-10-19 复审 **ACCEPTED（源码验收）**；安装包尚未重建，仍列 BLOCKED 项不视为通过。

### P1（REQ-21）代次切换与窗口副作用串行化

- **P1-1 等待环消除（异步命令）**：`open_region_overlay` / `confirm_region_record` /
  `cancel_region_record` / `close_region_overlay` 全部为 `pub async fn`。cancel/close
  不再在主线程同步阻塞获取 lifecycle；后台 open 持 lifecycle 等待主线程创建窗口时，
  主线程仍可处理消息。async 内不持 `std::MutexGuard` 跨 `.await`。
- **P1-2 Closing 状态 + 实际销毁等待**：`RegionOverlayInner.closing`；
  - `open_region_overlay` 在 lifecycle 内先 `ensure_open_allowed()`：`closing=true` →
    `E_INTERNAL`，不得复用/创建同 label 窗口；
  - `mark_closing_if_current`（confirm 成功）/`mark_closing_if_request`（cancel）/
    `mark_closing`（close）在 lifecycle 内置位；
  - `wait_for_overlay_destroyed(app,label,2s)`：`close()` 后 10ms 短轮询
    `get_webview_window(label).is_none()`；**在 lifecycle 内同步执行**（阻塞 async worker，
    不阻塞主线程）；可注入 probe 的 `wait_until_destroyed` 作为测试 seam；
  - cancel：invalidate → emit cancel → mark closing → close → wait destroyed →
    成功 clear；超时保持 closing 并返回明确错误；
  - close：context → emit cancel（record）→ mark closing → close → wait destroyed →
    成功 clear；超时保持 closing 并返回明确错误；
  - confirm 成功提交：emit → mark closing → close → wait destroyed → 成功 clear；
    超时保持 closing（后续 open 被拒绝），目标已 emit 故返回 Ok 并记录诊断；
  - `clear`/`clear_if_current` 均把 closing 置 false；旧代次 clear 不会清除新代次 closing；
  - 窗口不存在时直接 clear/Ok。
- **lifecycle 设计**：`lifecycle: Mutex<()>`（与 inner 分离）；`with_lifecycle` 串行
  open/cancel/close/confirm；`run_if_current(generation,request_id,f)` 在同一临界区完成
  check + 副作用。confirm 分三段：枚举/校验在锁外；hide 段在 lifecycle 内；150ms 等待在
  lifecycle 外 `.await`；提交段（emit/close/wait/clear）在 lifecycle 内。
- **关闭回调重入**：`lib.rs` 只在 `window.label()=="main"` 拦 CloseRequested；region
  overlay 的 close 不进入该分支，也未在关闭事件中调用生命周期命令，无递归重入 lifecycle。
- 取消/关闭清理链路：发布 `region_record_cancelled{request_id}`；主窗只清理匹配 request_id。
- UI 防御：`ui/app.js` 的 `state.regionRequestId` 过滤 stale `region_target_added`；
  成功、取消、关闭均清理（保留 mock-only runner 断言）。
- 确定性测试（真实互斥/可注入 probe，不只是谓词）：
  - `unit_region_lifecycle_hide_blocks_open_and_stale_commit_skips_side_effects`：
    A hide 持 lifecycle → open B 阻塞；A 释放后 B begin；A 提交 stale，副作用不执行；
  - `unit_region_lifecycle_cancel_and_old_request_do_not_close_new_session`：
    cancel A 后 A 提交 stale；open B 后旧 request cancel/提交不关闭 B；
  - `unit_region_lifecycle_success_commit_clears_only_current`：只清当前代次；
  - `unit_region_closing_gate_blocks_open_until_cleared_and_old_clear_is_scoped`：
    closing 拒绝 open；旧代次 clear 不清新代次 closing；clear 后允许；
  - `unit_region_closing_waits_for_real_destruction_before_open`：可注入 probe 未确认销毁时
    不得返回成功/open；确认销毁 + clear 后才允许；
  - `unit_region_lifecycle_commands_are_async`：源码签名断言四个生命周期命令均 async；
  - `unit_region_confirm_generation_survives_wait_boundary_only_when_current` 保留。
- 截图路径说明：`take_region_screenshot` 既有截图语义未改（P2 已接受）；
  M18 record/open/cancel/close 已串行；旧截图 E2E 重跑全 PASS。

### P2（REQ-22/23）真实 GUI 隔离双应用与首帧

- 修复位置：`tools/e2e/region-e2e.ps1` + `tools/e2e/RegionE2E.psm1`
  - **真实 UI 隔离双应用**：660Hz 与 880Hz 分别由两个独立 `ffplay` 进程播放；
    660 进程在 app 启动前播放以进入 UI 音频源快照；真实 UIA 点击「隔离」模式并按
    `AutomationId=cfg-region-app-<660 PID>` 勾选 660 应用；随后启动 880 进程（不勾选）；
    区域录制成品断言 1 条应用音轨、660 FFT 单频点存在、880 至少低 15dB。
    实测：660=17.7dB、880=-120.7dB、差 138.4dB（`d15`）。
  - **默认系统正例**（d8）只断言 660 存在；880 明示“未播放，仅参考”，不再冒充负例。
  - 首帧改为 `ffmpeg -i final -frames:v 1`（无 `-ss 1`），5s 帧仍单独取；纯绿目标
    未被覆盖层压暗。
  - 成品尺寸改为读取覆盖层确认后的 UI 最终尺寸（UIA 文本“已选…区域 W×H”），
    与实际视频宽高**精确相等**（d4/d14），不再用容差猜测。
  - `tools/ui/region-record.mjs` 明确输出 **mock-only**；真实 GUI 贯通由 E2E 承担。
- 证据：`docs/milestones/m18-e2e/region-record.mkv`、
  `region-record-isolated.mkv`、`record-first-frame.png`、`record-frame-5s.png`。

### P3（旧截图回归）鼠标容差

- `Send-MouseMove` / `Send-Click` / `Send-Drag` 默认容差恢复为 **2px**；录制场景显式
  `-TolerancePx 6`（仅该场景），旧截图 runner/断言未放宽。
- 成品尺寸由确认矩形精确校验（见 P2），容差仅用于真实输入定位，不进入产品断言。
- 旧截图 E2E `-Scenario all` 重跑：a0–c1 全 PASS（PNG 与覆盖前底层 meanDiff=0）。

### M18b 重跑命令与结果

| 命令 | 结果 | 日志 |
|---|---|---|
| `cargo fmt --all -- --check` | PASS | — |
| `cargo test --workspace` | 全绿 | `m18-l1.log` |
| `cargo check --workspace` | PASS | `m18-check.log` |
| L2 `region_video`（10 用例，串行） | **10/10 PASS** | `m18-l2-region.log` |
| L2 `route` / `recovery` / `container` | 5/5、5/5、14/14 | `m18-reg-route.log`、`m18-reg-recovery.log`、`m18-reg-container.log` |
| E2E `-Scenario record` | **d0–d15 PASS**（真实隔离双应用） | `m18-e2e-record.log` |
| E2E `-Scenario all`（截图回归） | **a0–c1 PASS** | `m18-e2e-shot.log` |
| 三个 UI runner | 全 PASS（region runner 为 mock-only） | `m18-ui-region.log`、`m18-ui-settings.log`、`m18-ui-audio-sources.log` |
| `git diff --check` | PASS | — |

**仍 BLOCKED/未验证**：本机无采集设备 → 麦克风分项 BLOCKED；真实 96k 捕获端点 L3；
AC-8 系统播放器人工记录仍待用户（M17 遗留）。README/法律调查内容未改动。

> 本轮 lifecycle 修复仅改 `src-tauri` 命令层与 UI；`route`/`recovery`/`container`
> 为 M18b 早前重跑结果（pipeline 未变），本轮未重复长套件。
