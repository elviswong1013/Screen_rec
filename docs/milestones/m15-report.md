# M15 报告：P1 采样率可选（REQ-1..8）

- 日期：2026-10-18
- 规格：`specs/002-recording-modes.md`（v3.1，plan-reviewer APPROVED + 用户 Gate-2 批准）
- 范围：`crates/sr-audio`、`crates/sr-pipeline`、`src-tauri`、`ui/`、
  `tests/sr-itests`、`tools/ui/`；规格 §8 M15 行状态。
- 约束遵守：未改 PRD 与 `specs/001-screen-recorder.md`；未执行任何 `git` 提交/推送/
  重置/暂存（仅只读 `git status/diff`，以及一次 `git checkout --` 恢复被编码误写的
  单个测试文件 `crates/sr-audio/tests/drift_resampler.rs`——工作树当时无用户改动）；
  未重建安装包；未触碰 `SR_DIAG_AV` 机制；未改 `docs/design/ui-spec.md`。

## 0. 结论

| 项 | 结果 | 证据 |
|---|---|---|
| REQ-1 设置项 | 四档 + 默认 48000；字段级容错只降级本字段；磁盘 / `set_settings` 同归一化；启动快照不随保存变更 | §2；`unit_settings_sample_rate_*`（6 条）；`m15-workspace-L1.log` |
| REQ-2 设置页 / 查询命令 | 四档下拉 + 原生率只读 + 升采样提示 + 进程环回标注；刷新=设置页打开+麦克风变更；real/mock 双接线；runner 五态全过 | §2；`tools/ui/settings-sample-rate.mjs`（全部 PASS）；L2 `it_endpoint_mix_rate_query` |
| REQ-3 输出率生效 | MKV AAC `-ar` 参数化；纯录音 WAV 写线程重采样；`base==1 && 0ppm` 恒等直通不变 | §3；`it_route_sample_rate_*`、`it_wav_sample_rate_*`、`unit_ffmpeg_args_audio_rate_parameterized` |
| REQ-4 验证口径 | 成品**全音轨** ffprobe 实际率 + 时长 + 440Hz FFT 检出（防改标签） | `m15-verify-route-sample-rate.log`、`m15-verify-wav-sample-rate.log` |
| REQ-5 回归 | 48k 基线全绿；sync_dual 新增 96k 变体（两者均跑满 610s）；audio_m6 麦克风正路径本机 BLOCKED（无采集设备，非通过） | §5 L2 表 |
| REQ-6 基础比率 | `new(channels,in_rate,out_rate)`；`ratio=base/(1+ppm/1e6)`；base≠1 的 0ppm 不直通；补齐/延迟/flush 覆盖 | `unit_resample_base_ratio_*`（8 条） |
| REQ-7 分源策略 | System/Mic 原生采样率（仍 f32/双声道），失败回退 48000+autoconvert；Process 固定 48000；同步捕获按实际率钉帧数 | `unit_capture_format_strategy_*`（6 条） |
| REQ-8 实际率与计数域 | `StreamMsg::Ready` 携带 `{rate,channels}`；路由按实际捕获率构造重采样；Pacer/device_frames 在捕获率域、漂移比较到秒域 | `unit_capture_format_strategy_ready_*`、`unit_drift_mixed_rates_no_false_correction` |
| L1 | `cargo test --workspace` exit 0；53 个测试二进制全部 `test result: ok` | `m15-workspace-L1.log` |
| L2 | 规格要求套件串行通过（除 audio_m6 麦克风正路径环境 BLOCKED） | §5；`m15-verify-*.log` |

## 1. 红证据（实现前）

全部 §4 测试先落盘（含新增文件 `crates/sr-audio/tests/base_ratio.rs`、
`tests/sr-itests/tests/sample_rate.rs`、`tools/ui/settings-sample-rate.mjs`），
再运行：

```text
$ cargo test --workspace
error[E0422]: cannot find struct, variant or union type `CaptureFormat` in this scope
error[E0532]: expected tuple struct or tuple variant, found unit variant `StreamMsg::Ready`
error[E0425]: cannot find function `capture_format_strategy` in this scope
error[E0425]: cannot find function `sync_target_frames` in this scope
EXIT=101
```

（`docs/milestones/m15-red-L1.log`。编译在首个 crate `sr-audio` 处终止；settings /
L2 测试引用的 `audio_sample_rate`、`start_routes_with_options_and_rate`、
`start_audio_only_with_options_and_rate`、`content_offset_ms` 等同样尚不存在。）

UI runner 红：

```text
FAIL  api.js real 接线 get_endpoint_mix_rate
FAIL  api.js mock 接线 getEndpointMixRate
FAIL  app.js 写入 audio_sample_rate
FAIL  app.js 调用 getEndpointMixRate
FAIL  runner 执行 — Error: 等待页面条件超时: !!document.querySelector('#set-sr')
EXIT=1
```

（`docs/milestones/m15-red-ui.log`。）

## 2. 实现摘要

### REQ-1 / 设置

- `src-tauri/src/settings.rs`
  - 新增 `pub audio_sample_rate: u32`；`#[serde(default = "default_sample_rate",
    deserialize_with = "deserialize_sample_rate")]`；`AUDIO_SAMPLE_RATES` 四档、
    具名 `default_sample_rate()=48000`、`Settings::default()` 同步。
  - 字段级容错：自定义 visitor 接受任意 JSON 值，负数字符串/null/溢出/浮点/对象/
    数组等只把本字段降级为默认；其余字段保留（修复整体重置）。
  - `normalize_sample_rate` / `Settings::normalize`：四档外数值 → 48000；
    `load_or_default` 在反序列化后归一化。
- `src-tauri/src/lib.rs`
  - 抽出 `persist_settings(dir, state, settings)`：`normalize → validate → save →
    更新内存`；`set_settings` 复用；测试验证 `set_settings` 入口归一化与
    `SettingsState::snapshot()` 返回的启动快照不随后续保存变更。
- `src-tauri/src/commands.rs`
  - `start_routes_impl` / `start_audio_only` 各读取一次设置快照并把
    `audio_sample_rate` 传入管线；托盘 `start_from_tray` 复用 `start_routes_impl`
    （调用时取新快照）。在录路由不再读取设置。

### REQ-2 / UI 与查询

- `crates/sr-audio/src/wasapi.rs`：新增 `query_endpoint_mix_rate(device_id) ->
  Option<EndpointMixRate { rate, kind, device_name }>`，专用 MTA 线程；`None=None`
  渲染端点 / `Some(id)` 采集设备；任何失败返回 `None`。
- `src-tauri/src/commands.rs` / `lib.rs`：`get_endpoint_mix_rate` DTO
  `{rate, kind, device_name}`（`Option`，失败为 null），`spawn_blocking` 执行并注册
  `invoke_handler`；与 `probe_capabilities` 完全隔离。
- `ui/api.js`：real 分支 `invoke('get_endpoint_mix_rate', { deviceId })` 原样透传
  （不进入 `normalizeProbe`）；mock 分支同形返回，并支持 runner 注入
  `native_rate` / `mic_rate` / `native_unknown` / `native_fail`。
- `ui/app.js`：设置页动态生成四档下拉 `#set-sr`、只读原生率说明
  `#set-sample-rate-note`；渲染端点/麦克风 = 设备混音率，进程环回固定标注
  「跟随引擎混音率（通常 48k）」；所选 > 已知原生率时提示
  「升采样（不提升真实音质）」；未知/查询失败只展示「未知」，**不参与**任何比率
  推导。刷新时机 = 设置页打开 + 麦克风设备变更（seq 防过期结果）。
- `tools/ui/settings-sample-rate.mjs`：内置静态服务器 + headless Chrome/CDP；覆盖
  `<`/`=`/`>`、未知、保存重载、查询失败、麦克风变更刷新、四档选项与 real/mock
  静态接线断言。

### REQ-3/6/7/8 / 音频管线

- `crates/sr-audio/src/resample.rs`
  - `DriftResampler::new(channels, in_rate, out_rate)`；`base=out/in`；
    `ratio=base/(1+ppm/1e6)`；ppm 仍限幅 ±1000。
  - `base != 1` 构造即进入 rubato（0ppm 不直通）；`base == 1 && 0ppm` 保持逐样本
    恒等直通。延迟记账/补齐/flush 沿用并覆盖转换路径。
- `crates/sr-audio/src/wasapi.rs`
  - 新增 `CaptureFormat { sample_rate, channels }`、
    `capture_format_strategy(target, queried)`（可注入查询结果）、
    `sync_target_frames(duration, rate)`。
  - `StreamMsg::Ready(CaptureFormat)`；`LoopbackStream` 保存并在
    `capture_format()` 暴露实际格式；同步捕获按实际率钉帧数并返回实际率。
  - System/Mic 在 MTA 线程内 `get_mixformat()` 查询原生采样率（仍请求 f32/双声道）；
    查询失败记录日志并回退 48000 + autoconvert；Process 不查询、固定 48000。
- `crates/sr-pipeline/src/sidecar.rs`：`AudioPipeSpec { pipe_path, sample_rate }`；
  `-ar` 使用该值。
- `crates/sr-pipeline/src/route.rs`
  - 新增 `start_routes_with_options_and_rate` /
    `start_audio_only_with_options_and_rate`；既有 48k API 委托默认 48000。
  - 视频音轨写线程按 `stream.capture_format()` 构造
    `DriftResampler::new(channels, capture_rate, out_rate)`；`device_frames` 在捕获
    率域；漂移比较 `content_offset_ms = (reference_secs − frames/capture_rate)×1000`。
  - 纯录音 WAV 写线程把 pad+payload 送入重采样器，停止/采集错误路径先 flush 再封口；
    WAV 头采样率 = 输出率。
- `crates/sr-audio/src/wav.rs`：文档同步为「输出率」；写入接口本已支持采样率参数。

### 测试更新（spec 批准）

- `tests/sr-itests/tests/audio_m6.rs`：`:549-559`、`:635-643`、`:733-742` 三处
  率域迁移为 `cap.sample_rate`（帧数/时长/裁剪/FFT）；默认 48k 断言域保留。
- `tests/sr-itests/tests/sync_dual.rs`：抽出 `run_dual(output_rate, expected_audio)`；
  48k 基线与新增 96k 变体（`aac,96000,2`）共用同一内容级阈值。
- `crates/sr-pipeline/tests/ffmpeg_args_snapshot.rs`：`AudioPipeSpec` 增加
  `sample_rate: 48_000`；新增 `unit_ffmpeg_args_audio_rate_parameterized`。

## 3. REQ → 测试映射

| REQ | 测试 | 层 | 结果 |
|---|---|---|---|
| REQ-1 | `unit_settings_sample_rate_defaults_and_four_rate_round_trip` / `_missing_field_keeps_other_fields` / `_bad_types_degrade_field_only` / `_disk_load_normalizes` / `_normalize_is_idempotent` / 跨 crate `unit_settings_sample_rate_set_settings_normalizes_and_snapshot_is_stable` | L1 | 6/6 |
| REQ-6 | `unit_resample_base_ratio_{96000_to_48000,48000_to_96000,48000_to_44100,96000_to_44100,zero_ppm_is_not_passthrough,ppm_adjusts_around_base_and_hold_recovers,chunked_equals_oneshot,identity_only_when_base_is_one}` | L1 | 8/8 |
| REQ-7 | `unit_capture_format_strategy_{system_uses_native_rate,mic_uses_native_rate,query_failure_falls_back_48k,process_never_queries,ready_carries_actual_rate_and_channels,sync_frames_use_capture_rate}` | L1 | 6/6 |
| REQ-8 | `unit_drift_mixed_rates_no_false_correction`；Ready 携带格式 / 实际率传递见上 | L1 | 1/1 |
| REQ-3/4 | `it_route_sample_rate_{44100,88200,96000}`；`it_wav_sample_rate_{44100,88200,96000}` | L2 | 3/3 + 3/3 |
| REQ-2 | `it_endpoint_mix_rate_query`；`tools/ui/settings-sample-rate.mjs` | L2 / UI | 1/1；runner 全 PASS |
| REQ-3 | `unit_ffmpeg_args_audio_rate_parameterized` | L1 | 1/1 |
| REQ-5 | 48k 回归清单 + sync_dual 96k 变体 | L2 | 见 §5（audio_m6 麦克风 BLOCKED） |

## 4. L1 结果

环境：Windows 11 25H2 build 26200.9711。

```text
$ cargo fmt --all -- --check        → exit 0        （m15-fmt.log）
$ cargo test --workspace            → exit 0；53 个测试二进制 test result: ok
                                                     （m15-workspace-L1.log）
$ node --check ui/app.js ui/api.js tools/ui/settings-sample-rate.mjs → OK
$ node tools/ui/settings-sample-rate.mjs → exit 0，全部 PASS（m15-ui-runner.log）
```

`m15-workspace-L1.log` 无 `FAILED` / `error[` / `failures:`；新增 L1 用例逐条为
`... ok`（见 §3）。

## 5. L2 结果（串行、`SESSIONNAME=Console`、独占机器）

**环境偏差记录（诚实标注）**：首次启动的 PowerShell keep-awake 助手使用了错误的无符号
参数（`0x80000003` 被解释为 `-2147483645`），每轮调用均抛 `UInt32` 转换错误，**实际
未生效**；整个 21 分钟 sync_dual 运行期间机器未休眠，L2 全部完成。修正后的调用
（`SetThreadExecutionState([uint32]0x80000003)`）已验证返回 2147483648 =
`ES_CONTINUOUS`，并在短套件复跑与 `sample_rate` 全套 7 条（72.92s）运行期间生效；
仅 `sync_dual` 两变体（21 分钟）为修正前完成——该期间机器未休眠，结果完整。若验收要求
其余套件也复跑于「助手生效」口径，可按 §5 命令复跑（最长为 sync_dual 约 21 分钟）；
本轮结果不受机器休眠影响。

命令口径：`cargo test -p sr-itests --test <suite> -- --ignored --nocapture
--test-threads=1`（`SR_DIAG_AV` 未设置）。

| 套件 | 结果 | 耗时 | 日志 |
|---|---|---|---|
| route（48k 基线） | **5 passed / 0 failed** | 60.86s | `m15-verify-route.log` |
| multi_route | **4 passed / 0 failed** | 38.65s | `m15-verify-multi_route.log` |
| audio_isolation | **4 passed / 0 failed** | 48.98s | `m15-verify-audio_isolation.log` |
| recovery | **2 passed / 0 failed** | 48.48s | `m15-verify-recovery.log` |
| probe | **1 passed / 0 failed** | 1.54s | `m15-verify-probe.log` |
| merge | **1 passed / 0 failed** | 0.94s | `m15-verify-merge.log` |
| m11_route_commit | **3 passed / 0 failed** | 13.88s | `m15-verify-m11_route_commit.log` |
| m11_audio_only_recovery | **2 passed / 0 failed** | 5.65s | `m15-verify-m11_audio_only_recovery.log` |
| multi_route_audio_dual | **1 passed / 0 failed** | 10.81s | `m15-verify-multi_route_audio_dual.log` |
| audio_m6 | **3 passed / 1 BLOCKED（麦克风正路径）** | 38.93s | `m15-verify-audio_m6.log` |
| sample_rate — WAV | **3 passed / 0 failed** | 27.67s | `m15-verify-wav-sample-rate.log` |
| sample_rate — MKV | **3 passed / 0 failed** | 45.96s | `m15-verify-route-sample-rate.log` |
| sample_rate — mix rate | **1 passed / 0 failed** | 0.03s | `m15-verify-mix-rate.log` |
| sample_rate — 全套 7 条（修正后 keep-awake 生效） | **7 passed / 0 failed** | 72.92s | `m15-verify-sample_rate-all.log` |
| sync_dual（48k + 96k，各 610s） | **2 passed / 0 failed** | 1283.09s | `m15-verify-sync_dual-48k.log`（含两变体） |

关键实测：

- **WAV**：44.1k → 6.201s（273478 帧 ÷ 44100）、FFT 440Hz 高于本底 143.5dB；
  88.2k → 6.201s、138.0dB；96k → 6.201s、151.4dB。ffprobe 实际率
  `pcm_s16le,{44100,88200,96000},2`。
- **MKV**：44.1k → `aac,44100,2`/10.473s、FFT 130.2dB；88.2k →
  `aac,88200,2`/10.485s、119.4dB；96k → `aac,96000,2`/10.476s、116.3dB。
  FFT 以声明率解码检出 440Hz（若仅改标签未重采样，音调会按比例偏移）。
- **sync_dual 48k**：route-A max 96.5ms / slope 0.0807ms/s；route-B max 89.8ms /
  slope 0.0779ms/s；`aac,48000,2`。
- **sync_dual 96k**：route-A max 95.3ms / slope 0.0780ms/s；route-B max 91.9ms /
  slope 0.0700ms/s；`aac,96000,2`。
- **mix rate**：`it_endpoint_mix_rate_query` 读到默认渲染端点
  `LS24A600N (NVIDIA High Definition Audio) 48000 Hz`；本机无采集设备，麦克风端点
  查询正路径与 audio_m6 麦克风用例一样记环境缺失（非通过）。
- **audio_m6**：设备环回 RMS 0.0645（440/880 均在、差约 1.3dB）；全音轨 3 轨分类
  通过；WAV 正常停止/崩溃修复路径通过；麦克风用例打印 `BLOCKED: 本机当前无麦克风
  设备` 并按 §4 失败（BLOCKED 不算通过）。

UI runner 覆盖（全部 PASS，`m15-ui-runner.log`）：`<`（原生 44100 → 升采样提示）、
`=`（48000 → 无提示）、`>`（96000 → 无提示）、未知（null → 「未知」且保存成功）、
保存重载（96000 持久化）、查询失败（「未知（查询失败）」且不影响下拉）、麦克风
设备变更刷新（96000 → 44100）、四档选项、real/mock 静态接线。

## 6. 未决 / 范围说明

1. **REQ-5 的「44.1k/96k 各补 isolation、recovery 子集」未单独新增套件**：本轮按
   编排者给出的显式验证清单执行（既有回归 + `it_route_sample_rate_*` /
   `it_wav_sample_rate_*` + sync_dual 96k 变体）。现有覆盖为：44.1k/88.2k/96k 的
   真实录制（WGC 视频 + 应用音轨）与纯录音 WAV、96k 双路内容同步，以及 48k 的
   isolation/recovery 全套。若 acceptor 要求按 §4 字面追加 44.1k/96k 的
   isolation/recovery 专项，需编排者确认（避免擅自扩大测试范围）。
   **已闭环（M15b，2026-10-18）**：编排者确认后按 §4 字面补齐，见 §7。
2. **audio_m6 麦克风正路径 BLOCKED**：本机 `list_capture_devices()` 为空（与既有
   基线一致）；按 §4 记 BLOCKED、不算通过，需有采集设备的机器复跑。
3. **真实 96k 捕获不可在本机验证**：本机端点混音率 48k；96k 输出路径由
   48k 捕获 → 96k 重采样 + L1 96k 捕获比率单测覆盖，真实 96k 捕获留 L3（与
   spec §6 风险一致）。**96k 输出测试 ≠ 验证 96k 捕获**。
4. **UI 查询命令的 real 分支仅静态接线 + L2 底层查询**：runner 在无 `__TAURI__` 的
   mock 分支驱动真实页面；`get_endpoint_mix_rate` 的 Tauri invoke 端到端未像 M14
   那样通过 WebView2 CDP 实调（命令注册由编译/L1 覆盖）。如需应用层证据可追加
   debug 构建 CDP 验证。
5. **工作树并发改动**：`docs/milestones/l3-signoff.md` 在本轮期间出现与本任务无关的
   修改（L3-2 状态更新），未触碰/未回退，交付提交时请编排者区分归属。
6. **日志被 `.gitignore` 的 `*.log` 忽略**：`docs/milestones/m15-*.log` 均已落盘，
   但默认不会进入 git；若验收要求提交日志，需编排者显式 `git add -f`。

## 7. M15b 增量（2026-10-18）：REQ-5 字面补齐 44.1k/96k isolation + recovery 子集

按 `specs/002-recording-modes.md` REQ-5「44.1k 与 96k 各补 isolation、recovery
子集」字面补齐；仅改测试，无产品代码变更。内容同步子集已由既有
`it_sync_dual_routes_content_10min_96k` 覆盖（`m15-verify-sync_dual-48k.log`，2/2），
本轮不重复。

- **isolation（`tests/sr-itests/tests/audio_isolation.rs`）**：既有窗口路由 +
  目标应用音轨 + 880Hz 干扰源的隔离用例参数化为 `multitrack_isolation_case(output_rate)`；
  48k 基线断言保持不变，新增具名变体
  `it_audio_multitrack_ffprobe_fft_44100`、`it_audio_multitrack_ffprobe_fft_96000`。
  断言：恰 1 视频 + 1 目标应用音轨、`aac,{44100|96000},2`、440Hz 在 / 880Hz
  ≤−40dB、时长≈录制段。
- **recovery（`tests/sr-itests/tests/recovery.rs`）**：新增合成崩溃残留率域子集
  `it_recovery_sample_rate_{44100,96000}`（2 段 + 截断尾段 → 目录重扫/封口 → 恢复
  收尾；`keep_segments=true` 以核对原段）。断言成品整文件可解码、音轨
  `aac,{rate},2`、时长≈封口段和、原段保留。
- **L2 结果**（Windows 11 25H2 build 26200.9711；`SESSIONNAME=Console`；串行
  `--test-threads=1`；运行期 `SetThreadExecutionState(0x80000003)` 保持唤醒；
  `SR_DIAG_AV` 未启用）：
  - `audio_isolation` **6 passed / 0 failed**（75.77s）：48k 基线 + 44.1k/96k
    isolation 变体 + 既有 capture/背压用例。日志
    `m15b-verify-audio-isolation.log`。
  - `recovery` 率域子集 **2 passed / 0 failed**（0.90s）：成品
    `aac,44100,2`（2.066s）、`aac,96000,2`（2.027s）。日志
    `m15b-verify-recovery-rates.log`。
- 未改 PRD / `specs/001-*` / `specs/002-*` 需求文本 / `SR_DIAG_AV`；未执行 `git`；
  未重建安装包；既有断言未削弱（仅参数化 + 新增变体）。

## 8. M15c/acc-7 增量（2026-10-18）：44.1k 内容同步变体 + 规格状态

- **规格状态**：`specs/002-recording-modes.md:3` 的 `Status: Draft` 改为
  `Status: Approved`（plan-reviewer APPROVED 与用户 Gate-2 批准均为 2026-10-18）。
  除该字段外未改任何需求/测试映射文本。
- **REQ-5 内容同步子集补齐**：`tests/sr-itests/tests/sync_dual.rs` 参数化既有
  `run_dual(output_rate, expected_audio)`，新增
  `it_sync_dual_routes_content_10min_44100`（`aac,44100,2`，非整数基础比率路径）；
  同步阈值/样本/短音/边界断言与 48k 基线完全一致，未放宽。
- **L2 实测（610s，串行）**：Windows 11 25H2 build 26200.9711；
  `SESSIONNAME=Console`；`--test-threads=1`；`SetThreadExecutionState(0x80000003)`
  返回 2147483648（保持唤醒）；独占机器；`SR_DIAG_AV` 未启用。
  - `it_sync_dual_routes_content_10min_44100` **1 passed / 0 failed**
    （639.00s，录制 span 610.025s）：
    - route-A(440Hz)：dur 610.400s、samples 14746、beeps 611、mean 45.2ms、
      max 88.3ms、head 26.9ms、tail 57.3ms、slope 0.0730ms/s、边界 n=2837 /
      max 85.8ms；
    - route-B(880Hz)：dur 610.284s、samples 14635、beeps 610、mean 35.8ms、
      max 83.8ms、head 16.6ms、tail 47.6ms、slope 0.0751ms/s、边界 n=2793 /
      max 83.8ms；
    - 两路音轨均 `aac,44100,2`，整文件可解码。日志
      `m15c-verify-sync_dual-44100.log`。
- **恢复清理前成品复核（acc-7 阻断 1）**：修复见 `m14-report.md` §9；同一轮证据：
  新增单元回归红→绿（`m15c-red-recovery.log` / `m15c-green-recovery-unit.log`）、
  L2 `recovery` **5/5**（含新回归，49.68s，`m15c-verify-recovery.log`）、
  workspace L1 exit 0（`m15c-workspace-L1.log`）、route 抽查 **1/1**
  （`m15c-verify-route.log`）。
- 未改 PRD / 其它 `specs/*` 内容 / `SR_DIAG_AV`；未执行 `git`；未重建安装包。
