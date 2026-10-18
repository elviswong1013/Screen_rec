# M9-B 后端命令接线报告（§3.3 Tauri commands + REQ-31 Channel）

- 日期: 2026-10-17（文中所有 cargo 命令均为本机实际执行；日志见 §7）
- 角色: **executor-backup 承担实现**（编排者指派后备通道；沿 M5–M8 先例）
- 规格: specs/001-screen-recorder.md §2.8（REQ-30/31）、§3.1（线程规则/启动停止协议）、
  §3.3（Tauri commands / Channel / 错误路径）、§3.4（Settings/AudioSourceSpec/RouteConfig/错误码）、
  §3.5（错误路径表）
- 既有能力来源（本轮**未修改**）：sr-core（`events::ChannelHub`）、sr-enum（窗口/显示器/音频会话）、
  sr-audio（`AudioSourceSpec`/采集设备）、sr-pipeline（`start_routes` 全成功或回滚 / `start_audio_only` /
  `probe` / `recovery`）、sr-shot（截图）、sr-merge（合并）
- 约束: 仅改 `src-tauri/`（+ 本报告与验证日志）；未改 `crates/`、`tests/`、`specs/`、`docs/spikes/`；
  **未 git 提交**（仓库无提交历史）

## 1. 变更文件

| 文件 | 变更 |
|---|---|
| `src-tauri/src/commands.rs`（新增，1255 行） | §3.3 全部命令 + JSON DTO + Channel 订阅/转发 + 运行时状态（Hub/活动路由表/恢复缓存）+ 错误码映射 + 8 个 L1 单测 |
| `src-tauri/src/lib.rs` | `pub mod commands`；`SettingsState` 提升为 `pub(crate)` 并加 `snapshot()`；setup 中 `manage(CommandsState::new())`；`invoke_handler` 注册 15 个命令；注释与当前行为同步（托盘/关闭语义） |
| `src-tauri/Cargo.toml` | 新增 6 个 **workspace path 依赖**（sr-core/sr-enum/sr-audio/sr-pipeline/sr-shot/sr-merge）；**零新增外部包**（`Cargo.lock` 无新增版本） |
| `docs/milestones/m9-commands-report.md` | 本报告 |
| `docs/milestones/m9-commands-*.log` ×5 | 验证原始日志（§7：check/test ×4 + GUI 启动烟测） |

## 2. 前端 invoke 清单（16 个，含沿用）

| # | JS `invoke` 名 | 参数（JS 键，**顶层 camelCase**） | 返回（JSON） | 备注 |
|---|---|---|---|---|
| 1 | `probe_capabilities` | — | `ProbeOut` | REQ-28；含三层归类明细 |
| 2 | `list_targets` | — | `{monitors, windows}` | REQ-1/2/3；每次调用=完整重枚举 |
| 3 | `list_audio_sources` | — | `[{pid,created_at_unix_ms,process_name,app_name,active}]` | REQ-4 值快照 |
| 4 | `list_audio_devices` | — | `[{id,name,kind,is_default}]` | REQ-16；`kind` 恒为 `"mic"` |
| 5 | `start_routes` | `{routes: RouteConfigIn[]}` | `{route_ids: string[]}` | **全成功或回滚**（管线语义） |
| 6 | `stop_route` | `{routeId}` | `{route_id, requested}` | 「已请求停止」；成品经事件 |
| 7 | `stop_all` | — | `{requested: string[]}` | 终态路由跳过 |
| 8 | `start_audio_only` | `{sources, outputDir?}` | `{route_ids: string[]}` | REQ-19；省略输出目录=设置值 |
| 9 | `take_screenshot` | `{request: {target, output_dir?}}` | `{path}` | REQ-5..7；省略输出目录=设置值 |
| 10 | `merge_recordings` | `{paths: string[], out}` | `{out}` | REQ-23；输入 ≥2 |
| 11 | `recover_sessions` | — | `{sessions: SessionOut[]}` | REQ-22；新发现非活动会话发事件 |
| 12 | `finalize_session` | `{sessionId}` | `{finals, repaired_wavs}` | 幂等；原段永不删除 |
| 13 | `discard_session` | `{sessionId}` | `{session_id, discarded}` | 删除会话临时目录 |
| 14 | `subscribe` | `{channel}` | `StateSnapshot` | REQ-31；立即返回快照 |
| 15 | `get_settings` | — | `Settings` | 沿用（M1-D） |
| 16 | `set_settings` | `{settings}` | `null` | 沿用（M1-D） |

> **命名规则**：Tauri v2 命令参数在 JS 侧默认 camelCase（`routeId`/`sessionId`/`outputDir`），
> **对象内部字段保持 §3.4 的 snake_case**（`canvas_w`/`created_at_unix_ms`/`output_dir`/`device_id`）。
> 事件与快照使用 snake_case 字段。

## 3. JSON 契约与示例

### 3.1 目标 / 音源 / 路由配置（请求侧）

```jsonc
// target：三选一（内部标签 kind）
{ "kind": "window",  "hwnd": 66052 }
{ "kind": "monitor", "index": 0 }
{ "kind": "region",  "rect": { "x": -1280, "y": 0, "w": 1280, "h": 720 } }

// audio.sources：三选一（内部标签 kind）
{ "kind": "app",    "pid": 1234, "created_at_unix_ms": 1729000000000 }
{ "kind": "system" }
{ "kind": "mic",    "device_id": "{0.0.1.00000000}.{...}" }

// RouteConfigIn（video.cursor 可省略，缺省 true，REQ-11）
{
  "target": { "kind": "window", "hwnd": 66052 },
  "video": { "canvas_w": 1920, "canvas_h": 1080, "fps": 30, "cursor": true },
  "audio": {
    "mode": "isolated",
    "sources": [{ "kind": "app", "pid": 1234, "created_at_unix_ms": 1729000000000 }]
  },
  "output_dir": "C:\\Users\\elvis\\Videos\\Screen_rec"
}
```

`mode` 取值 `"isolated"` / `"all_tracks"`（与 Settings 的 `default_audio_mode` 同一套枚举）。
**校验**（命令层，REQ-17）：`isolated` 不得包含 `system` 音源（隔离=目标应用轨[+麦克风]）；
需要系统轨请用 `all_tracks`。`sources` 右侧顺序即音轨顺序（每源一条独立轨，互不混音）。

### 3.2 各命令回执示例

调用侧（参数键=顶层 camelCase）：

```js
const { route_ids } = await invoke('start_routes', { routes: [cfg1, cfg2] });
await invoke('stop_route', { routeId: route_ids[0] });
await invoke('merge_recordings', {
  paths: ['C:\\Rec\\a.mkv', 'C:\\Rec\\b.mkv'],
  out: 'C:\\Rec\\merged.mkv',
});
await invoke('finalize_session', { sessionId: '1792300000000_0' });
```

```jsonc
// probe_capabilities（示例值来自 M8 实测记录，本轮未重跑真实探测）
{
  "os_build": "10.0.26200.9711", "os_supported": true,
  "encoders": [
    { "name": "h264_nvenc", "init_ok": true,  "detail": "..." },
    { "name": "h264_qsv",   "init_ok": false, "detail": "..." }
  ],
  "selected_encoder": "h264_nvenc",
  "loopback_ok": true, "mic_privacy": false, "wgc_ok": true,
  "canvas_limits": { "max_width": 7680, "max_height": 4320 },
  "layers": {
    "loopback": { "status": "ok",          "detail": "process loopback 试激活成功（激活即停）" },
    "mic":      { "status": "unavailable", "detail": "无采集（麦克风）设备" },
    "wgc":      { "status": "ok",          "detail": "WGC 试激活成功（Monitor 0，…，即时停止）" }
  }
}
// status ∈ ok | denied | timeout | canceled | unavailable | failed（REQ-28 六态）
// *_ok / mic_privacy = (status == "ok")，明细以 layers 为准

// list_targets
{
  "monitors": [
    { "index": 0, "name": "Monitor 0", "width": 2560, "height": 1440, "x": 0, "y": 0, "primary": true }
  ],
  "windows": [
    { "hwnd": 66052, "title": "无标题 - 记事本", "app_name": "Notepad",
      "process_name": "Notepad.exe", "pid": 1234, "created_at_unix_ms": 1729000000000 }
  ]
}

// list_audio_sources（active 恒 true：枚举只返回活动会话）
[ { "pid": 4321, "created_at_unix_ms": 1729000000123,
    "process_name": "chrome.exe", "app_name": "chrome", "active": true } ]

// list_audio_devices
[ { "id": "{0.0.1.00000000}.{...}", "name": "麦克风阵列", "kind": "mic", "is_default": true } ]

// start_routes / start_audio_only
{ "route_ids": ["route-1", "route-2"] }

// stop_route（requested=false = 该路已 finalized/failed，未重复请求；下例为成功请求）
{ "route_id": "route-1", "requested": true }

// stop_all
{ "requested": ["route-1", "route-2"] }

// take_screenshot（命名由 sr-shot 决定：shot_{kind}_{millis}_{seq}.png）
{ "path": "C:\\Users\\elvis\\Videos\\Screen_rec\\shot_window_1792300000000_0000.png" }

// merge_recordings
{ "out": "C:\\Users\\elvis\\Videos\\Screen_rec\\merged.mkv" }

// recover_sessions
{
  "sessions": [{
    "session_id": "1792300000000_0",
    "dir": "C:\\Users\\elvis\\Videos\\Screen_rec\\.screenrec_tmp\\1792300000000_0",
    "created_at": 1792300000000,
    "active": false,
    "manifest_ok": true,
    "routes": [{
      "route_id": "route-0", "name": "记事本", "streams": ["v", "a0"],
      "state": "recording", "segments": 3, "final_path": null
    }],
    "sealed_segments": ["...\\out0_00000.mkv", "...\\out0_00001.mkv"],
    "tail_segment": "...\\out0_00002.mkv"
  }]
}

// finalize_session
{
  "finals": [{ "route_id": "route-0", "path": "C:\\...\\20261017_101500_记事本.mkv" }],
  "repaired_wavs": []
}

// discard_session
{ "session_id": "1792300000000_0", "discarded": true }
```

> **两套 route id 命名空间**：运行时 `start_routes`/`start_audio_only` 返回的 `route_ids` 是命令层
> 分配的 `route-{n}`（用于 `stop_route`/事件 id）；恢复侧（`recover_sessions`/`finalize_session`）
> 的 `route_id` 来自会话清单（管线内部 id，如 `route-0`），两者**不可混用**。

### 3.3 Channel（REQ-31）

订阅（快照 + 事件流）：

```js
const channel = new window.__TAURI__.core.Channel();
channel.onmessage = (ev) => handle(ev);          // ev.event ∈ 五种事件名
const snapshot = await window.__TAURI__.core.invoke('subscribe', { channel });
// snapshot = { routes: [{id, state, final_path}], recovery: [SessionOut] }
```

事件（内部标签 `event`；字段 snake_case）：

```jsonc
{ "event": "route_state",     "id": "route-1", "state": "recording" }
{ "event": "route_level",     "id": "route-1", "track": "a0", "rms": 0.25 }
{ "event": "route_error",     "id": "route-1", "code": "E_TARGET_LOST",
  "message": "窗口已关闭", "severity": "error" }
{ "event": "route_finalized", "id": "route-1", "path": "C:\\...\\20261017_101500_记事本.mkv" }
{ "event": "recovery_found",  "session": { /* SessionOut，同 recover_sessions 条目 */ } }
```

- `state` 状态机：`recording → stopping → finalized | failed`（`route_finalized`/`route_error` 为终态）。
  批量启动成功后才产生 `recording`；启动失败（回滚）不产生半批次 route id。
- **告警与电平分通道**：`ChannelHub` 的 alerts（状态/告警/收尾/恢复）与 levels（电平）是两条独立
  通道；转发线程**先取告警**、再单次最多排空 32 条电平，告警不会被电平洪泛饿死（REQ-27/31）。
- **电平 ≤10Hz**：由 `ChannelHub` 每订阅 100ms 节流（首条立即放行，区间内保留最新值 trailing 送达）。
- **发送失败不中断录制**：Channel 写入失败只退出转发线程；hub 在下次发布时回收失效订阅，UI 重载
  后重新 `subscribe` 以快照恢复。
- 事件名与字段与 §3.3 一一对应；`route_level` 当前**无发布者**，见 §8-1。

### 3.4 错误码（invoke 拒绝 = `CODE: 文本`）

| 来源 | 码 |
|---|---|
| 录制启动/运行（`RouteError::Display` 原样） | `E_ENCODER_UNAVAILABLE` `E_CONSENT_DENIED` `E_SIDECAR_EXIT` `E_WAIT_TIMEOUT` `E_LOOPBACK_UNAVAILABLE` `E_TARGET_LOST` `E_CAPTURE_FAILED` `E_START_FAILED` `E_FINALIZE_FAILED` `E_DISK_FULL` `E_INTERNAL` |
| 命令层校验 | `E_INVALID_CONFIG`（空批次/fps=0/画布=0/output_dir 空）`E_INVALID_AUDIO_PLAN`（isolated+system）`E_INVALID_REGION`（区域尺寸 0）`E_ROUTE_NOT_FOUND` `E_INVALID_SESSION_ID` |
| 截图（`ShotError` 映射） | `E_TARGET_MINIMIZED` `E_TARGET_LOST` `E_INVALID_REGION` `E_CAPTURE_UNAVAILABLE` `E_CAPTURE_TIMEOUT` `E_OUTPUT_UNAVAILABLE` `E_INTERNAL` |
| 合并（`MergeError` 映射） | `E_MERGE_INCOMPATIBLE`（仅不可读/非 MKV；异构放行）`E_OUTPUT_UNAVAILABLE` `E_SIDECAR_EXIT` `E_CANCELLED`（同步 API 不会产生） |
| 恢复（`RecoveryError` 映射） | `E_SESSION_NOT_FOUND` `E_SESSION_ACTIVE`（拒绝收尾，§3.5）`E_RECOVERY_FAILED`（清单损坏/无可收尾段；原段保留） |
| 设备/环境 | `E_MIC_UNAVAILABLE`（采集设备枚举失败） |

`E_INVALID_CONFIG`/`E_INVALID_AUDIO_PLAN`/`E_ROUTE_NOT_FOUND`/`E_SESSION_*`/`E_OUTPUT_UNAVAILABLE`/
`E_CAPTURE_*`/`E_TARGET_MINIMIZED` 为命令层补充码（§3.4 枚举未覆盖，语义见上）；`E_CANCELLED`
沿用 sr-merge 变体，本命令不触发取消。

## 4. 线程 / 阻塞 / 生命周期

| 关注点 | 实现 |
|---|---|
| 长任务不阻塞 UI 线程 | 枚举/探测/批量启动/截图/合并/恢复扫描与收尾全部 `async` + `spawn_blocking`（UI 主线程只做参数转换）；`stop_route`/`stop_all`/`subscribe` 为同步但只做内存操作与线程启动 |
| 全成功或回滚 | 直接复用 `sr_pipeline::route::start_routes`（逐路启动，失败时停已启各路并删除成品/段后才返回 Err）；命令层在 Ok 后才分配 route id |
| 停止语义 | `request_stop` 立即返回；每路一个监视线程 `wait_finalized`（1h 超时后继续等）→ `route_finalized` / `route_error`；已在 `stopping` 的路由重复 `stop_route`/`stop_all` 不再重复发状态事件 |
| 纯录音 | `start_audio_only` 一次会话 → N 个 WAV；N 个 route id 共享同一 handle（任一 id 停止即整组停止；簇内 id 一并进 `stopping`） |
| Channel 转发 | 每订阅 1 个转发线程（50ms 轮询告警 + 电平有界排空）；发送失败即退出 |
| 路由表 | `BTreeMap<route_id, slot>`（id 字典序，非时间序）；终态条目保留以便 UI 重绑后从快照看到 `final_path` |
| 恢复缓存 | `recover_sessions` 结果入快照；`finalize/discard` 成功后移除 |

## 5. 设置与输出目录

- `start_routes`：`output_dir` 为请求字段（必填，原样传管线）。
- `take_screenshot` / `start_audio_only`：`output_dir` 可选，省略或空白 → `Settings.output_dir`。
- `recover_sessions` / `finalize_session` / `discard_session`：会话目录 = `Settings.output_dir` +
  `.screenrec_tmp/<session_id>`（REQ-24/D12）；**改设置后旧输出目录的会话不再被扫描**（见 §8-6）。
- `session_id` 校验：拒绝空、`.`、`..`、路径分隔符、`:`、通配符与控制字符（收尾/丢弃会删改磁盘）。
- `discard_session` 按 M8 契约**无条件删除**会话目录（不做活动探测）——UI 应只对
  `active=false` 的会话提供「丢弃」；`finalize_session` 则由 recovery 侧拒绝活动会话
  （`E_SESSION_ACTIVE`，§3.5）。

## 6. 测试（新增 L1，8 个）

`src-tauri/src/commands.rs` 内 `#[cfg(test)]`（不依赖 WebView/交互会话）：

1. `route_config_in_parses_window_isolated_json`——文档 JSON 逐字段解析 + 映射为管线配置。
2. `route_config_in_parses_monitor_all_tracks_json`——`cursor` 缺省 true；system/mic 映射。
3. `invalid_plans_are_rejected_with_codes`——isolated+system / 空批次 / fps=0 / hwnd=0 / 纯录音空源。
4. `event_payload_shapes_match_frontend_contract`——五种事件 JSON 形状（含 `recovery_found.session`）。
5. `route_error_code_and_message_are_separated`——code/message 拆分（事件载荷）。
6. `session_id_validation_rejects_traversal_and_separators`——路径穿越防护。
7. `snapshot_round_trips_route_state`——快照形状。
8. `channel_forwarder_delivers_alerts_and_levels`——真实 `ChannelHub` + Tauri `Channel::new` 捕获端：
   订阅快照 + 告警/电平都经转发线程送达（REQ-31 转发链路可执行验证）。

## 7. 验证（实际执行，冻结树最后一次改动之后）

| # | 命令 | 结果 | 日志 |
|---|---|---|---|
| 1 | `cargo check --workspace` | **exit 0；0 warning 0 error** | `m9-commands-check-workspace.log` |
| 2 | `cargo check -p screen-rec --all-targets` | **exit 0；0 warning** | `m9-commands-check-src-tauri.log` |
| 3 | `cargo test -p screen-rec` | **exit 0；15 passed / 0 failed**（原 7 + 新增 8） | `m9-commands-test-src-tauri.log` |
| 4 | `cargo test --workspace` | **exit 0；L1 24 passed / 0 failed；L2 门 30 ignored**（与基线一致，逐目标全绿） | `m9-commands-test-workspace.log` |
| 5 | GUI 启动烟测：`target/debug/screen-rec.exe` 启动观察 10s 后强杀 | **进程存活 10s、stderr/stdout 均 0 行（无 panic）**、无 ffmpeg 残留；验证 `CommandsState::manage` + 15 命令注册的启动路径 | `m9-commands-gui-smoke.log` |

> 烟测边界：未覆盖 WebView 侧 `invoke` 往返（本轮不改 UI，`ui/index.html` 仍只调
> `get_settings`/`set_settings`）；命令签名与参数形状由 `generate_handler!` 编译期校验 +
> §6 单测（DTO 解析/序列化/转发链路）覆盖。

基线对照（本任务改动前同一命令）：`cargo test --workspace` **exit 0，L1 16 passed**、L2 30 ignored；
改动后 L1 24 passed（+8 新增；`screen_rec_lib` 7 → 15）。**既有测试零回归**。

唯一 warning：`tests/sr-itests/tests/probe.rs` 的 `unused import: EncoderProbe`——**基线即存在**
（M8 绿报告 §5-9 已记录），位于 `tests/`（本任务禁改范围），非本改动引入。`cargo check --workspace`
（任务验收口径）为 0 warning。

## 8. 未决问题 / 缺口（返回编排者）

| # | 事项 | 现状与建议 |
|---|---|---|
| 8-1 | **`route_level` 无数据源** | 事件形状/通道/≤10Hz 节流均已就绪（`ChannelHub::publish_level`），但 sr-pipeline 不暴露任何电平/RMS 读数；命令层**不伪造**。需管线新增读数 API（本轮禁改 `crates/`）；接线点已在 `CommandsState`（hub）。**这是本任务唯一未完全落地的 §3.3 条目。** |
| 8-2 | `region` 目标录制 | `RouteTarget::Region` 在 sr-pipeline 中仍是「M3 未实现」形态：命令接受 JSON 形状并原样返回 `E_TARGET_LOST`（区域截图不受影响）。 |
| 8-3 | 合并进度 | §3.3 提到「进度经 Channel」，但 sr-merge 同步 API 无进度钩子；本命令只返回 `{out}`，不伪造进度。需合并层回调（跨 crate，本轮禁改）。 |
| 8-4 | 显示器 `name` | sr-enum 只暴露 `index`/物理分辨率/位置/主标识，无设备友好名；命令层以 `"Monitor {index}"` 稳定标识（前端如需真名需扩展 sr-enum）。 |
| 8-5 | 新增字段（超集，需前端确认） | `AudioSourceOut.created_at_unix_ms`（**必需**：构造 `{kind:"app"}` 音源并过 REQ-14 身份校验）、`AudioDeviceOut.is_default`（默认设备标识）、`SessionOut.{dir,active,manifest_ok,sealed_segments,tail_segment}`、`RouteStateOut.final_path`。均为只增不减字段。 |
| 8-6 | 输出目录变更 | 恢复扫描/收尾/丢弃均按**当前** `Settings.output_dir` 定位会话；改目录后旧目录的崩溃残留不会被列出（`dir` 字段可提示用户切回）。 |
| 8-7 | 回滚残留的「空会话」 | `start_routes` 回滚会删成品/段/收尾清单，但**会话目录与 manifest 保留**（管线 M8 行为；`RouteHandle` 不暴露 `session_id`，命令层无法定位清理）。失败批次后 `recover_sessions` 可能列出 `sealed_segments=[]` 的会话，只能 `discard_session`。建议 UI 规则：`manifest_ok && sealed_segments.is_empty() && tail_segment == null` → 仅给「清理」。 |
| 8-8 | 已收尾会话仍会列出 | REQ-22「原段永不删除」→ 会话目录保留；`finalize_session` 成功后再次 `recover_sessions` 仍会列出（幂等收尾直接复用成品）。UI 可据 `routes[].final_path/state == "final"` 隐藏或只给「清理」。 |
| 8-9 | 订阅转发线程生命周期 | Tauri `Channel` 无 liveness 信号：WebView 重载后旧转发线程要等下一次「发送失败」才退出；空闲时以 50ms 轮询存活（每订阅 1 线程）。开发期频繁重载会短期累积空闲线程（进程内、可回收），不影响录制。 |
| 8-10 | 托盘「开始/停止」与退出分级 | 本轮按任务不接 UI/托盘操作：托盘仍只有「显示/退出」；`quit` 直接退出（sidecar 由 Job 对象连带终止，残留会话可恢复）。 |
| 8-11 | 合并输出目录 | sr-merge 要求 `out` 父目录已存在（不自动创建）；UI 需保证（保存对话框路径天然存在）。 |

## 9. 证据分级

- **实际执行**（observed）：§7 全部 cargo 命令与计数；§6 单测；GUI 启动烟测（无 panic）；
  `Cargo.lock` 仅新增 6 个 path 依赖（无新外部包）。
- **文档记载**（documented）：§3.2 `probe_capabilities` 示例值取自 M8 绿报告实测（本轮未重跑真实探测）；
  sr-pipeline/sr-shot/sr-merge 的行为语义引自其模块文档与 M3–M8 报告。
- **代码推理**（inferred）：UI 侧调用方式（§2/§3.3 的 JS 片段）未在浏览器中实跑（本轮不改 UI）；
  §8-7/8-8 的 UI 建议规则。
