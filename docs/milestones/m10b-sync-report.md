# M10b 缺口补齐报告（REQ-25 内容级同步 + REQ-13 resize）

- 日期: 2026-10-17
- 角色: **executor-backup 合并红/绿**（GLM 通道不可用；编排者将独立复核）
- 依据: specs/001-screen-recorder.md §2.3 REQ-13、§2.6 REQ-25、§4 测试映射、§8 M0 冻结；
  docs/milestones/m10-rehearsal-report.md §7-1/§7-2、§8-1/§8-2
- 约束: 未修改任何既有测试（仅新增 3 个测试/用例）；未改 `specs/`、`docs/spikes/`；未 git 提交

## 0. 结论摘要

| 用例 | 层 | 结果 | 说明 |
|---|---|---|---|
| `unit_epoch_unit_mapping` | L1 | **PASS** | 单位换算/T0 时间轴/负数/饱和/跨日 全断言通过 |
| `it_window_resize_canvas_stable` | L2 | **PASS** | 改尺寸/比例/最小化恢复后画布恒定、段参数一致且独立可播、全程非黑 |
| `it_sync_content_10min` | L2 | **FAIL（真实产品缺陷）** | 发现 **concat 段边界 PTS 空洞**：每 5s 段边界约 100ms、610s 累计 **+8.47s（1.39%）**，音画内容偏差随之漂移 ≈20–29ms/s —— REQ-21「拼接无跳变」/REQ-25「≤150ms 不累积」被违反（详见 §4） |
| 全量回归 | — | **全绿** | L1 25 passed；route/multi_route/audio_isolation/merge/recovery/probe 全通过（§5） |

> **交付判定**：3 个新用例中 **2 绿 + 1 红**。红不是测试缺陷或环境噪音，而是新用例命中的
> **真实 P0 级拼接缺陷**（有帧数/分段时长/PTS 间隙三重硬证据）。断言未削弱；修复该缺陷需改
> 生产编码/收尾管线并重跑全部 L2（超出本 lane 的最小改动边界），故如实上报。

## 1. 变更文件

| 文件 | 变更 |
|---|---|
| `crates/sr-pipeline/src/epoch.rs`（新增） | REQ-25 单位换算：`qpc_ticks_to_hns`/`hns_to_qpc_ticks`（round-half-away-from-zero、i128 中间量、饱和、非法频率→None）、`unix_ms_to_hns`/`hns_to_unix_ms`、`Timeline`（公共 T0：QPC ↔ 100ns ↔ epoch；负值/跨日）、`qpc_now_hns` |
| `crates/sr-pipeline/src/lib.rs` | `pub mod epoch` |
| `crates/sr-pipeline/src/route.rs` | `SessionManifest.start_qpc` 改用 `crate::epoch::qpc_now_hns()`；删除私有重复实现 `qpc_100ns`（行为等价，舍入差 ≤1 hns） |
| `crates/sr-pipeline/tests/epoch_mapping.rs`（新增） | L1 `unit_epoch_unit_mapping` |
| `tests/sr-itests/tests/sync.rs`（新增） | L2 `it_sync_content_10min` + `it_window_resize_canvas_stable`；测试内 WASAPI 渲染参考源 + 内容时钟窗口 + 解码/内容时钟分析助手 |
| `tests/sr-itests/Cargo.toml` | 新增 `wasapi = "0.26"`（仅测试用：参考源渲染） |

既有测试文件未改动（`route/multi_route/audio_*/merge/shot/enum_*/probe/recovery/soak` 原样）。

## 2. L1：`unit_epoch_unit_mapping`（REQ-25）

命令：`cargo test -p sr-pipeline --test epoch_mapping -- --nocapture` → **1 passed**（`m10b-verify-epoch.log`）。

覆盖：
- 10MHz 恒等；典型 QPC 3.579545MHz：1 秒精确、逐 tick 单调、往返误差 ≤1 tick；
- round-half-away-from-zero 与负数对称；`freq<=0` → `None`；i64 饱和（i128 中间量不回绕）；
- UNIX ms ↔ 100ns：向下取整、epoch 前夹 0、u64/i64 饱和、日界往返恒等；
- `Timeline`：T0 零点/1s/屏障前负值、QPC 域与 WASAPI 100ns 域一致性、epoch 回算、`rel_hns_to_qpc`
  可逆、跨日 T0（23:59:59.5 + 1s → 次日 00:00:00.5）。

## 3. L2：`it_window_resize_canvas_stable`（REQ-13）

命令：`cargo test -p sr-itests --test sync it_window_resize_canvas_stable -- --ignored --nocapture`
→ **1 passed**（11.92s，`m10b-verify-resize.log`）。

场景：640x480 窗口起录 → t=2s 改 800x600（4:3）→ t=4s 改 960x360（8:3，比例变化）→ t=6s 最小化 →
t=8s 恢复 → t=10s 停止。

实测：
- 录制段 **10.033s**；成片 **10.100s**（差 0.067s）；收尾 **0.31s**；
- 成片：1 条 h264、**1280x720@30**、无音轨、整文件解码无错误；
- 会话段 **2 个**：逐段 ffprobe 参数与画布完全一致，且**各自独立解码无错误**（闭合 GOP，REQ-21 兼容）；
- 内容：3fps 采样 **30 帧全部非黑**（含最小化区间帧 18..24；最小化期间「零尺寸帧不出帧 → 最近帧重复」，
  未结束录制）；
- → **REQ-13 通过**。未覆盖：显示器热插拔（会改用户桌面拓扑，L2 不可安全自动化；留 L3/专项）。

## 4. L2：`it_sync_content_10min`（REQ-25）——FAIL，命中真实拼接缺陷

命令：`cargo test -p sr-itests --test sync it_sync_content_10min -- --ignored --nocapture`
→ **FAIL**（`m10b-verify-sync.log`）。

### 4.1 方法（内容时钟法，对捕获帧率不敏感）

- **参考源（测试进程内，单一内容时钟）**：
  - WASAPI 渲染线程向默认渲染端点写 48kHz/f32/立体声：每 1.000s 一次 80ms 880Hz 短音；
  - 内容时钟 = 已写出样本数（µs，原子量）；GDI 窗口按该时钟绘制 **2s 周期横向秒表条**
    （640x360，条宽 20px、行程 620px）；
  - 音与画由**同一时钟**驱动 → 源信号严格同刻，不依赖外部播放器呈现时序。
- **测量**：解码成片为 64x36 灰度帧，取亮条列加权中心 → `φ_v = (pos×2s) mod 1s`；解码音轨 8kHz、
  5ms RMS 包络检短音 onset → 对成片时刻 t 求 `φ_a = (t−前一短音) mod 1s`；
  `offset = wrap1s(φ_a − φ_v)`；仅取内容发生变化的新鲜捕获帧。
- **断言（未削弱）**：`|offset| ≤150ms`；末 20% − 首 20% `≤100ms`；`|斜率| ≤0.20ms/s`；
  5s 段边界 ±0.5s 样本 `≤150ms` 且 ≥100 个；短音 ≥580、样本 ≥1000；
  成片 1v+1a / 1080p30 / aac48k / **时长≈录制段（±3s）** / 解码无错。
- 先用锁定 ffmpeg 生成的同结构参考片跑同一解码路径自检：**19681 样本，mean 19.1ms，max|Δ| 31.7ms**
  → 解码器与相位映射正确（源自检无漂移）。

### 4.2 610s 实测：时长与帧数（决定性证据）

| 指标 | 值 |
|---|---|
| 录制段（request_stop 时刻） | **610.164s** |
| 成片容器时长 | **618.636s**（**+8.472s，+1.39%**） |
| 视频帧数 | **18,308**（= 610.267s @30fps） |
| 会话段数 | **123** |
| 各段容器时长之和 | **618.600s**（每段比 `帧数/30` 多 ~68ms） |
| 成片 PTS 空洞 | **122 个**（恰为段边界），单个 **92–108ms**（首个 203ms），合计 ≈ **8.4s** |

结论：**每个 5s 段边界处视频时间轴被冻结约 100ms**，610s 累计 8.4s → 视频时间轴比真实录制段长 1.39%。

**「t0 前预录 ~8s」假设的排除**（编排者提出，已逐项核对）：

| 核对项 | 实测 | 说明 |
|---|---|---|
| 视频帧数 | **18,308 帧 = 610.267s @30fps** | 与录制段 610.164s 一致（+0.10s）；若含 8.4s 预录应为 ~18,559 帧 |
| 路由启动→成片落盘（墙钟） | 文件名时间 **12:35:45** → 成片 mtime **12:45:56.761** = **611.8s** | ≈ 启动 ~0.7s + 录制段 610.16s + 收尾 0.92s；**不含 8.4s 预录** |
| 成片首帧 PTS | **0.021s** | 首帧即录制起点，无负/偏移预录段 |
| 音频包数 | **28,609 包 × 21.33ms = 610.3s** | 音频内容同样为 610.3s，与录制段一致 |
| 容器时长 | **618.636s** | 比两路内容（610.27/610.3s）多 8.4s ⇒ 差额只能是**时间轴空洞** |

音频侧同样存在边界空洞：**122 个 >30ms 的 PTS 间隙**（最大 191ms，累计多出 ≈8.26s），与视频的 122 个
间隙一一对应（段边界）。→ **段容器时长（含音频越过视频切点的尾巴）被 concat 用作段偏移**，
两侧时间轴在每个边界同时插入 ~100ms 停顿；成片比真实时间慢 1.4%。

因此：**时长断言以「录制段 span」为口径是正确的**（首帧时刻 ≈ span 起点，无预录可豁免）；
把断言改为「与首帧时刻一致」在本例不会改变结论（首帧 = 0.021s，时长仍多 8.4s），
而把 ~8s 计为合法预录则会**掩盖真实缺陷**。150ms/不累积/边界断言保持原样。

### 4.3 对照：无音轨时无累积

M10 soak（2 路 1080p30 纯视频，620s，≤124 段）：18,606 帧 = 620.2s 内容，容器时长 **620.300s**
（+0.1s，≈0.8ms/段）→ **无累积缺陷**。
→ 缺陷与**音频流存在**相关：段容器时长取 max(视频, 音频)（AAC 帧粒度/末帧时长），
音频越过视频切点使每段时长偏长；concat demuxer 以此推进时间轴，而视频帧仍按原 30fps 间距排列 →
边界处出现空洞（REQ-21「拼接边界无跳变」）。

### 4.4 内容级测量（15s 快速复现，5 次独立运行）

15s 录制（3 个段边界；调试期临时缩短 `RECORD_SECS` 复现，仓库内用例仍为 spec 的 610s）即可复现：`offset` 由首个样本 ~+2–70ms 线性增长至 12.8s 处 **+250–314ms**，
斜率 **25–29ms/s**（与「~100ms 空洞 / 5s」≈20ms/s 的量级一致，叠加首个 203ms 空洞）。
610s 运行在 `assert_sync_output_shape` 的时长断言处失败（618.636s vs 610.164s，容差 3s），
尚未进入内容分析——**时长证据本身已足以判定失败**；内容级漂移由 15s 复现提供。

### 4.5 根因结论与影响

- **产品缺陷（P0）**：`sr_pipeline::route` 的收尾 concat（`-f concat -c copy`）在**含音轨**的
  分段录制中，按段容器时长推进时间轴，而段容器时长因音频（AAC 帧对齐/末帧时长）比视频内容长约 68–100ms；
  拼接后视频 PTS 在每个段边界出现 ~100ms 空洞。
- 影响：REQ-21「拼接边界无可检测跳变」、REQ-25「任意应同步轨对 ≤150ms 且不累积」不成立；
  10 分钟录制音画漂移 8.4s；REQ-27 长录制的 A/V 同步亦受影响。
- **未在本次修复**：属生产编码/收尾管线改动（段时长口径/音频切点对齐/收尾时间轴重建），需改
  `sidecar`/`route` 并重跑全部 L1+L2（含 10 分钟同步）——超出「最小支持」边界，且须独立评审。
  建议修复方向（供编排者立项）：
  1. 收尾按**视频轨**时长（ffprobe 逐段）生成 ffconcat 的 `duration`，或
  2. 段内音频裁齐视频切点（cut 时对齐 AAC 帧），或
  3. 收尾后以 copy 模式重建连续时间轴（丢弃边界空洞）并校验总时长≈录制段。
- **测试有效**：新用例在 15s 内即可稳定复现，可直接作为修复的验收门。

## 5. 全量回归（独立串行，全部实际执行）

| # | 命令 | 结果 | 日志 |
|---|---|---|---|
| 1 | `cargo test --workspace` | **exit 0；25 passed / 0 failed / 33 ignored** | `m10b-verify-l1-workspace.log` |
| 2 | `cargo test -p sr-pipeline --test epoch_mapping` | **1 passed** | `m10b-verify-epoch.log` |
| 3 | `cargo test -p sr-itests --test sync it_window_resize_canvas_stable -- --ignored` | **1 passed（11.92s）** | `m10b-verify-resize.log` |
| 4 | `cargo test -p sr-itests --test sync it_sync_content_10min -- --ignored` | **FAIL（§4；真实缺陷）** | `m10b-verify-sync.log` |
| 5 | `cargo test -p sr-itests --test route -- --ignored` | **5 passed（60.57s）** | `m10b-verify-route.log` |
| 6 | `cargo test -p sr-itests --test multi_route -- --ignored` | **4 passed（37.39s）** | `m10b-verify-multi_route.log` |
| 7 | `cargo test -p sr-itests --test audio_isolation -- --ignored` | **4 passed（46.26s）** | `m10b-verify-audio_isolation.log` |
| 8 | `cargo test -p sr-itests --test merge -- --ignored` | **1 passed（0.88s）** | `m10b-verify-merge.log` |
| 9 | `cargo test -p sr-itests --test recovery -- --ignored` | **2 passed（47.29s）** | `m10b-verify-recovery.log` |
| 10 | `cargo test -p sr-itests --test probe -- --ignored` | **1 passed（1.50s）** | `m10b-verify-probe.log` |

- 既有测试零回归（route/multi_route/audio_isolation/merge/recovery/probe 与 M10 记录一致）。
- 唯一 warning 仍为既有 `tests/sr-itests/tests/probe.rs` 的 `unused import: EncoderProbe`（非本轮引入）。
- `cargo check --workspace` 0 warning（本轮新增代码无告警；epoch/sync/soak 编译干净）。

## 6. 未决问题 / 交接

1. **P0（阻塞 AC-9）**：concat 段边界 PTS 空洞（§4.5）→ 修复后重跑 `it_sync_content_10min`（10 分钟）
   与 route/multi_route/M10 soak 回归。**在修复前 AC-9 不能判定通过。**
2. `it_sync_content_10min` 依赖测试进程内 WASAPI 渲染（默认渲染端点）；无端点 → 显式 BLOCKED。
3. 本机 WGC 有效捕获率实测约 3.8–4.5 帧/s（临时探针 + CopyFromScreen 双重测量）。内容时钟法对此
   不敏感；但若后续改用短闪 onset 法，必须先确认捕获率（本机不可用）。
4. 显示器热插拔未覆盖（L2 不可安全自动化；留 L3）。
5. `epoch` 目前仅被 `route::start_qpc` 使用；`sr-audio`/`sr-capture` 内仍有各自的 QPC→100ns 旧实现，
   后续可统一（本轮未动，避免扩大范围）。

## 7. 证据分级

- **实际执行（observed）**：§2/§3/§5 全部命令与计数；§4.2 的帧数/段时长/PTS 空洞（ffprobe 实测）；
  §4.4 的 15s 复现（6 次）；§4.3 的 soak 对照（既有输出文件复算）。
- **推断（inferred）**：缺陷与音频存在相关的机制（容器时长口径）；修复方向 1–3。
- **文档记载（documented）**：REQ-21/25/27 阈值引自 spec；M10 缺口引自 m10-rehearsal-report §7/§8。
