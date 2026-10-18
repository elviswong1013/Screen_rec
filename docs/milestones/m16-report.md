# M16（P2 全屏音轨）实现报告

- 日期：2026-10-18
- 规格：`specs/002-recording-modes.md` §3.2（REQ-9..15，v7 已批准）+ §4；PRD `docs/prd/002-recording-modes.md` AC-4..5
- 状态：**实现完成（TDD 红→绿）**；L1/UI 全绿，L2 端点链路除「端点+麦克风」组合外全绿；该组合因本机无采集设备 BLOCKED；acceptor 待跑
- 基线：`081c785`（M15/P1 acc-8 ACCEPTED）；P1 语义未回退
- 执行：单执行者承担红/绿（GLM 配额未恢复）

---

## 1. 结论

1. **渲染端点枚举与屏幕匹配**（REQ-9/10）：`list_render_endpoints` 已接线；显示器元数据链路按 `szDevice ↔ viewGdiDeviceName` **名称关联**（非位置），monitorDevicePath 经 CM **设备接口属性**读取 `DEVPKEY_Device_ContainerId`；本机 2 台显示器均与对应 NVIDIA 音频端点 ContainerId 命中。
2. **端点环回捕获**（REQ-11）：`{kind:"render_endpoint", device_id}` 贯通命令 DTO → `AudioSourceSpec::RenderEndpoint` → `CaptureTarget::RenderEndpoint` → WASAPI 设备环回；启动校验存在且为 Render；内容级 FFT 在 48k/96k 均检出端点 880Hz 音调。
3. **失效传播**（REQ-12）：IMMNotificationClient（设备移除/状态非 Active）与 `AUDCLNT_E_DEVICE_INVALIDATED` 双路检测；写线程更新 `stop_notice=E_TARGET_LOST(endpoint=<名>)` + 共享 stop → worker 优雅停路；其它路继续；成功与收尾失败两分支 notice 均可读；保留矩阵三分支通过。`ChannelHub` 先注册后读快照、告警逐订阅清理、命令层快照生成/提交串行化、UI 终态护栏均已落地。
4. **逐项勾选与钉住**（REQ-13/14）：显示器逐屏独立选择，sources = 恰好所选条目（顺序 endpoint/system/mic/apps）；System device_id 启动一次解析并进入捕获目标（激活 `get_device(钉住ID)`，无默认端点二次查询）；去重在轨数/管道分配前；零音源命令层拒绝 + UI 明示。
5. **回归**（REQ-15）：48k 基线 route/multi_route/audio_isolation/recovery/audio_m6/probe/merge/m11_*/multi_route_audio_dual/sync_dual 48k 全部通过（audio_m6 麦克风正路径 BLOCKED）；44.1k/96k isolation 与 recovery 通过。

---

## 2. REQ → 实现锚点

| REQ | 实现 |
|---|---|
| REQ-9 | `crates/sr-enum/src/render_endpoints.rs`：MTA 枚举 Render DeviceCollection，`as_immdevice().OpenPropertyStore` 读 `PKEY_Device_ContainerId`；失败返回空 + 日志。命令 `list_render_endpoints`（`commands.rs`/`lib.rs`）。`ui/api.js` real/mock 双接线。 |
| REQ-10 | `crates/sr-enum/src/monitors.rs`：DisplayConfig `GET_SOURCE_NAME`/`GET_TARGET_NAME` + `join_display_metadata` 名称关联；`CM_Get_Device_Interface_PropertyW` 读 ContainerId；`MonitorOut` 增 `friendly_name/device_path/container_id`。`render_endpoints::match_endpoint` 纯函数（GUID 规范化/全零=None/唯一命中/多端点名称取优/仅 active hint/Ambiguous 不取首项）。`ui/endpoint-match.js`：逐屏状态按 device_path、缺失 path 用 `idx:{i}` 且不跨刷新复用。 |
| REQ-11 | `AudioSourceIn::RenderEndpoint`/`AudioSourceSpec::RenderEndpoint`/`CaptureTarget::RenderEndpoint`；`CaptureTarget::DeviceLoopback { device_id: Option<String> }`（System 钉住）；`validate_render_endpoint`（存在 + Render）；格式策略继承 P1 REQ-7。 |
| REQ-12 | `wasapi.rs`：`StreamEvent::EndpointLost`、HRESULT 识别、进程级 `DeviceMonitor`（IMMNotificationClient 分发）、测试注入走同一分发路径；`route.rs`：写线程 → `Shared::set_stop_notice`（首因）→ `shared.stop` → worker `graceful_stop`；`Finalized.stop_notice` + `RouteHandle::stop_notice`。`events.rs`：subscribe 先注册后快照、publish_alert 逐订阅清理。`commands.rs`：`snapshot_lock` 串行化、快照/终态事件携带 `stop_notice`、失败分支从 handle 读取。`ui/route-state.js`：终态护栏（迟到旧快照/旧状态不回退）。 |
| REQ-13 | `route::resolve_audio_sources`（一次解析 System、按实际 ID 去重、保留端点语义）+ `capture_target_for`（钉住 ID → `get_device`）；`OpenRenderEndpoint` 可注入 opener 单测证明钉住路径不查默认端点。命令层：显式端点仅显示器目标、纯录音拒绝、显示器零音源拒绝。UI：每显示器独立勾选、顺序 [endpoint,system,mic,apps]、隔离/全音轨切换保留勾选并如实计入。 |
| REQ-14 | `to_multi_route` 显示器零音源 → `E_INVALID_AUDIO_PLAN`（含端点/系统/麦克风引导）；`app.js` 启动前拦截并 toast。 |
| REQ-15 | 见 §4 回归清单。 |

### 关键设计说明

- **无包端点续流**：显式非默认端点在无客户端时可能长期 0 包；若首包才建立 T0，音频管道无数据会让 ffmpeg 阻塞读视频，最终触发 5s 视频写超时（红阶段实测）。修复：`SampleSlotPacer::start_at(启动 QPC)` 为设备/端点环回建立 T0 锚点，墙钟续流从首包等待期即补静音；进程环回保持原语义。这同时满足 REQ-12「静音/无包不判失效」。
- **激活边界**：`RenderEndpointOpener` trait 仅暴露 `device_by_id` / `default_device`；生产 `create_capture_client` 经 `open_render_endpoint(opener, pinned)` 选择。单测 fake opener 断言 `device_by_id("pinned-id")` 且 `default_calls == 0`。
- **收尾失败注入**：`RouteHandle::force_finalize_failure_for_tests()`（`#[doc(hidden)]`，生产不调用）仅用于 L2 断言「失败仍保留原段 + notice 可读」，不改变正常路径。

---

## 3. 红 → 绿证据

红日志：`docs/milestones/m16-red.log`（含失败原文/日志摘录）；逐步实现过程中真实观察到的失败：

| 红 | 原因 | 绿 |
|---|---|---|
| `unit_channel_snapshot_rebind` 活订阅首条告警 Timeout | `publish_alert` 存在死订阅即整条丢弃 | 先注册后快照 + 逐订阅 `retain_mut` 清理 |
| `it_monitor_metadata_real` 两屏 container=None → BLOCKED | monitorDevicePath 是设备**接口**路径，`CM_Locate_DevNodeW` 失败 | `CM_Get_Device_Interface_PropertyW` 读接口 ContainerId（devnode 兜底） |
| `it_route_endpoint_runtime_lost_keeps_others` WaitTimeout（97s） | 非默认端点 0 包 → sidecar 背压 → `FlushFileBuffers` 卡住 | T0 启动锚点 + 墙钟静音续流 |
| `it_route_endpoint_silence_does_not_stop` 超时 | 同上 | 同上 |
| UI runner 3 项 FAIL（同分候选/独立预选/终态回包） | 测试夹具等长名称；pending finalized 未以成品路径置 done | 修正夹具；`applyPending` 对 finalized 走 `applyRouteFinalized` |

---

## 4. 测试结果

### 4.1 L1（`cargo test --workspace`）

- 结果：**57 个测试二进制，121 passed / 0 failed**（6/4/7/… 等 `ignored` 为 L2 门控）。
- 日志：`docs/milestones/m16-l1.log`。
- 本次新增/迁移的 L1 覆盖：
  - `crates/sr-enum/tests/render_endpoint_match.rs` 11 项（ContainerId 唯一/多端点取优/名称 hint 仅 active/零命中/全零 GUID/缺名/同分/全 inactive/顺序无关/Ambiguous 不取首项）。
  - `crates/sr-enum/tests/monitor_metadata_join.rs` 4 项（按设备名而非位置关联/元数据缺失只降级字段/空名不匹配/重复名首项）。
  - `crates/sr-core/tests/channel_snapshot_rebind.rs`（迁移 + 终态订阅）；`events.rs` 单元测试（订阅注册先于快照读取的确定性交错）。
  - `src-tauri/src/commands.rs` 新增 `render_endpoint` JSON 解析测试。
  - `crates/sr-audio/src/wasapi.rs` 单测：端点格式策略、`AUDCLNT_E_DEVICE_INVALIDATED`、注入分发、钉住激活不查默认端点、无包锚点续流。
  - `crates/sr-pipeline/src/route.rs` 单测：System 一次解析/去重/钉住后默认变化不回退/捕获目标映射。
  - `src-tauri/src/commands.rs` 单测：端点校验矩阵（窗口/纯录音拒绝、零音源、去重时机）、终态快照/事件 stop_notice、快照生成与终态提交串行化。

### 4.2 L2（逐套件 `--ignored --nocapture --test-threads=1`，`SESSIONNAME=Console`）

| 套件/用例 | 结果 | 日志 |
|---|---|---|
| `render_endpoints`（`it_list_render_endpoints`、`it_monitor_metadata_real`） | **2 passed**；本机 3 个活动端点，2 台显示器 ContainerId 均命中 | `m16-l2-render-endpoints.log` |
| `endpoint_audio::it_route_endpoint_audio_fft_nondefault_48000` | **passed**（FFT 880Hz 差 162.7dB；轨道 aac/48000/2） | `m16-l2-endpoint-fft-48k.log` |
| `endpoint_audio::it_route_endpoint_audio_fft_nondefault_96000` | **passed**（FFT 差 142.2dB；轨道 aac/96000/2） | `m16-l2-endpoint-fft-96k.log` |
| `endpoint_audio::it_route_endpoint_invalid_at_start` | **passed**（不存在端点 → E_TARGET_LOST、无会话目录；非 Render 分支本机无采集设备，内联记 BLOCKED） | `m16-l2-endpoint-invalid.log` |
| `endpoint_audio::it_route_endpoint_runtime_lost_keeps_others` | **passed**（成功+keep=false 清理 / 成功+keep=true 保留 / 收尾失败保留 + notice；其它路继续；静音无包不误停） | `m16-l2-endpoint-runtime-lost.log` |
| `endpoint_audio::it_route_endpoint_silence_does_not_stop` | **passed** | `m16-l2-endpoint-silence.log` |
| `endpoint_audio::it_route_display_endpoint_tracks` | **passed**（仅端点 1 轨；端点==系统去重 1 轨；端点≠系统 2 轨；48k/96k） | `m16-l2-endpoint-tracks.log` |
| `endpoint_audio::it_route_display_endpoint_plus_mic_tracks` | **BLOCKED**（本机 `list_capture_devices` 为空；SKIP 不算通过） | `m16-l2-endpoint-mic-blocked.log` |
| `route` | 5 passed | `m16-l2-reg-route.log` |
| `multi_route` | 4 passed | `m16-l2-reg-multi-route.log` |
| `audio_isolation` | 6 passed（含 44.1k/96k 与背压） | `m16-l2-reg-audio-isolation.log` |
| `recovery` | 5 passed（含 44.1k/96k） | `m16-l2-reg-recovery.log` |
| `audio_m6` | 3 passed + 1 **BLOCKED**（麦克风正路径：本机无采集设备） | `m16-l2-reg-audio-m6.log` |
| `probe` | 1 passed | `m16-l2-reg-probe.log` |
| `enum_targets` / `enum_audio` | 4 + 1 passed（MonitorEntry 新字段未破坏枚举契约） | `m16-l2-reg-enum-*.log` |
| `merge` | 1 passed | `m16-l2-reg-merge.log` |
| `m11_route_commit` | 3 passed | `m16-l2-reg-m11-route-commit.log` |
| `m11_audio_only_recovery` | 2 passed | `m16-l2-reg-m11-audio-only-recovery.log` |
| `multi_route_audio_dual` | 1 passed | `m16-l2-reg-multi-route-audio-dual.log` |
| `sync_dual::it_sync_dual_routes_content_10min`（48k 基线） | **passed**（610s；max ≤98.4ms，不累积） | `m16-l2-reg-sync-dual-48k.log` |
| `sync_dual::it_sync_dual_routes_content_10min_44100` | **passed**（610s；max ≤92.7ms） | `m16-l2-sync-dual-combined.log` |
| `sync_dual::it_sync_dual_child_mode` | passed | `m16-l2-sync-dual-combined.log` |
| `sync_dual::it_sync_dual_routes_content_10min_96k` | **passed**（610s；max ≤123.2ms，不累积；音轨 aac,96000,2） | `m16-l2-reg-sync-dual-96k.log` |

### 4.3 UI runner（单独执行）

- `node tools/ui/audio-sources-builder.mjs` → **全部通过**（静态 real/mock 接线 + 生产纯函数 + 真实 UI 点击捕获 start DTO）。
  - 覆盖：隔离预选仅端点；端点+麦克风 2 源；取消端点→系统预选；全取消→REQ-14 提示；模式切换勾选保留；刷新不覆盖；index 交换不串屏；双屏无元数据临时键不复用/不合并；终态早于启动回包；迟到旧状态不覆盖终态；失败告警+成品路径并存。
  - 日志：`docs/milestones/m16-ui-audio-sources.log`。
- `node tools/ui/settings-sample-rate.mjs` → **全部通过**（M15 回归；日志 `m16-ui-settings.log`）。

### 4.4 环境

- Windows 11 `10.0.26200.9711`，`SESSIONNAME=Console`，交互式会话；锁定 ffmpeg `n8.1.3`。
- 渲染端点 3 个：`LS24A600N (NVIDIA HDA)`、`數位輸出 (High Definition Audio Device)`、`U27G4WS (NVIDIA HDA)`；采集设备 0 个。
- 显示器：LS24A600N（#0）/ U27G4WS（#1）；ContainerId 与对应端点一致。

---

## 5. 未决 / BLOCKED / 限制

1. **BLOCKED（环境）**：`it_route_display_endpoint_plus_mic_tracks` 与 `audio_m6::it_mic_capture_present_or_skip` 需采集设备；本机 0 个采集端点。端点+麦克风 2 轨的**命令/管线逻辑**已由 L1（命令校验）与 UI runner（2 源 DTO）覆盖；真实麦克风成品轨数留待有采集设备的 L2/L3。
2. **非 Render 启动校验分支**：本机无采集设备，`it_route_endpoint_invalid_at_start` 中该子分支内联记 BLOCKED；无效 device_id 分支已实际通过。
3. **L3（按规格）**：真实拔插/禁用端点、真实 96k 采集端机、克隆/同型号双屏 ContainerId 完整性、其它驱动 ContainerId 缺失场景。
4. **UI 手动端点候选**：Ambiguous 时以候选提示 + 端点下拉供用户确认（默认系统混合）；未确认前勾选端点不绑定任何设备（acc-9 P1-2 已移除枚举首项兜底），符合「不取首项、由用户确认」契约。

---

## 6. 变更文件

产品/测试：

- `crates/sr-enum/src/render_endpoints.rs`（新：端点枚举 + 纯匹配）
- `crates/sr-enum/src/monitors.rs`（DisplayConfig/CM 元数据链路；MonitorEntry 字段）
- `crates/sr-enum/src/lib.rs`、`crates/sr-enum/Cargo.toml`（模块/feature）
- `crates/sr-enum/tests/render_endpoint_match.rs`、`monitor_metadata_join.rs`（新）
- `crates/sr-audio/src/wasapi.rs`（端点目标/校验/失效检测/DeviceMonitor/注入、无包锚点）
- `crates/sr-audio/src/slots.rs`（`start_at` 续流锚点）
- `crates/sr-core/src/events.rs`（订阅顺序/逐订阅清理）
- `crates/sr-core/tests/channel_snapshot_rebind.rs`（§4 迁移 + 终态订阅）
- `crates/sr-pipeline/src/route.rs`（端点源/钉住解析/去重/stop_notice/失效处理/失败注入）
- `src-tauri/src/commands.rs`（DTO/命令/校验/快照串行化/stop_notice/端点枚举）
- `src-tauri/src/lib.rs`（命令注册）
- `tests/sr-itests/tests/render_endpoints.rs`、`tests/sr-itests/tests/endpoint_audio.rs`（新）
- `ui/api.js`、`ui/app.js`、`ui/index.html`、`ui/endpoint-match.js`、`ui/route-state.js`
- `tools/ui/audio-sources-builder.mjs`（新）
- `specs/002-recording-modes.md`（§8 M16 状态）

验证/证据：

- `docs/milestones/m16-red.log`
- `docs/milestones/m16-l1.log`
- `docs/milestones/m16-l2-*.log`、`m16-l2-reg-*.log`
- `docs/milestones/m16-ui-*.log`
- 本报告

---

## 7. acc-9 P1 修复（2026-10-18，M16 未提交工作树内）

验收发现两项可复现规格违例；本次仅改 UI 层与测试工具，不动后端/规格语义。

### 7.1 P1-1：REQ-12 终态单调护栏覆盖注册/缓存路径

- `ui/app.js` `registerRoute`：已有卡片改为**只合并展示元数据**（kind/label/icon/trackLabels），不重置 `state/startedAt/path/paths/error`；启动回包或快照对同 id 再注册不再把 `done/failed` 降回 `starting`。
- `ui/app.js` `mergePending`：注册前缓存事件同样执行终态护栏——已缓存 `done/failed` 后，迟到的更早状态（recording/stopping/…）不得覆盖；done 与 failed 均为终态时 done（成品就绪）优先；`error/final_path/paths/stop_notice` 始终补全不丢。
- 回归（`tools/ui/audio-sources-builder.mjs`，Node VM 加载生产 `app.js`；仅将末尾 `init()` 引导替换为回归导出，不改产品代码）：
  1. 终态后重注册：`done + final_path + stop_notice` → 同 id 再注册 → 仍 done 且字段保留；
  2. 注册前 failed + 随后旧 recording：注册后仍 failed，`error/stop_notice` 保留。

### 7.2 P1-2：REQ-10⑤ Ambiguous 不得取枚举首项

- `ui/app.js` 显示器端点勾选：删除 `firstActive` 兜底。无已确认 `deviceId` 且匹配不是 matched/matched_by_hint 时，勾选端点**不绑定任何设备**，回退并保持系统混合，toast 明示需先在端点下拉选择具体候选；显式选择候选后仍由下拉写入 `device_id` 并计入 sources。
- 测试夹具 `ui/api.js` 增 `?endpoints=ambiguous`：枚举首项 `ep-unrelated`（无关）+ 候选 `ep-candidate-a/b`（同 ContainerId 同分）。
- 真实 UI 点击回归：未确认候选勾选端点 → start DTO 无 `render_endpoint`/`unrelated-first`，系统混合保留；显式选 `candidate-b` → start DTO 绑定 `candidate-b`。

### 7.3 红 → 绿证据

- 红：`docs/milestones/m16c-ui-red.log`，4 FAIL：
  - 生产 app.js 终态后重注册 → `state=starting`（成品路径/告警丢失）；
  - 生产 app.js 注册前 failed + 旧 recording → `state=recording`；
  - Ambiguous 未确认候选勾选端点 → 未回退（端点仍选中）；
  - Ambiguous start DTO → `[{"kind":"render_endpoint","device_id":"ep-unrelated"},{"kind":"system"}]`。
- 绿：`docs/milestones/m16c-ui-green.log`，全部通过（含原有 M16 用例）。

### 7.4 验证

| 命令 | 结果 | 日志 |
|---|---|---|
| `node tools/ui/audio-sources-builder.mjs` | 全部通过（新增 10 项 + 既有） | `m16c-ui-green.log` |
| `node tools/ui/settings-sample-rate.mjs` | 全部通过 | `m16c-ui-settings-regression.log` |
| `cargo fmt --all -- --check` | 0（无输出） | `m16c-fmt.log` |
| `cargo test --workspace` | 57 bins，121 passed / 0 failed（6/4/7… ignored 为 L2 门控） | `m16c-l1.log` |
| `git diff --check` | 0（无 whitespace 错误） | `m16c-diff-check.log` |

### 7.5 变更文件（本次修复）

- `ui/app.js`（`registerRoute` / `mergePending` / Ambiguous 端点勾选）
- `ui/api.js`（`?endpoints=ambiguous` mock 夹具）
- `tools/ui/audio-sources-builder.mjs`（Node VM 生产 app.js 两项回归 + 真实 UI Ambiguous 点击回归）
- `docs/milestones/m16c-*.log`、本报告

### 7.6 未决

- 无新增未决；真实端点拔插等 L3 项仍按 §5 原样保留。未提交、未重建安装包（按编排者要求）。
