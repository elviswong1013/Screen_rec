# M12-H 同步稳定性专项：sync_dual 间歇失败归因与处置

- 日期：2026-10-17
- 范围：`tests/sr-itests/tests/sync_dual.rs` 的 route-A 超阈失败（165.8ms / 245.9ms）。
- 结论：**两次历史失败均不是录制器缺陷**；根因是**参考源（测试子进程）依赖的交互式会话/音频端点状态被打断**：
  - 失败 1（m12f）：两路内容在 294.3s 同时停摆（= 启动+294.3s，21:19:25），距记录的 `Kernel-Power 566 SessionUnlock`（21:19:31）**早 6s**——与会话/端点重配窗口时序邻近（**因果为推断**，未直接观测端点事件）；录制器仍按墙钟补静音、时长与时间轴完整。
  - 失败 2（m12g）：显示/会话状态切换（Kernel-Power 566 InputHid，21:40:18）→ 目标窗口一帧陈旧内容（条形图滞后 166ms）→ 单样本 165.8ms；音频 611 个短音间隔精确 1.0000s，无任何采集异常。
- 应用侧功能代码**未改动**（无应用缺陷可修）；新增 `SR_DIAG_AV=1` 开关下的诊断计数（默认关闭、行为中性），用于后续同类问题归因。
- 验证：冷态 1 次 PASS；**最终源码连续 2 次 PASS**（max 125.2/89.9ms、92.6/88.3ms）；route / multi_route / audio_isolation / multi_route_audio_dual 回归全绿；单测/`check`/`fmt` 全绿。
- 未改测试、`specs/`、`docs/spikes/`；未 `git`；未重建安装包。

## 0. 结论表

| 项 | 结论 | 证据 |
|---|---|---|
| 165.8ms（m12g） | **单帧陈旧内容**（源窗口绘制/合成停顿 166ms），非音频间隙、非 onset 口径问题 | §2.2：611 短音间隔 1.00000s；异常帧 t=248.733 条形图落后 5 帧；前一帧全黑、随后 6px 跳变 |
| 245.9ms（m12f） | **源内容整体停摆的边界伪值**（audio 与 bar 同时冻结），非录制丢块 | §2.1：两条路由在 294.3s 同时失去内容，之后 315.9s 音频为**精确 0.0**、视频冻结；成品仍 610.2s |
| 录制器侧 | 时间轴/时长/漂移均正常：无写阻塞（max 1.06ms）、无 discontinuity、重采样 0 次校正 | §3 计数器；§4 运行表 |
| 两次失败相关性 | 时序与会话状态切换邻近：m12g 同秒，m12f 早 6s（因果为推断） | §2.3（21:19:25/21:19:31；21:40:17.7/21:40:18） |
| 修复 | 无需修应用；仅加诊断（env 开关） | §5 |
| 稳定 PASS | 最终源码 D/E 连续 PASS | §4 |

## 1. 方法与证据来源

1. **离线复核历史失败成品**（关键证据，仍在 `%TEMP%`）：
   - m12f：`sr_m11_dual_sync_96780\out\20261017_211431/211432_*.mkv`
   - m12g：`sr_m11_dual_sync_4744\out\20261017_213609_*.mkv`
   脚本复刻 `sync_dual.rs` 的 onset/64×36 条形图口径（数值与测试日志一致，样本数差 <1），
   输出见 `docs/milestones/m12h-artifact-analysis.log`。
2. **系统事件**：`Get-WinEvent`（System / Application / TerminalServices-LocalSessionManager），
   20:00–23:05 全窗口查询。
3. **仪器化**：新增 `SR_DIAG_AV=1`（§5），在最终源码的 D/E 两次运行中开启，采集侧计数与运行结果同日志。
4. **运行方式**：`cargo test -p sr-itests --test sync_dual -- --ignored --nocapture --test-threads=1`，
   串行、独占机器、未运行其他负载。
   - 说明：工具 shell 未继承 `SESSIONNAME`，运行前显式设为 `Console`（`qwinsta` 实测当前为
     `console ... 1 Active`；仅满足 `require_interactive_session` 的 L2 前置判断，未改测试）。
   - 为排除显示器休眠：临时进程保持 `SetThreadExecutionState(ES_DISPLAY_REQUIRED|ES_SYSTEM_REQUIRED)`；
     该进程与诊断脚本均在 `%TEMP%\sr_diag`，**不在仓库内**。

## 2. 失败取证（观察到）

### 2.1 失败 1（m12f，21:14:31 起，route-A beeps 295/611）

离线复核（`m12h-artifact-analysis.log`）：

```
route-A: audio=610.240s video=610.233s beeps=295 last_beep=294.27  interval=[1.00000,1.00000]s
  last nonzero content at 294.3s; after 300s: max|sample|=0 zero_share=1.0000
  video changed frames=7445/18307  frozen_runs>1s=11 (longest 72.27s at 350.93s)
route-B: audio=610.176s video=610.133s beeps=296 last_beep=294.30  interval=[0.10000,1.00000]s
  last nonzero content at 294.3s; after 300s: max|sample|=0 zero_share=1.0000
  video changed frames=6932/18304  frozen_runs>1s=9  (longest 63.40s at 330.63s)
```

- **两条路由在 294.3s 同时停止产生内容**（不同子进程、不同渲染客户端，同一默认端点）。
- 之后（294.35s 至末尾）音频为**逐样本 0.0**（抽样 300–610s：`zero_share=1.0000`，`max|sample|=0`），
  视频内容冻结但帧数/时长完整：
  `samples=7366` 只是"有无短音"的派生量（295×≈25），不是"丢失约半数音频块"。
- 录制器行为正确：时长 610.240/610.176s、可解码、时间轴未断（墙钟续流生效）。
- 最大 245.9ms 出现在冻结边界 t=294.433s（最后一个"内容变化"帧），属**源内容停摆的边界伪值**。

事件时序：m12f 启动 21:14:31 + 294.3s = **21:19:25**；`Microsoft-Windows-Kernel-Power` 566
`SessionUnlock`（63→64）记录于 **21:19:31**，即失效起点之后 **6 秒（并非同一秒）**。
默认渲染端点为 **NVIDIA HDMI Audio**（`powercfg /requests` 显示该驱动被占用），显示空闲
超时 `VIDEOIDLE(AC)=300s`；294.3s 与"最后一次输入 + 空闲超时"量级一致，但**未直接观测到
端点事件或解锁起始时刻**。可确证的是两条路由在 294.3s 同一刻失去内容、6s 内记录了会话
状态转换；「会话/显示/端点重配 → 共享模式渲染客户端失效（`get_available_space_in_frames`
不再推进、`pos` 冻结）」是**推断**，不是直接观测。测试子进程不处理端点失效，故此后
316s 无声。

### 2.2 失败 2（m12g，21:36:09 起，route-A max 165.8ms）

离线复核复现测试统计：`samples=13281 mean=43.5 max=165.8 head=27.0 tail=56.3 slope=0.0708`（测试 13276）；
route-B 同一时刻也出现 `max=132.7ms @ t=248.633`（未超阈）：

```
route-A: beeps=611 interval=[1.00000,1.00000]s (最大偏差 <0.125ms 采样分辨率)
worst: t=248.733s off=165.8ms
  frames: i=7456 t=248.533 帧全黑（无条形图）
          i=7457..7461 t=248.567..248.700 pos=0.1527（5 帧完全重复=窗口未重绘）
          i=7462 t=248.733 pos=0.1688（唯一帧，滞后约 5 帧/166ms）
          i=7464.. t=248.800.. pos=0.2681→0.2964（6px 跳变追上）
```

- 音频侧：该秒内短音波形干净（1ms 包络自 248.2295 起连续 0.33–0.37，无零段），
  **611 个短音间隔精确 1.0s** → 不是 onset 检测口径、不是音频间隙。
- 视频侧：一帧陈旧条形图（源窗口 WM_TIMER 重绘/合成停顿约 166ms），随后 2 帧内追平；
  之后均值/斜率/末段均正常，**无累积偏移**。
- 事件对齐：启动 21:36:09 + 248.7s = **21:40:17.7**；Kernel-Power 566 `InputHid`（64→66）
  记录于 **21:40:18**（显示/会话状态切换）。输入 HID 触发的是"显示重新点亮"类转换，
  期间 DWM 组合暂停，WGC 会话给出"时间戳新、内容陈旧"的帧。

### 2.3 事件相关性（19:40–23:05）

| 时间 | Kernel-Power 566 | 与运行的关系 |
|---|---|---|
| 19:45:44 | SessionUnlock 60→61 | 两次历史 PASS（19:56 acc-2、20:07 m12-final）之前；两次 PASS 运行窗口内无 566 事件 |
| 20:28:25 | InputHid 61→63 | 两次历史 PASS 之后 |
| **21:19:31** | **SessionUnlock 63→64** | **m12f 失效起点 21:19:25（+294.3s）** |
| **21:40:18** | **InputHid 64→66** | **m12g 异常帧 21:40:17.7（+248.7s）** |
| 21:47:51 | SessionUnlock 66→67 | m12g 结束（21:46:24）之后 |
| 21:50:24 | InputHid 67→69 | 冷态运行 A 之前 |
| 21:50–23:05 | 无 566 事件 | 运行 A–E 期间 |

两次失败中，m12g 与 566 记录同秒；m12f 的失效起点早于 566 记录 6s（时序邻近，**因果为
推断**）。两次历史 PASS 的运行窗口内无 566 切换。
`dwm.exe` 自 2026-10-16 06:05 起未重启；无 TDR/显示驱动/应用崩溃事件。

## 3. 仪器化与数据（D/E 两次 PASS，`SR_DIAG_AV=1`）

新增输出（每次停止各一行）：

```
[diag-video]  slots=18309 fresh=14509 repeats=3800 max_repeat_run=2 max_late_ms=38.0 write_failed=false
[diag-audio]  sr-audio-stream-<pid>: packets=61033 padded=2387847 dropped=2387818 discontinuities=0
[diag-audio-pipe] a0: exit=stop pcm=122054 pad_frames=2387782 payload_frames=26907578
              writes=122054 write_avg_ms=0.03 write_max_ms=0.84 measures=2965 corrections=0
              ppm=[0,0] offset_ms=[-17.4,16.7] flush_frames=1
[diag-route] stop path: request_stop
```

判读：

| 项 | 数据（D/E 四轨） | 结论 |
|---|---|---|
| WASAPI 包 | 61,027–61,035 / 610s（≈100/s） | 采集连续，无丢包 |
| discontinuity | 0 | 引擎未报断流 |
| 管道写 | avg 0.03ms，max 0.48–1.06ms；0 次超时 | **无写阻塞/丢块**（5s 超时从未接近） |
| 视频节拍滞后 | max 1.4–38.0ms（<1 槽=33ms） | 发送器从未被背压顶住 |
| 重复帧 | 3,743–5,202 槽（max 连续 2） | 60Hz 源对 30fps 采样的正常重复，无长停 |
| 漂移决策 | measures 2,963–2,965，**corrections=0**，ppm=[0,0] | 重采样器全程直通（flush=1 帧）；录制器未施加任何速率修正 |
| 漂移内部偏差 | offset_ms ∈ [-30.0, +16.7] | 视频帧计数 vs 音频编排帧数 ≤30ms，无累积 |
| EOF 冲刷 | 1 帧（直通契约） | 收尾正常 |

补充（推断，非本次失败原因，见 §7-2）：`padded == dropped`（D：2,129,363≈2,129,340 与
2,387,847≈2,387,818；E：1,122,432≈1,122,428 与 1,748,504≈1,748,479），即 `pad_to_ts(qpc_now)`
在稳态每轮插入的少量静音随即被下一包迟到裁切等价丢弃——时间轴对齐不受影响，但每秒约
`padded/48000` 的内容被静音替换（本次 23–50s/610s，运行 B 最低 12.5s）。

## 4. 运行结果（观察到）

| # | 时间 | 源码/开关 | route-A | route-B | 结果 |
|---|---|---|---|---|---|
| A（冷态） | 21:52–22:02 | 基线 + SESSIONNAME | dur 610.301, beeps 610, mean 34.9, **max 93.6**, head 15.8, tail 48.0, slope 0.0772 | dur 610.259, beeps 611, mean 37.2, **max 84.8**, slope 0.0769 | **PASS** |
| B | 22:04–22:15 | 诊断 v1 | beeps 610, mean 40.1, **max 91.3**, slope 0.0781 | beeps 610, mean 41.5, **max 93.2**, slope 0.0735 | **PASS** |
| C | 22:15–22:26 | 诊断 v1 | 录制在 **229.667s** 提前结束（见 §6） | 同 | **FAIL（时长断言，非同步）** |
| D | 22:29–22:40 | **最终源码** + `SR_DIAG_AV=1` | beeps 611, mean 45.7, **max 125.2**, head 26.9, tail 58.4, slope 0.0772 | beeps 611, mean 30.6, **max 89.9**, slope 0.0763 | **PASS** |
| E | 22:40–22:51 | **最终源码** + `SR_DIAG_AV=1` | beeps 611, mean 32.6, **max 92.6**, head 14.2, tail 44.9, slope 0.0736 | beeps 611, mean 34.3, **max 88.3**, slope 0.0739 | **PASS** |

D、E 为**最终源码上的连续两次 PASS**（各 ≈10m41s，串行独占）。日志：
`m12i-sync_dual.log`（A）、`m12j`（B）、`m12k`（C）、`m12l`（D）、`m12m`（E）。
冷→热覆盖：A 为冷态首跑，B 紧随其后（同一热态机、仅重建诊断版）PASS；此后 C 为非同步的
提前终止（§6），D/E 为最终源码连续 PASS。

回归（最终源码，`--ignored --nocapture --test-threads=1`）：

| 用例 | 结果 | 日志 |
|---|---|---|
| `route` | 5 passed / 0 failed（60.6s） | `m12h-reg-route.log` |
| `multi_route` | 4 passed / 0 failed（38.8s） | `m12h-reg-multi_route.log` |
| `audio_isolation` | 4 passed / 0 failed（48.5s） | `m12h-reg-audio_isolation.log` |
| `multi_route_audio_dual` | 1 passed / 0 failed（10.9s） | `m12h-reg-multi_route_audio_dual.log` |

单测/静态检查：`cargo test -p sr-audio -p sr-pipeline -p sr-capture` **exit 0**；
`cargo check --workspace --all-targets` **exit 0**（仅既有 `probe.rs` 未用导入告警）；
`cargo fmt --all -- --check` **exit 0**。

## 5. 变更（仅诊断，默认关闭）

功能行为**无改动**；以下均由 `SR_DIAG_AV=1` 门控（`"1"/"true"` 才输出；未设置时仅多几次整数累加）：

| 文件 | 变更 |
|---|---|
| `crates/sr-audio/src/wasapi.rs` | `CaptureStats` 增 `packets`（第 94 行）；`pub fn diag_enabled()`（108）；采集核累加包数；`LoopbackStream` 线程退出时打印 `[diag-audio] packets/padded/dropped/discontinuities`（715–745） |
| `crates/sr-capture/src/wgc.rs` | `FrameForwarder::on_closed`（138）打印 `[diag-video] WGC capture item closed`——区分「目标关闭」与「采集线程静默结束」；本文件内 `diag_enabled()` |
| `crates/sr-pipeline/src/route.rs` | ① `SlotSender` 诊断：`slots/fresh/repeats/max_repeat_run/max_late_ms/write_failed`（245–300，`Instant` 计时仅开关开启时）；② `AudioDiag`（835）：`pcm/pad/payload/writes/write_avg/write_max/measures/corrections/ppm 区间/offset 区间/flush`（905）；③ `flush_audio_tail` 返回冲刷帧数（1440）；④ `run_worker` 三条停止路径打点 + `window_alive`（`IsWindow`）判定（1465–1493）；⑤ `RunCtx.target_hwnd`（800/1161/1418） |

诊断口径说明：本管线的音频输入是无时间戳的 `f32le` 管道，ffmpeg 按帧连续赋 PTS，
**不存在 AAC 输入 PTS 跳跃**；采集侧的"间隙"只能来自 `pad_frames`（补零）与
`trimmed_frames`（迟到裁切），二者已在 `[diag-audio]` 中直接记账。
开关关闭时行为中性：`SR_M11_DUAL_SECS=20` 短跑（未设 `SR_DIAG_AV`）PASS 且日志无任何
`[diag` 行（仅验证输出被门控，不作为同步结论）。

## 6. 运行 C 的提前终止（未决，非同步失败）

- 现象：两路均在 **229.667s** 结束（46 个 5s 段；manifest `state=final`），成品可解码、
  `final_path` 已提交；测试在 `sync_dual.rs:598` 时长断言失败（229.667 vs 610.116）。
- 侧证：停止路径诊断当时尚未加入；两条路由的音频写线程均 `exit=stop`（由关停触发），
  视频发送器 `write_failed=false`、滞后 ≤29.9ms；无 sidecar stderr 残留、无应用/驱动/会话事件
  （22:15:49–22:26:02 无 Kernel-Power 566、无 WER、`dwm` 未重启）。
- 归因：**WGC 采集项在 229.7s 结束**（`CaptureControl::is_finished()`），录制器按"目标丢失"
  语义优雅停止——行为符合契约，但触发源未定（目标窗口销毁 vs 采集线程终止）。
- 处置：已在 `run_worker` 增加停止路径打点与 `window_alive` 判定；下次复现即可判定。
  1/5 次运行出现，未影响 D/E 结论。

## 7. 未决 / 建议（回交编排者）

1. **会话/端点状态是长测的第一风险（已实证）**：两次失败均与 Kernel-Power 566 记录时序邻近
   （m12g 同秒、m12f 早 6s；因果为推断）。
   建议 L2 长测规范：运行前解锁、运行期间禁止锁屏/切换显示、显式保持
   `ES_DISPLAY_REQUIRED`（本次 D/E 即在此条件下通过）。测试子进程本身不处理
   `AUDCLNT_E_DEVICE_INVALIDATED`（端点失效后 `pos` 永久冻结），属测试源局限。
2. **（已发现；M13 已修复）静音补齐/迟到裁切稳态互抵**：
   `run_capture_core` 每轮 drain 后无条件 `pad_to_ts(qpc_100ns())`，而健康管线的包时间戳
   天然落后 QPC 一个端点延迟 → 每包补少量静音、下一包再被等价裁切。计数（§3）显示
   610s 内 0.6–2.4M 帧（12–50s）内容被静音替换；时间轴对齐不受影响（故不影响本用例），
   但属内容完整性缺陷，且使 REQ-26 漂移核看不见端点时钟偏差（`corrections=0`）。
   建议最小补丁：仅当"距上一包到达 > 50ms"才 `pad_to_ts`（语义回归其文档本意
   "源停滞/音源消失续流"），随后重跑 sync/sync_dual/soak 验证。**M12-H 时未实施**
   （超出同步归因范围，且会改变全部录制的音频内容，需编排者决策）；**M13 已按此方案
   实施并验证**（见 `docs/milestones/m13-report.md`）。
3. **用例固有裕度**：即使无任何会话切换，参考源窗口的绘制抖动仍可达 91–125ms（阈值 150ms）；
   `slope` 73–88ppm 是参考源渲染时钟（DAC）相对 QPC 的偏差（录制器重采样 0 校正可证），
   非录制漂移。若要提高稳定性，建议（需批准改测试）让参考源用 QPC 墙钟驱动条形图/短音，
   或增大条形图尺寸/降低量化误差；当前不改测试。
4. **WGC 采集项提前结束的再武装**：若 §6 复现为 `window_alive=true`，建议在 worker 中
   以同一 HWND 重建 WGC 会话（复用最近帧槽与 30fps 发送器，帧间隔用重复帧填充），
   而不是停止整条路由；需单列任务。
5. **运行 C 的 229.667s 停止**未定位（§6）；不影响 D/E 的连续 PASS 结论，但说明长测仍有
   非同步类的环境/OS 风险。

## 8. 证据索引

| 证据 | 内容 |
|---|---|
| `docs/milestones/m12h-artifact-analysis.log` | 历史失败成品与 PASS 成品的离线复核（beeps/间隔/静音段/视频冻结/offset 统计/异常帧序列） |
| `docs/milestones/m12i-sync_dual.log` | 冷态运行 A：PASS，max 93.6/84.8ms |
| `docs/milestones/m12j-sync_dual.log` | 运行 B：PASS，含首批诊断计数 |
| `docs/milestones/m12k-sync_dual.log` | 运行 C：229.667s 提前结束（时长断言失败） |
| `docs/milestones/m12l-sync_dual.log` | **最终源码 PASS #1**：max 125.2/89.9ms + 完整诊断 |
| `docs/milestones/m12m-sync_dual.log` | **最终源码 PASS #2**：max 92.6/88.3ms + 完整诊断 |
| `docs/milestones/m12h-reg-*.log` | route / multi_route / audio_isolation / multi_route_audio_dual 回归 |
| 系统事件 | `Get-WinEvent` System：Kernel-Power 566（20:28:25 / 21:19:31 / 21:40:18 / 21:47:51 / 21:50:24）；21:50–23:05 无切换 |
| 现场成品 | `%TEMP%\sr_m11_dual_sync_96780`（m12f）、`..._4744`（m12g）、`..._100168`（B）、`..._91612`（D）、`..._99520`（E） |
