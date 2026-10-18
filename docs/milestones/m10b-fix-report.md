# M10b 修复报告：concat 段边界时间轴空洞（P0，阻断 AC-9）

- 日期: 2026-10-17
- 角色: **executor-backup**（实现 + 验证）；编排者独立复核
- 依据: `docs/milestones/m10b-sync-report.md` §4.2–§4.5；specs/001-screen-recorder.md REQ-21/25/27
- 约束: 未 git 提交；未改 `specs/`、`docs/spikes/`、`sr-merge`；除下述 sync 用例的调试开关外未改测试

## 0. 结论

| 项 | 修复前（m10b §4 实测） | 修复后（本轮实测） |
|---|---|---|
| 610s 成片时长 vs 录制段 | 618.636s vs 610.164s（**+8.472s / +1.39%**） | **610.344s vs 610.159s（+0.185s / +0.03%）** |
| 段边界 PTS 空洞（>30ms） | 视频 122 个 / 音频 122 个（92–108ms，首个 203ms） | **视频 0 个；音轨边界残差 ≤2ms（122 边界累计 +9.6ms）** |
| 成片视频帧间隔 | 边界处 32–170ms 台阶 | **仅 33/34ms（严格 CFR）** |
| 内容级 max\|offset\| / 斜率 | 15s 复现 +250–314ms / +25–29ms/s | **610s：98.5ms / 0.0780ms/s（通过）** |
| `it_sync_content_10min`（610s） | FAIL（时长断言） | **PASS** |

修复机制：**收尾 ffconcat 以视频轨为时间轴锚点逐段写 `duration`**，使下一段视频首帧恰好
接续上一段末帧的一个帧间隔；段首音频前导不再被计入成片时间轴（详见 §2）。

## 1. 根因（实测复核，非推断）

m10b §4.5 的结论方向正确，本轮补充了精确定位：

1. `-reset_timestamps 1` 让每个新段的时间零点由**段内最早包**决定。音频包通常领先视频
   关键帧 60–110ms 被写入新段（muxer 交织队列），因此每段都带一个**段首前导**：
   音轨从 0 开始，视频首帧从 0.059–0.100s 开始；段容器时长 = 前导 + 视频内容。
2. concat demuxer 在只写 `file` 时按**段容器时长**推进下一段起点，于是每段的段首前导
   变成成片时间轴上的空洞（视频与音轨同时发生，且累计）。
3. 实测复核（保留的 610s 原始输出，123 段 / 122 边界）：
   - 段容器时长 6.100/4.070/… 而视频内容恰为 6.000/4.000（每段多 59–100ms）；
   - 天真的 concat（无 `duration`）：视频/音轨各 **122 个 >30ms 间隙**，最大 170ms，
     合计 **+8269ms / +8299ms**，容器 **618.636s**。
4. 另一关键细节（首版修复踩中）：concat 的对齐零点取**段内最小包 PTS**，不是文件顺序
   首包。真机段常出现「视频包写在文件前面、音频包 PTS 更早」；按文件首包计算会引入
   21–91ms 的段间重叠（实测 `-f null` 报非单调 DTS）。修复必须以 min PTS 计算。

## 2. 修法

### 2.1 时间轴规则（生产代码）

逐段 ffprobe 读取：段内最小 PTS `origin`、视频首/末包 PTS、视频包数。对每个段边界写：

```
duration_k = (video_last_k − origin_k) + frame_k − (video_first_{k+1} − origin_{k+1})
frame_k    = (video_last_k − video_first_k) / (video_packets_k − 1)
```

即：在对齐后的时间轴上，让下一段视频首帧恰好等于上一段末帧 + 一个帧间隔。因 concat
按 min PTS 对齐，段首音频前导被吸收进 `duration`，不再产生空洞；音轨只承担
`duration_k^video − duration_k^audio` 的段边界残差（实测 ≤2ms、122 边界累计 +9.6ms、
单调不累积）。纯视频路由（无音轨）**不写 `duration`**，保持容器时长语义（M10 soak 无
累积缺陷）。

### 2.2 为什么不是「音轨内容跨度」

先实现了「`duration = 音轨末包结束 − min PTS`」（等价于方向②的音频侧对齐）：610s 重拼
后视频/音轨间隙均 ≤1ms、内容级 max 62.1ms / 斜率 -0.0021ms/s，但视频在边界出现 32ms
台阶；610s `-f null` 的 1/30 时间基换算把相邻帧压到同一刻度，报 **241 条非单调 DTS**
（`decode_clean` 要求 stderr 为空）→ 被否。视频锚定后成片视频严格 CFR，同时满足
「时空洞=0」与「整文件可解码无错误输出」。

### 2.3 代码变更

| 文件 | 变更 |
|---|---|
| `crates/sr-pipeline/src/finalize.rs`（新增） | `write_segment_list`（ffconcat + 校准 `duration`）、`SegmentTiming`/`duration_to`、ffprobe 包扫描与解析；3 个纯函数单测 |
| `crates/sr-pipeline/src/lib.rs` | 原 `finalize` TODO 占位 → `pub mod finalize;` |
| `crates/sr-pipeline/src/sidecar.rs` | 新增 `ffprobe_exe()`（与锁定 ffmpeg 同目录） |
| `crates/sr-pipeline/src/route.rs` | `finalize()` 改用共享清单构建器；按路由有无音轨决定是否校准 |
| `crates/sr-pipeline/src/recovery.rs` | 崩溃收尾 `finalize_session` 复用同一清单构建器（按 `route.streams` 判断音轨）；`ffprobe_path` 改为委托 `sidecar::ffprobe_exe()` |
| `tests/sr-itests/tests/sync.rs` | 仅加调试开关：`SR_SYNC_RECORD_SECS` 覆盖录制时长（默认 **610** 不变），覆盖率阈值（短音/样本/边界样本数）按同一比例缩放，610s 时与冻结值逐项一致（580/1000/100）。断言阈值 150ms、100ms、0.20ms/s、±3s **未改** |

未新增依赖；`sr-merge` 未动。

## 3. 验证（全串行，实际执行）

工具链 `%USERPROFILE%\.cargo\bin`；L2 运行环境 `SESSIONNAME=Console`（交互式 console
会话；当前 shell 未继承该变量，运行时显式设置，非测试改动）。

### 3.1 快速 15s 变体

```
$env:SR_SYNC_RECORD_SECS="15"
cargo test -p sr-itests --test sync it_sync_content_10min -- --ignored --nocapture
```

通过（`m10b-fix-quick15d.log`）：

| 指标 | 值 | 门槛 |
|---|---|---|
| span / 成片 / 收尾 | 15.031s / 通过时长断言 / 0.45s | span ±3s |
| 内容时钟样本 / 短音 | 42 / 16 | ≥25 / ≥14（按 15/610 缩放） |
| max\|offset\| | 52.4ms | ≤150ms |
| 首 20% − 末 20% | 1.4ms | ≤100ms |
| 斜率 | -0.0814ms/s | ≤0.20ms/s |
| 边界样本 / max | 7 / 52.4ms | ≥2 / ≤150ms |
| 整文件解码 | 无错误输出 | 必须 |

**说明（重要）**：15s 只有 3 个边界、42 个样本，斜率估计噪声约 ±0.5ms/s；本代码共跑 4
次，1 次通过（-0.081ms/s），3 次仅斜率项超限（>0.20ms/s，首次失败实测 0.251ms/s；
其余各项均通过）。逐样本 std≈16–17ms 来自 64px 秒表条量化与 5ms 短音 onset；这是短窗
统计功效问题，不是空洞回归（610s 完整用例以 2315 样本给出 0.078ms/s，见 §3.2）。

### 3.2 完整 610s（验收门）

```
cargo test -p sr-itests --test sync it_sync_content_10min -- --ignored --nocapture
```

**PASS**（632.57s，含 610s 录制与 ~12s 分析+生成；`m10b-fix-sync610b.log`）：

| 断言 | 实测 | 门槛 |
|---|---|---|
| span / 成片 | 610.159s / **610.344s（+0.185s）** | ±3s |
| max\|offset\| | **98.5ms** | ≤150ms |
| 末 20% − 首 20% | **32.6ms** | ≤100ms |
| 斜率 | **0.0780ms/s** | ≤0.20ms/s |
| concat 边界样本 / max | 449 / **98.5ms** | ≥100 / ≤150ms |
| 短音 / 内容样本 | 611 / 2315 | ≥580 / ≥1000 |
| 视频/音轨/画幅/帧率 | 1v+1a、h264 1920x1080@30、aac 48k 立体声 | 形状断言 |
| 整文件解码 | **无错误输出** | 必须 |
| 收尾耗时（123 段） | 4.54s（含逐段 ffprobe ≈3.6s；修复前 0.92s） | 超时 180s |

逐窗口（60s）均值：0–240s 为 +20–23ms，240–610s 为 +49–55ms（一次性 ~27ms 台阶，之后
无累积；斜率 0.078ms/s，远低于门槛）。该台阶出现在源端内容时钟/采集链路，本轮收尾
时间轴无对应跳变（音频边界残差累计仅 9.6ms），未纳入本次修复范围（见 §5）。

### 3.3 修复前基线复核（用保留的 610s 原始输出重拼）

| 变体 | 成片时长 | >30ms 边界间隙 | 最大间隙 | 合计 |
|---|---|---|---|---|
| 无 `duration`（修复前现状） | 618.636s | 视频 122 / 音频 122 | 170ms | +8269 / +8299ms |
| 音轨内容跨度（否决） | 610.337s | 0 / 0 | ≤1ms | — |
| **视频锚定（采用）** | **610.301s** | **0** | 视频 33/34ms 仅 CFR；音频 ≤2ms | 音频 +9.6ms |

视频锚定后 `-f null` 整文件解码 0 条告警；视频 PTS 增量仅 33/34ms。音轨内容跨度变体
在 610s 下产生 241 条非单调 DTS 告警（故否决，§2.2）。

**通过用例成品的直接复核**（`sr_m10b_sync_sync-out_53268\…mkv`，ffprobe 包扫描）：

| 指标 | 实测 |
|---|---|
| 视频包 / 首帧 / 末帧结束 | 18306 / 0.021s / 610.262s |
| 视频 PTS 增量 | 仅 33/34ms（12162/6143） |
| 视频边界间隙 >30ms | **0**（最大 1.0ms） |
| 音轨边界间隙 >30ms | **0**（最大 2.0ms，累计 +9.576ms） |
| 容器时长 | 610.344s |

### 3.4 回归（全串行）

| # | 命令 | 结果 | 日志 |
|---|---|---|---|
| 1 | `cargo test --workspace` | **28 passed / 0 failed / 33 ignored**（既有 25 + 新增 finalize 单测 3） | `m10b-fix-l1.log` |
| 2 | `cargo test -p sr-itests --test route -- --ignored --test-threads=1` | **5 passed（60.82s）** | `m10b-fix-route.log` |
| 3 | `cargo test -p sr-itests --test multi_route -- --ignored --test-threads=1` | **4 passed（37.48s）** | `m10b-fix-multi_route.log` |
| 4 | `cargo test -p sr-itests --test audio_isolation -- --ignored --test-threads=1` | **4 passed（46.61s）** | `m10b-fix-audio_isolation.log` |
| 5 | `cargo test -p sr-itests --test audio_m6 -- --ignored --test-threads=1` | **3 passed / 1 BLOCKED**：`it_mic_capture_present_or_skip` 本机无麦克风设备（环境 BLOCKED，非回归） | `m10b-fix-audio_m6.log` |
| 6 | `cargo test -p sr-itests --test merge -- --ignored --test-threads=1` | **1 passed（0.86s）** | `m10b-fix-merge.log` |
| 7 | `cargo test -p sr-itests --test recovery -- --ignored --test-threads=1` | **2 passed（47.18s）** | `m10b-fix-recovery.log` |
| 8 | `cargo test -p sr-itests --test probe -- --ignored --test-threads=1` | **1 passed（1.35s）** | `m10b-fix-probe.log` |
| 9 | `cargo test -p sr-itests --test sync it_window_resize_canvas_stable -- --ignored` | **1 passed（11.79s）** | `m10b-fix-resize.log` |
| 10 | `cargo test -p sr-itests --test soak -- --ignored --test-threads=1`（620s 纯视频 ×2） | **1 passed（631.26s）**：A 620.124s→620.333s（+0.21s）、B 620.845s→620.733s（-0.11s），无累积、无孤儿 | `m10b-fix-soak.log` |

- `cargo check --workspace`：0 warning。
- 唯一既有 warning：`tests/sr-itests/tests/probe.rs` 的 `unused import: EncoderProbe`（非本轮引入）。
- 纯视频 soak 路径未写 `duration`（`has_audio == false` 时跳过 ffprobe），时延与修复前一致。

## 4. 证据分级

- **实际执行（observed）**：§3.1–§3.4 全部命令、计时与计数；§3.2 的成片时长/内容统计/
  decode 输出；§3.3 的三个重拼变体（基于 123 个真实段文件）；soak 双路数据。
- **推断（inferred）**：§1 的 muxer 交织队列机制（由段内 PTS 分布与 concat 对齐行为反推）；
  §3.2 中 240s 台阶的源端归属（收尾侧残差不足以解释）。
- **文档记载（documented）**：REQ-21/25/27 阈值引自 spec；修复前基线数字引自
  m10b-sync-report §4（本轮用保留输出复算一致）。

## 5. 遗留 / 未决

1. **15s 快速变体斜率噪声**：仅 3 个边界时 0.20ms/s 门槛统计功效不足（4 次中 1 次通过，
   其余仅斜率超限 >0.20ms/s）。未改冻结阈值；快速复现建议用 ≥60s（`SR_SYNC_RECORD_SECS=60`）
   或直接以 610s 门为准。若需长期把 15s 作为 CI 快检，应单独评审该门限的统计口径。
2. `audio_m6::it_mic_capture_present_or_skip` 本机 BLOCKED（`list_capture_devices` 为空，
   与 S3 记录一致）；需有声学输入端点的主机复跑。
3. 观察但未修：`-force_key_frames expr:gte(t,n_forced*5)` 在本机链路未生效，实际关键帧
   仍由 `-g 2*fps` 驱动（段长 4/6s 交替）。不影响本缺陷（时间轴校准逐段测量，不依赖
   段长），但 REQ-21 的 5s 段长冻结建议另案核查编码器/参数组合。
4. 收尾时延：610s/123 段增加约 3.6s（每段一次 ffprobe 包扫描，实测 29ms/段）。如需压缩，
   可在编码期落盘段边界时间戳，或并行探测；本轮按最小改动未做。
5. 崩溃恢复路径已复用同一校准逻辑（recovery 2 用例通过），但未对「恢复产物」重复做
   10 分钟内容级测量（成本原因）。
