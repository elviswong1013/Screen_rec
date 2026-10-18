# M11-A 验收整改报告（后端 P1/P2）

- 日期: 2026-10-17
- 角色: **executor-backup**（按 acceptor REJECTED 清单整改；编排者独立复跑）
- 范围: P1-2 / P1-3 / P1-4 / P2 编码器 / P2 托盘 / P1-1 区域框选后端
- 约束: 未 git 提交；未改 `specs/`、`docs/spikes/`；**未修改任何既有测试文件**
  （仅新增测试文件与 5 个源码内单测）；生产改动外科手术式。

## 0. 结论

| 项 | 状态 | 关键证据 |
|---|---|---|
| P1-2 双路音频 + 长时同步接线（REQ-25/26） | **完成** | 录制管线实际调用 `Timeline` + `DriftController` + 变比重采样（route.rs:1006/1038/1043/1096）；`sync_dual` 610s 双路 PASS（max 97.8/97.1ms、斜率 0.0796/0.0807ms/s） |
| P1-2 双路均带音频 soak 620s | **完成** | `soak_dual` PASS（A 620.457s / B 620.265s vs span 620.006s；包数 18610/18603；decode 干净；无残留） |
| P1-2 两路不同应用音源 FFT | **完成** | `multi_route_audio_dual` PASS：A 非目标 −52.71dB、B −44.07dB（≤−40dB） |
| P1-3 纯录音进恢复状态机 | **完成** | `start_audio_only` 建真实会话 + manifest（route.rs:459–501）；强杀→重扫→恢复测试 PASS（修复后 WAV 4.181s vs 录制 4.001s） |
| P1-4 正常收尾安全提交 | **完成** | 临时成品→ffprobe 验证→`safe_commit`（route.rs:1277/1291–1352）；同秒同目标并发两路预留名不同；冲突不覆盖、无 `.tmp`、段保留 |
| P2 编码器设置生效 | **完成** | Settings→`encoder_override`（commands.rs:1012）→`start_routes_with_encoder`（route.rs:330）；非法名 → `EncoderUnavailable`（无回退）；`h264_mf` 正路径 PASS |
| P2 托盘开始/停止 | **完成（源码级）** | 菜单项/事件（lib.rs:108–130）；开始=最近成功配置重放（commands.rs:811），无目标→原生 MessageBox 明确提示（lib.rs:72/121）；停止=`stop_all_impl`（commands.rs:786） |
| P1-1 区域框选后端 | **完成（源码级 + L1）** | `get_overlay_geometry`/`open_region_overlay`/`close_region_overlay`（region.rs:81/89/132）；物理并集单测 PASS；与 `ui/region.js` 调用契约逐字段一致 |

## 1. P1-2：双路音频与长时同步（REQ-25/26 接线）

### 1.1 生产接线（实际调用方）

| 调用点 | 位置 | 作用 |
|---|---|---|
| `Timeline::new(qpc_frequency(), qpc_now_ticks(), current_unix_ms())` | `crates/sr-pipeline/src/route.rs:1006` | 会话公共 T0；音频漂移测量的墙钟兜底 |
| `DriftController::new(DriftCfg{40,20,500})` | `route.rs:1038` | 每音轨独立滞回决策（spec 锚点） |
| `DriftResampler::new(2)` | `route.rs:1043` | 变比重采样器（样本域，见 1.2） |
| `drift.on_measurement(offset_ms)` | `route.rs:1096` | 节流 200ms：`offset = (视频 CFR 槽数 / fps − 已编排音频帧) / 48`（ms） |
| `resampler.set_rate_ppm(ppm)` | `route.rs:1099` | 把决策速率应用到写入管道的样本流（补齐静音与载荷一起缩放） |
| `SlotSender.frames`（视频槽计数） | `route.rs:214–264` | 漂移参照 = 实际送出的视频帧时间轴（与音频/编码同一会话） |

测量信号：`offset_ms > 0` 表示音频内容落后视频时间轴 → 控制器给正 ppm →
`step = 1 + ppm/1e6` → 输出帧数减少（追赶）；负偏差反向。补齐帧与载荷一起进入重采样，
避免补齐把校正抵消。视频尚未出帧时用 `Timeline` 的相对 100ns 作兜底参照。

### 1.2 变比重采样实现（决策说明）

新增 `sr_audio::resample::DriftResampler`（`crates/sr-audio/src/resample.rs:87`；
`process` :108，`set_rate_ppm` :103）：线性插值、跨块保留 1 帧历史、±1000ppm 输入限幅。

**为何不新增 rubato 依赖**：本任务要求「变比重采样路径实际接入」；spec D7 将 rubato 列为
候选实现。漂移限幅 500ppm（0.05% 采样间隔）下线性插值误差远低于可闻阈值，且避免为一个
极小比率引入 FFT 分块延迟与新外部包（AGENTS 依赖纪律）。接口与 rubato 的「set-rate +
process」同形，后续替换实现无需改 `route.rs` 调用点。若验收强制 rubato，可在
`DriftResampler` 内部替换而不动管线。

L1 证据（`cargo test -p sr-audio --test drift_resampler`，4 passed）：
- 0ppm 分块与整块逐样本一致（仅尾部 1 帧延迟）；
- +500ppm 输出 47,976±2 帧 / −500ppm 48,024±2 帧（1s 输入）；
- 分块/整块在 300ppm 下逐样本一致（max Δ<1e-5），无 NaN；
- 超限输入夹到 1000ppm。

### 1.3 新增测试与结果

**(a) `tests/sr-itests/tests/sync_dual.rs` — 双路 × 610s，每路不同应用音频**

源设计：两个子进程（同一测试二进制 `it_sync_dual_child_mode`）各自 WASAPI 渲染 1Hz 短音
（440/880Hz）并绘制同钟驱动的 2s 秒表条窗口；父进程逐路 `App{pid}` + `Window{hwnd}` 采集。
逐路断言与 `it_sync_content_10min` 同口径（150ms / 100ms / 0.20ms/s / 边界≥100 / 短音≥580 /
样本≥1000 / 时长±3s / decode 干净）；`SR_M11_DUAL_SECS` 可缩短调试（默认 610）。

```
SR_M11_DUAL_SECS=60 cargo test -p sr-itests --test sync_dual it_sync_dual_routes_content_10min -- --ignored --test-threads=1
→ 1 passed（66.01s）：A max 57.7ms / slope -0.0022；B max 66.6ms / slope 0.0349

cargo test -p sr-itests --test sync_dual it_sync_dual_routes_content_10min -- --ignored --test-threads=1
→ 1 passed（638.44s）：
  span=610.175s
  route-A(440Hz): dur=610.621s samples=2318 beeps=611 mean=45.0 max=97.8 head=25.6 tail=59.3
                  slope=0.0796ms/s boundary n=449 max=97.8ms
  route-B(880Hz): dur=610.408s samples=2317 beeps=610 mean=41.3 max=97.1 head=21.4 tail=55.8
                  slope=0.0807ms/s boundary n=446 max=97.1ms
```
两路均 1v+1a、1920x1080@30、aac48k stereo、decode 零错误输出。

**(b) `tests/sr-itests/tests/soak_dual.rs` — 2 路均带音频 × 620s**

```
SR_M11_SOAK_DUAL_SECS=60 ... → 1 passed（65.91s）
cargo test -p sr-itests --test soak_dual -- --ignored --test-threads=1
→ 1 passed（662.61s）：span=620.006s
  路A dur=620.457s packets=18610 v=h264,1920,1080 a=aac,48000,2
  路B dur=620.265s packets=18603 v=h264,1920,1080 a=aac,48000,2
  decode 干净；收尾后无 ffmpeg sidecar 残留
```

**(c) `tests/sr-itests/tests/multi_route_audio_dual.rs` — 两路不同应用音源 FFT**

```
cargo test -p sr-itests --test multi_route_audio_dual -- --ignored --test-threads=1
→ 1 passed（11.17s）：
  route-A(440): 目标幅度=372.9 非目标相对=-52.71dB
  route-B(880): 目标幅度=355.0 非目标相对=-44.07dB
```

### 1.4 既有同步用例回归

```
cargo test -p sr-itests --test sync it_sync_content_10min -- --ignored --test-threads=1
→ 1 passed（631.99s）：n=2313 max|Δ|=95.2ms 首末差 32.5ms 斜率 0.0794ms/s（decode 干净）
```
漂移接线接入后既有 610s 单路同步仍通过（与 M10b 修复后同量级）。

## 2. P1-3：纯录音进入恢复状态机

| 变更 | 位置 |
|---|---|
| 会话目录 + manifest（`streams=["wav"]`，state=recording） | `route.rs:459–501`（create_session_dir:472，save_manifest_atomic:494） |
| WAV 先写会话目录（崩溃可就地修复） | `route.rs:519`（`next_available(&session_dir, …)`） |
| 正常停止：封口 → safe_commit 到输出根 → 清单 state=final | `route.rs:591` + `commit_audio_only`（602–626，safe_commit:621） |
| recovery 扫描/修复（既有路径） | `recovery.rs:549`（scan_sessions）、`:643–646`（wav 路由 repair_wavs_in） |

新增测试 `tests/sr-itests/tests/m11_audio_only_recovery.rs`（父进程实际开录子进程 → 强杀 →
`recovery::scan_sessions` 重扫 → `finalize_session` 修复）：

```
cargo test -p sr-itests --test m11_audio_only_recovery -- --ignored --test-threads=1
→ 2 passed（5.64s；child mode 1 + 主用例 1）
  子进程就绪 ok 90468
  重扫发现 …/out/.screenrec_tmp/1792217176952_0（wav=…audio0.wav，active=false）
  修复后 WAV 时长 4.181s（录制段 4.001s）
```
既有 `audio_m6` 正常停止路径同时回归通过（3 passed；见 §7），证明正常停止 WAV 仍位于输出
目录内（`commit_audio_only` 提交到输出根）。

## 3. P1-4：正常收尾安全提交

| 变更 | 位置 |
|---|---|
| 最终名启动时分配并**进程内预留**（同秒同目标并发唯一） | `route.rs:739`（reserve_final_path）、`:821`；`RouteHandle::reserved_final_path` `:201` |
| 启动失败释放预留 / 成功移交收尾 | `route.rs:759–765`（FinalReservation）、`:823`、`:1141`（mem::forget） |
| 收尾：临时成品（会话目录）→ ffprobe 验证 → `safe_commit` | `route.rs:1277`（守卫）、`1291–1345`（tmp 路径）、`1348`（safe_commit） |
| 失败清理临时成品、保留段 | `route.rs:1324`、`1348–1350` |

新增测试 `tests/sr-itests/tests/m11_route_commit.rs`：

```
cargo test -p sr-itests --test m11_route_commit -- --ignored --test-threads=1
→ 2 passed（9.43s）
  [并发] 预留：…20261017_140543_Monitor 0.mkv / …_1.mkv
         成品[0] 3.833s；成品[1] 3.866s；无覆盖、无 .tmp 残留
  [冲突] 占用最终名 → FinalizeFailed；既有文件 "EXTERNAL-SENTINEL" 原样保留；
         无半成品；段保留（segments 非空）
```
说明：冲突用例模拟「收尾提交阶段中断/外部占用」——safe_commit 的 exists 检查先于
verify/rename，因此不会覆盖既有文件，也不会留下半成品；真正的进程级强杀恢复由
`recovery` 用例覆盖。

## 4. P2：编码器设置生效

| 链路 | 位置 |
|---|---|
| Settings.encoder（既有字段） | `src-tauri/src/settings.rs`（EncoderSetting） |
| 线格式：`"auto"` / `"h264_mf"`（兼容旧 `{"named":...}`） | `settings.rs:53`（Serialize）、`:62`（Deserialize） |
| 设置 → 管线覆盖（Auto→None；Named→Some） | `src-tauri/src/commands.rs:1012`（encoder_override），`:703`（start_routes_impl 取当前设置） |
| 管线：显式编码器实际初始化探测，失败即 `EncoderUnavailable` | `crates/sr-pipeline/src/route.rs:330`（start_routes_with_encoder）、`:656–665` |
| 探测实现 | `crates/sr-pipeline/src/sidecar.rs:132`（probe_named_encoder） |

新增测试 `tests/sr-itests/tests/m11_encoder_setting.rs`：

```
cargo test -p sr-itests --test m11_encoder_setting -- --ignored --test-threads=1
→ 1 passed（4.69s）
  [encoder] 非法名 → EncoderUnavailable（无回退）
  [encoder] h264_mf 显式设置 → 成品 …Monitor 0.mkv（可解码）
```

L1：
- `encoder_setting_maps_to_pipeline_override`（commands.rs:1328）通过；
- `encoder_setting_accepts_ui_string_and_legacy_tagged`（settings.rs:280）通过。

**线格式兼容（实测发现并修复）**：UI 发送 `encoder: "auto"|"h264_mf"`（`ui/app.js:944`），
而原 `EncoderSetting` 外部标签序列化为 `{"named":"..."}`，`set_settings` 会解析失败。现改为
纯字符串线格式并兼容旧形态；既有 `serde_full_round_trip` / `serde_partial_file_fills_defaults`
仍通过。

## 5. P2：托盘开始/停止

| 变更 | 位置 |
|---|---|
| 菜单项「显示/开始/停止/退出」 | `src-tauri/src/lib.rs:108–111` |
| 「开始」= 重放最近一次成功配置；无配置/已在录制 → 明确错误 | `commands.rs:811`（start_from_tray）、`:439–456`（remember/last/has_active） |
| 「停止」= `stop_all_impl`（与命令同一语义） | `commands.rs:786`；托盘事件 `lib.rs:128–130` |
| 无选中目标提示（原生 MessageBox，不依赖前端状态） | `lib.rs:72`（tray_notice）、`:119–123` |
| 启动放后台线程，避免同步握手阻塞托盘事件循环 | `lib.rs:119–123` |
| `start_routes` 命令与托盘共用实现 | `commands.rs:696/700` |

L1：`tray_start_without_selection_is_explicitly_empty`（commands.rs:1337）通过，验证
「无最近配置 → 无选中目标」的空态语义。

## 6. P1-1：区域框选后端（契约固定）

| 命令 | 位置 | 说明 |
|---|---|---|
| `get_overlay_geometry` | `src-tauri/src/region.rs:81` | 虚拟桌面**物理**并集（sr-enum DEVMODE 物理边界）+ overlay scale（已存在窗口 scale_factor，否则主屏；无设备 1.0） |
| `open_region_overlay` | `region.rs:89` | 复用或创建 label=`region-overlay`、`region.html`、无边框/置顶/跳任务栏/透明/覆盖虚拟桌面；创建后按物理像素 set_position/set_size |
| `close_region_overlay` | `region.rs:132` | 幂等关闭 |
| 注册 | `src-tauri/src/lib.rs:167–169` | invoke_handler |

坐标契约实现：`physical = origin + client * scale`；`take_screenshot(Region{rect})` 沿用
`RectIn`（物理像素）→ `ScreenRect`（`src-tauri/src/commands.rs:837` 调用、`:1113–1134`
`shot_target` 映射），不做逻辑坐标换算。

与前端（并行 lane 已产出）核对：`ui/region-geometry.js` 的 `clientToPhysical` 使用
`origin_x + cx*scale`、字段名与命令一致；`ui/region.js` 调用 `get_overlay_geometry` /
`take_screenshot({request:{target:{kind:'region',rect}}})` / `close_region_overlay`，与后端
签名逐项一致。

L1：`union_bounds_handles_negative_origin_and_mixed_sizes`（结果 `(-1280,0,3200,1124)`）、
`union_bounds_empty_is_none` 通过。GUI 打开/关闭 overlay 属人工验证项（见 §8）。

## 7. 验证汇总（实际执行）

| # | 命令 | 结果 |
|---|---|---|
| 1 | `cargo test --workspace` | **49 binaries：37 passed / 0 failed / 42 ignored**；唯一 warning 为既有 `probe.rs: unused import EncoderProbe` |
| 2 | `cargo check --workspace` | **0 warning / 0 error**（Finished） |
| 3 | `cargo test -p sr-audio --test drift_resampler` | 4 passed |
| 4 | `cargo test -p sr-itests --test sync_dual it_sync_dual_routes_content_10min -- --ignored` | **PASS 610s 双路**（见 §1.3） |
| 5 | `cargo test -p sr-itests --test soak_dual -- --ignored` | **PASS 620s 双路带音频**（见 §1.3） |
| 6 | `cargo test -p sr-itests --test multi_route_audio_dual -- --ignored` | 1 passed（−52.7/−44.1dB） |
| 7 | `cargo test -p sr-itests --test m11_audio_only_recovery -- --ignored` | 2 passed |
| 8 | `cargo test -p sr-itests --test m11_route_commit -- --ignored` | 2 passed |
| 9 | `cargo test -p sr-itests --test m11_encoder_setting -- --ignored` | 1 passed |
| 10 | 既有 `sync it_sync_content_10min`（610s） | 1 passed（max 95.2ms / slope 0.0794） |
| 11 | 既有 `route` | 5 passed（60.84s） |
| 12 | 既有 `multi_route` | 4 passed（37.85s） |
| 13 | 既有 `audio_isolation` | 4 passed（46.88s） |
| 14 | 既有 `audio_m6` | **3 passed / 1 BLOCKED**（本机无麦克风设备；环境） |
| 15 | 既有 `merge` | 1 passed（0.87s） |
| 16 | 既有 `recovery` | 2 passed（47.34s） |
| 17 | 既有 `probe` | 1 passed（1.50s） |
| 18 | 既有 `shot` | 2 passed / **2 BLOCKED**（本机无负坐标显示器；遮挡前置条件被其它置顶窗口占用；环境，非本轮改动） |
| 19 | 既有 `sync it_window_resize_canvas_stable` | 1 passed（11.81s） |
| 20 | 既有 `soak`（2 路 620s 纯视频） | 1 passed（631.44s；A 620.109→620.066s、B 621.309→621.300s） |
| 21 | GUI 启动冒烟：`target\debug\screen-rec.exe`（6s 后强杀） | 进程存活、stderr 为空 → 托盘 4 项菜单（显示/开始/停止/退出）与命令注册构建成功（点击交互未自动化，见 §8） |

新增 **L2** 测试均 `#[ignore]` + 文件内 `GATE` 串行 + 环境缺失 `blocked()`（fail，不静默
skip），默认 `cargo test` 不执行；新增**纯逻辑 L1**（`drift_resampler` 4 项 + 源码内 5 项）
无环境依赖，随默认 `cargo test` 运行（L1 约定）。既有测试文件零修改。

## 8. 未决 / 限制

1. **GUI-only 项**：托盘「开始/停止」与区域 overlay 的窗口行为无法在 L1/L2 自动断言
   （需真实 Tauri GUI 交互）。已完成源码接线 + 命令注册 + L1 几何/空态测试；建议 L3
   人工冒烟（托盘开始→录制→停止、区域框选→截图落盘）。
2. `audio_m6::it_mic_capture_present_or_skip` 本机 BLOCKED（`list_capture_devices` 为空）；
   `shot` 的负坐标/遮挡子用例环境 BLOCKED（历史一致）。
3. P1-2 采用线性插值重采样而非 rubato（§1.2）；若验收强制 rubato，可仅在
   `DriftResampler` 内部替换，不动调用点。
4. P1-4「收尾中断」以提交阶段冲突（最终名被外部占用）模拟验证：不覆盖、无半成品、段保留；
   进程级强杀恢复由既有 `recovery` 用例覆盖（本轮回归通过）。
5. 纯录音正常停止后 WAV 提交到输出根、`manifest.final_path` 记首个 WAV；崩溃恢复就地
   修复会话目录 WAV（不自动提交）。若要求崩溃恢复也提交到输出根，需另行扩展恢复事务。
6. 漂移校正按音轨独立、参照同一视频槽计数；双路单音轨 610s 已验证，多音轨×长时组合
   未单独做内容级测试（既有多音轨用例回归通过）。
7. 并行 lane 修改了 `specs/001-screen-recorder.md`、`README.md`、`ui/*`、
   `docs/milestones/m11-b-report.md` 与 `Cargo.lock`（非本任务改动；本任务未触碰
   specs/docs/spikes）。
8. 新增 `src-tauri` 直接依赖 `windows`（仅 MessageBox；版本 0.62 已在 workspace/lock，
   非新增外部包）。

