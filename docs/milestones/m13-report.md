# M13 报告：WASAPI 补齐/裁切互抵修复（P1）

- 日期：2026-10-18（工作窗口 2026-10-17 23:30 – 2026-10-18 00:20）
- 范围：`crates/sr-audio/src/slots.rs`、`crates/sr-audio/src/wasapi.rs`、新增
  `crates/sr-audio/tests/stall_pad_policy.rs`；`docs/milestones/m12-h-report.md` 归因措辞更正。
- 上游：`docs/milestones/m12-h-report.md` §200–207（第 4 轮验收 REJECTED 的唯一硬阻断）。
- 约束遵守：未改既有测试断言/`specs/`/PRD；未 `git`；未重建安装包；诊断仍走 `SR_DIAG_AV` 门控；
  临时红色验证改动已还原（当前源码为门控版，见 §3）。

## 0. 结论

| 项 | 结果 | 证据 |
|---|---|---|
| 补静音触发条件 | **已修**：仅「距上一真实包结束 ≥ 50ms」才补齐；正常端点延迟不再被补零替换 | §2；`DEFAULT_STALL_100NS` |
| 新回归（验收 a/b） | **3 个 L1 用例**（正常延迟 0 替换 / 真实停滞补齐 / 阈值边界），默认 `cargo test` 执行；红→绿已验证 | §3 |
| 内容完整性（610s 双路） | 被静音替换量 **1,122,428–2,387,847 帧（23.4–49.7s）→ 0 帧（0.000s）**；真实载荷 560.6–586.9s → **610.25s / 610.13s** | §4 |
| 成品内容级复核 | 确定性短音能量：修复前 0.3339–0.3429，修复后 0.3478/0.3487（+1.4%…+4.4%，与被移除比例一致） | §4.2 |
| 回归 | `cargo test --workspace` exit 0；route 5/5、audio_isolation 4/4、m11_audio_only_recovery 2/2、multi_route_audio_dual 1/1；sync_dual PASS；soak_dual PASS | §5 |
| m12-h 措辞 | 已改为「21:19:25 早于 21:19:31 记录 6s，时序邻近，因果为推断」 | §6 |

## 1. 根因（代码路径与量级）

1. 旧实现（`crates/sr-audio/src/wasapi.rs:481–494`）在采集核每轮 drain 之后**无条件**
   `timeline.pad_to_ts(qpc_100ns())`，不判断源是否真实停滞。
2. loopback 包时间戳（设备位置）相对当前 QPC 天然落后一个端点延迟 `L`（M12-H 实测 0.3–0.7ms；
   本机 WASAPI 周期 10ms/26.6ms 两种，drain 线程调度抖动可达数十 ms）。
3. 于是每轮补齐 `L` → 下一包起始槽 < 水位 `L` → **迟到裁切 `L`**：时间轴长度不变，但
   `L` 份真实载荷被静音替换。落地为 `padded == dropped`（M12-H §3：1.1–2.4M 帧 / 23–50s 每 610s）。
4. 二阶层效应：`device_frames`（音频编排帧数）被补齐量锚在墙钟上，REQ-26 漂移核因此看不到
   端点时钟偏差（`corrections=0`，ppm 恒 0）。

## 2. 修复（diff 摘要）

### `crates/sr-audio/src/slots.rs`
- 新增 `pub const DEFAULT_STALL_100NS: i64 = 500_000;`（50ms）与取值论证（见下）。
- `SampleSlotPacer` 新增：
  - `last_packet_end_100ns: Option<i64>`：最近真实包的**结束时刻**（100ns，内容时间域）；
    仅在 `on_packet` 更新并取 `max`（乱序/迟到包不得把停滞判据往回拨）。
  - `stall_100ns: i64`：停滞阈值；`new()` 取默认值，新增
    `with_stall_threshold(sample_rate, stall_100ns)` 供特殊端点周期/测试注入。
  - `packet_end_100ns()`：`ts + frames×10⁷/rate`（与起始槽同一换算）。
  - `pad_to_wall_clock(now_100ns) -> u64`：`now − last_end < stall` → **不补、时间轴不动**；
    否则委托 `pad_to_ts(now)`。这是采集核唯一使用的续流入口。
- `pad_to_ts` 保留为**无条件原语**（文档标注「流式采集应使用 `pad_to_wall_clock`」）；
  `pad_to`（时长钉齐）不变；REQ-18 的补齐/裁切/单调水位语义不变。

### `crates/sr-audio/src/wasapi.rs`
- `CaptureTimeline::pad_to_ts` → `CaptureTimeline::pad_to_wall_clock`（委托 pacer）；
- 采集核调用点改为 `timeline.pad_to_wall_clock(qpc_100ns())`，并注释门控条件。

### 阈值取值（50ms）
- WASAPI 共享模式周期标称 **10ms**（本机实测 10ms；`m12k` 一次为 26.6ms）；
- endpoint/loopback 延迟实测 **0.3–0.7ms**（旧 `pad_frames`/事件 = 17–34 帧）；
- 50ms = **5×标称周期**、≈**1.9×最坏实测周期**，覆盖 drain 调度抖动，不会误判正常包；
- 真实源停滞（端点失效/音源消失）通常 **≫100ms**（M12-H m12f 为 316s），不会漏判；
- 阈值只决定「何时开始补」；补的目标仍是 `now`，停滞恢复后时间轴仍与墙钟对齐。

**语义保持**：短停滞（<50ms）不触发墙钟续流，但仍由 `on_packet` 的空洞路径按**包时间戳**
补齐（`pad_frames`）——REQ-18「流不断」不依赖墙钟门控；只有源完全不产包 ≥50ms 时才由
`pad_to_wall_clock` 兜底。SILENT 包占时间轴、迟到裁切、单调水位、`pad_to` 时长钉齐均未变。

## 3. 新增确定性回归（附加式，未改既有测试）

文件：`crates/sr-audio/tests/stall_pad_policy.rs`（**L1**，无 `#[ignore]`）。

**层级理由**：纯逻辑（`SampleSlotPacer`，无 COM/端点/真实时钟），确定性、微秒级；与既有
`crates/sr-audio/tests/silence_pad_late_packet.rs` 同层，`cargo test` / `cargo test --workspace`
默认执行，无需交互式会话（不必进 L2）。

| 验收要求 | 用例 | 断言（确定性） |
|---|---|---|
| (a) 正常端点延迟不得被补零替换 | `unit_normal_endpoint_delay_never_replaces_payload` | 600×10ms 包、延迟序列 0.2–8ms、每 7 轮模拟 2 包批量 drain → `padded==0`、`trimmed==0`、`written==288000` |
| (b) 真实停滞确实补齐 | `unit_real_stall_pads_only_after_threshold` | 1s 内容 + 300ms 停滞：<50ms 一律不补；总补 **14,400 帧（300ms）**；恢复包 `plan(0,0,62400)` 接续 |
| （附加）阈值边界 | `unit_stall_threshold_boundary_is_inclusive` | 差 1×100ns 不补；恰 50ms 补 2,400 帧 |

**红→绿证据**（临时把 `pad_to_wall_clock` 的门控改为旧的无条件路径，运行后已还原，
源码无残留；还原后 `cargo fmt --check` 通过、3/3 绿）：

| 用例 | 旧无条件行为（红） |
|---|---|
| 正常端点延迟 | `padded = 46,547` 帧（≈0.97s / 6s 模拟） → 断言失败 |
| 真实停滞 | 阈值前已补 `2,160` 帧 → 断言失败 |
| 阈值边界 | 50ms 前补 `2,399` 帧 → 断言失败 |

现结果：`cargo test -p sr-audio --test stall_pad_policy` → **3 passed / 0 failed**；
既有 `silence_pad_late_packet`（1）+ `drift_*`（12）合计 13 项仍全绿。

## 4. 内容完整性（修复前 vs 修复后，610s 双路，`SR_DIAG_AV=1`）

### 4.1 采集侧计数（每音轨）

| 运行 | 路由 | packets | padded（帧 / 秒） | dropped（帧） | payload（帧 / 秒） |
|---|---|---|---|---|---|
| m12j（B，修复前） | A | 61,023 | 599,222 / **12.48s** | 599,222 | 28,691,338 / 597.71s |
| | B | 61,007 | 2,066,573 / **43.05s** | 2,066,543 | 27,216,347 / 566.99s |
| m12l（D，修复前） | A | 61,033 | 2,387,847 / **49.75s** | 2,387,818 | 26,907,578 / 560.57s |
| | B | 61,028 | 2,129,363 / **44.36s** | 2,129,340 | 27,163,635 / 565.91s |
| m12m（E，修复前） | A | 61,035 | 1,748,504 / **36.43s** | 1,748,479 | 27,547,859 / 573.91s |
| | B | 61,027 | 1,122,432 / **23.38s** | 1,122,428 | 28,170,069 / 586.88s |
| **m13（修复后）** | A | 61,026 | **0 / 0.000s** | **0** | 29,292,000 / **610.25s** |
| | B | 61,014 | **0 / 0.000s** | **0** | 29,286,240 / **610.13s** |

- 修复前 `padded ≈ dropped`（差 ≤800 帧 = 末尾未补偿的补齐）：即补齐被等量裁切抵消。
- 修复后 **`padded = dropped = 0`**；真实载荷从 560.6–586.9s 增至 610.13–610.25s
  （增量 = 修复前被替换量，误差 <0.1s/610s）。
- 结论：**610s 内被静音替换的样本总量 23.4–49.7s → 0.000s**。

### 4.2 成品内容级复核（独立于采集计数）

`docs/milestones/m13-content-integrity.log`：对成品解码后，取每个短音
`onset+1ms .. onset+79ms` 的 RMS 中位数（源为确定性 0.35 幅度正弦；AAC 有损，故用整段能量
而非采样级零段；绝对值含 ffmpeg 立体声→单声道 downmix 增益 ×1.414，仅作**相对**比较）：

| 成品 | 短音数 | RMS 中位 | 相对修复前同路 |
|---|---|---|---|
| m12j routeA（padded 2.0%） | 610 | 0.34286 | — |
| m12j routeB（padded 7.0%） | 610 | 0.33386 | — |
| m12l routeA（padded 8.2%） | 610 | 0.33599 | — |
| m12m routeA（padded 6.0%） | 611 | 0.33918 | — |
| **m13 routeA（padded 0）** | 610 | **0.34776** | +1.44%（对 m12j-A）/ +3.50%（对 m12l-A） |
| **m13 routeB（padded 0）** | 610 | **0.34868** | +4.44%（对 m12j-B） |

- 修复后两路短音能量几乎一致（0.34776 vs 0.34868，差 0.26%）；修复前两路相差 2.7%
  （与 padded 比例差一致）。
- 能量增幅（+1.4%…+4.4%）小于名义替换比例（2.0%/6.0%/7.0%/8.2%），因为 AAC 会把短时
  零段与相邻样本抹平、部分能量被重建；方向与量级均与"载荷被静音替换"一致，且修复后
  回到同一水平。

## 5. 运行结果（观察到）

| 用例/检查 | 命令要点 | 结果 | 日志 |
|---|---|---|---|
| workspace L1 | `cargo test --workspace` | **exit 0**（全部 `ok`，L2 `#[ignore]` 未跑） | `m13-workspace-L1.log` |
| route | `--ignored --nocapture --test-threads=1` | 5 passed / 0 failed（60.83s） | `m13-reg-route.log` |
| audio_isolation | 同上 | 4 passed / 0 failed（48.54s） | `m13-reg-audio_isolation.log` |
| m11_audio_only_recovery | 同上 | 2 passed / 0 failed（5.64s） | `m13-reg-m11_audio_only_recovery.log` |
| multi_route_audio_dual | 同上 | 1 passed / 0 failed（10.88s） | `m13-reg-multi_route_audio_dual.log` |
| **sync_dual**（内容完整性运行） | 同上 + `SR_DIAG_AV=1` | **PASS**：A `max 99.6ms`（beeps 611）、B `max 93.3ms`（beeps 610）；`padded/dropped=0` | `m13-sync_dual.log` |
| **soak_dual**（620s×2 路） | 同上 + `SR_DIAG_AV=1` | **PASS**：两路 620s 均带音频；时/形状/可解码/无残留；`padded/dropped=0` | `m13-soak_dual.log` |

运行环境：`SESSIONNAME=Console`（实测 console 会话 1、Active、未锁屏）；运行期以
`SetThreadExecutionState(ES_DISPLAY_REQUIRED|ES_SYSTEM_REQUIRED)` 保持显示/系统唤醒
（M12-H 已证会话/显示切换是本用例的主要环境风险）；串行、独占机器。

**观察（非缺陷）**：修复后漂移核不再被补齐量掩盖——`sync_dual` 两路 `corrections=0`
（内部 offset ±20ms）；`soak_dual` 一路出现 1 次校正（`-500ppm`，offset 瞬态 -100ms 后回
Hold），属 REQ-26 预期响应，用例通过。

## 6. `m12-h-report.md` 措辞更正

验收指出的「21:19:25（首次停止）vs 21:19:31（566 事件）非精确同秒」已如实修正：

- §0 结论表：改为「时序与会话状态切换邻近：m12g 同秒，m12f 早 6s（因果为推断）」；
- §2.1：明确「失效起点 21:19:25，SessionUnlock 记录 21:19:31，**并非同一秒**；未直接观测
  端点事件或解锁起始时刻，因果为**推断**」；
- §2.3/§7-1：同步改为「m12g 同秒、m12f 早 6s」；
- §7-2：标注该补齐/裁切缺陷**已由 M13 修复**并指向本报告。

## 7. 未决 / 风险（回交编排者）

1. **阈值是经验值**：50ms 基于本机 10ms/26.6ms 周期与 0.3–0.7ms 端点延迟。若目标机器
   端点周期更大（部分蓝牙/USB 可到 20–40ms）或系统负载导致 drain 间隙接近 50ms，仍可能
   误补/漏补。已保留 `with_stall_threshold` 注入点；后续可按
   `IAudioClient::GetDevicePeriod` 自适应（建议单列任务，本任务不改契约）。
2. **漂移核行为变化**：修复后音频时间轴跟随设备时钟，漂移核首次能看到端点时钟偏差并可能
   校正（`soak_dual` 1 次）。建议在长稳任务中观察是否需要抑制启动期瞬态（本任务未改漂移核）。
3. **音频轨道总长**：修复前 = 载荷 + 补齐、修复后 = 全载荷，二者均 ≈610.2–610.3s；容器时长
   （视频锚定）与用例容差（±3s）不受影响。
4. **M12-H 运行 C 的 WGC 提前结束**（§6）本任务未复现；其诊断（`on_closed` + `window_alive`）
   已在位，若复现可按 M12-H §7-4 再武装。
5. 未重跑 `soak`（单路）与 `sync`（单路）——任务清单未要求；如需完整矩阵可补跑。

## 8. 证据索引

| 证据 | 内容 |
|---|---|
| `docs/milestones/m13-workspace-L1.log` | `cargo test --workspace` 全量输出（exit 0） |
| `docs/milestones/m13-sync_dual.log` | 610s 双路：PASS + `[diag-audio] padded=0 dropped=0` + `[diag-video]` |
| `docs/milestones/m13-soak_dual.log` | 620s×2 路：PASS + `padded=0 dropped=0` |
| `docs/milestones/m13-reg-route.log` | route 5/5 |
| `docs/milestones/m13-reg-audio_isolation.log` | audio_isolation 4/4 |
| `docs/milestones/m13-reg-m11_audio_only_recovery.log` | m11_audio_only_recovery 2/2 |
| `docs/milestones/m13-reg-multi_route_audio_dual.log` | multi_route_audio_dual 1/1 |
| `docs/milestones/m13-content-integrity.log` | 成品逐短音 RMS 对比（修复前/后） |
| 新测试 | `crates/sr-audio/tests/stall_pad_policy.rs`（3 项，L1） |
| 现场成品 | `%TEMP%\sr_m11_dual_sync_60100`（修复后 sync_dual）、`sr_m11_soak_dual_out_31692`（修复后 soak_dual） |
