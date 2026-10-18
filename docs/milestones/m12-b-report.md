# M12-B 验收整改报告：REQ-26 慢漂移改用 rubato Sinc

日期：2026-10-17
范围：`crates/sr-audio` 的 `DriftResampler` 实现/单测与依赖声明、`crates/sr-pipeline/src/route.rs`
音频写线程停止路径的冲刷接线、本报告。
未触碰 `specs/`、`docs/spikes/`、`Timeline`/`DriftController` 契约与既有测试断言。

## 1. 结论

REQ-26 慢漂移路径已从自定义线性插值替换为 **rubato 5.0.1 `Async` + Sinc 插值**
（BlackmanHarris2 窗、Cubic 插值、`FixedAsync::Input`），公开 API
`DriftResampler::{new, set_rate_ppm, process}` 保持不变，`route.rs:1043/1101`
调用点零改动。既有 4 项 `drift_resampler` 断言语义未变；新增 7 项用例（4 项正确性/
延迟补偿 + 3 项停止冲刷）。停止路径现在在音频管道关闭/EOF 之前调用 `flush()`，
补齐 rubato 滤波延迟尾部（直通补出保留的 1 帧）。
`cargo test -p sr-audio`、`cargo test -p sr-pipeline`、workspace L1、
`cargo check --workspace --all-targets`、`cargo deny check licenses` 与
`license_and_build_audit` 均通过。

## 2. 实现要点（`crates/sr-audio/src/resample.rs`）

| 项 | 位置 | 说明 |
|---|---|---|
| 依赖 | `crates/sr-audio/Cargo.toml:15-18` | `rubato = { version = "5.0.1", default-features = false }`；仅用 Async Sinc，关闭默认 `fft_resampler`，不引入 realfft/num-complex。`Cargo.lock` 相应新增 rubato/audioadapter 树（7 个包，许可均在 `deny.toml` 白名单内） |
| 滤波器参数 | `resample.rs:87` | `SINC_LEN = 128`（tap；48kHz 下群延迟约 1.3ms），`BlackmanHarris2` + Cubic + 默认 oversampling=128；S6 可调 |
| 分块 | `resample.rs:91,268-293,370-378` | 任意输入按 `MAX_CHUNK=8192` 帧切块，逐块 `set_chunk_size(n)` + `process`；rubato 内部状态跨块连续，无额外输入缓冲 |
| 速率映射 | `resample.rs:169-178` | `ratio = 1/(1+ppm/1e6)`（与既有语义一致：正 ppm = 音频落后 → 加速 → 输出变少）；运行中变速 `set_resample_ratio(ratio, false)` |
| 比率限幅 | `resample.rs:98,173` | 重采样器输入限幅 **±1000ppm**（沿用既有测试契约），`DriftController` 的规格锚点仍为 ±500ppm（`DriftCfg` 注入，`route.rs:1038-1042`） |
| 0ppm 直通 | `resample.rs:250-266` | 尚未进入校正（始终 0ppm）时保持逐样本恒等 + 1 帧尾部，满足既有 `zero_ppm_is_identity_across_chunks` 契约；一旦接入 rubato，0ppm 仍走 ratio=1.0，避免两种延迟间来回切换 |
| 延迟记账 | `resample.rs:102,232-240,358-360` | `delay_frames() -> f64`：直通=1；rubato 冷启动=`(SINC_LEN/2−1)×ratio`≈63 帧（1.31ms）；中途接入补偿后=0。常量，不随运行时间累积 |
| 延迟补偿 | `resample.rs:202-217,286-292` | 首次进入校正时用直通期保留的最近 64 帧历史预热 rubato，丢弃预热输出 + `T=H·r−W+D` 帧，使校正起点恰好从直通保留的下一帧接续：无重放/无跳帧（单测 `midstream_engagement_...`） |
| 停止冲刷 | `resample.rs:306-350` | `flush()`：直通返回保留的 1 帧；rubato 泵零直到已返回帧数达到 `round(Σ输入×比率 + 冷启动延迟)`，按目标截断；中途接入延迟项为 0。幂等（终态），输出自然衰减尾部而非额外静音段 |
| 冲刷接线 | `route.rs:1049-1054`（正常停止）、`route.rs:1111-1115`（采集流终止）、`route.rs:1158-1165`（`flush_audio_tail`） | 音频写线程退出前、`AudioTrack::shutdown` 关闭音频管道/发 EOF 之前写入尾部；写失败仅记录诊断，不改变停止归因 |
| 接口/鲁棒性 | `resample.rs:363-378` | 构造参数固定有效；`process` 输出交错 f32；空输入返回空；通道数断言不变 |

延迟补偿的边界：冷启动（尚未写过样本）时 rubato 前约 63 输出帧为滤波器启动斜坡，
由 `delay_frames()` 显式记账；停止时 `flush()` 按理论帧数（`N×ratio + 延迟`）补齐尾部，
流末不再截断。中途接入已把延迟补偿到 0，停止时总帧数 = 直通段 + 变比段（无额外延迟项）。

## 3. 测试（`crates/sr-audio/tests/drift_resampler.rs`）

既有 4 项：断言语义未改动（`resample.rs` 的替换使它们继续通过）。

新增 7 项（`file:line` 为测试起点）：

1. `impulse_delay_matches_accounted_filter_delay`（:100）
   - +500ppm、两枚脉冲（输入 1000 / 20000 帧）；
   - 断言峰值位置 = `p×ratio + delay_frames()` ±1.5 帧；峰值幅度 >0.5；
   - 断言两脉冲间距 = `19000×ratio` ±1.5 帧（延迟不累积）；
   - 断言 `delay_frames()` ∈ [60,68]，即约 `SINC_LEN/2`。
2. `sine_content_matches_compensated_ratio`（:155）
   - ±500ppm、1s 440Hz；
   - 输出帧数 = `48000×ratio` ±2；
   - 跳过启动斜坡/尾部后逐样本对齐 `sin(2π·440·(i−delay)·step/48000)`，最大误差 <1e-3。
3. `midstream_engagement_compensates_delay_without_replay`（:192）
   - 先 0ppm 直通 2400 帧，再切 +500ppm，继续 2400 帧；
   - 断言 `delay_frames()` 1.0 → 0.0；
   - 总帧数 ≈ `4800×ratio − 滤波延迟` ±4；
   - 接缝两侧按“直通内容 + step 累积”对齐，最大偏差 <3e-3（<0.1 帧舍入残差）。
4. `rate_changes_keep_stream_finite_and_ratio_correct`（:241）
   - +500ppm → 0 → −500ppm 连续切换；
   - 总帧数 ≈ `4800/1.0005 + 4800 + 4800/0.9995` ±4，输出全部有限。
5. `flush_recovers_filter_tail_and_matches_theory`（:262）
   - 冷启动 +500ppm、1s 输入：`process` 后 `flush()`；
   - 断言总帧数 ≈ `48000×ratio + delay_frames()`（含 filter delay）**±2 帧**；
   - 尾部非空、样本有限、二次 `flush()` 为空（幂等）。
6. `flush_after_midstream_engage_matches_mixed_rate_theory`（:287）
   - 直通 2400 帧 → +500ppm 2400 帧 → `flush()`；
   - 总帧数 ≈ `2400 + 2400×ratio` ±2（中途接入已补偿，无额外延迟项）。
7. `flush_passthrough_emits_held_tail_frame`（:313）
   - 未进入校正：`process` 仍保留 1 帧（既有契约），`flush()` 补出该帧；
   - 停止后总帧数 = 输入帧数，且逐样本 <1e-6 恒等。

## 4. 验证命令与结果（实际执行）

| # | 命令 | 结果 |
|---|---|---|
| 1 | `cargo test -p sr-audio` | **13 passed / 0 failed**（hysteresis 1 + resampler 11 + silence_pad 1）；`drift_resampler` 0.46s |
| 2 | `cargo test -p sr-audio --test drift_resampler -- --nocapture` | **11 passed / 0 failed**（既有 4 + 新增 7） |
| 3 | `cargo test -p sr-pipeline` | **10 passed / 0 failed / 0 ignored**（含 `license_and_build_audit`） |
| 4 | `cargo check --workspace --all-targets` | **Finished**；唯一 warning 为既有 `tests/sr-itests/tests/probe.rs:31 unused import EncoderProbe` |
| 5 | `cargo test --workspace` | **exit 0**；55 passed / 0 failed / 42 ignored（L2 长测按约定未执行）。注：并行执行期间另一 lane 正在编辑 `sr-pipeline/src/naming.rs`，曾出现一次瞬时语法错误/safe_commit 失败；其编辑完成后重跑上述命令为 exit 0（本 lane 未触碰该文件） |
| 6 | `cargo deny check licenses` | **licenses ok**（仅既有未命中白名单条目的 warning：BSD-2-Clause/BSL-1.0/OpenSSL） |
| 7 | `cargo test -p sr-pipeline --test license_and_build_audit -- --nocapture` | **1 passed**；其中 `cargo deny check licenses exit=0` |

未运行：任何 L2 长测（`sync_dual`/`soak_dual`/route/multi_route 等 `#[ignore]` 用例），
按任务要求不执行。未执行 `git` 操作。

## 5. 未决 / 说明

1. **0ppm 直通路径**保留为逐样本恒等（既有测试契约要求 input−1 帧且逐样本 <1e-6）。
   rubato 在 ratio=1.0 时无法满足该“零延迟恒等”断言，故仅在“从未进入校正”时直通；
   一旦发生漂移校正即永久走 rubato（0ppm 时 ratio=1.0），慢漂移路径本身 100% 由
   rubato Sinc 承担。
2. **停止冲刷（原未决 #2，已解决）**：`DriftResampler::flush()` + `route.rs` 写线程
   停止/采集终止路径接线；正常停止时在音频管道关闭、发 EOF 之前写入尾部。冷启动
   总帧数 = `N×ratio + 滤波延迟`，中途接入 = `直通段 + 变比段`；单测断言误差 ≤2 帧。
   若进程被强杀/未走 `AudioTrack::shutdown`，则无冲刷机会（进程级恢复不承诺，符合
   REQ-26 故障时间轴约定）。
3. `cargo fmt --check` 对该文件报告既有格式差异（`DriftAction` 等旧代码即非 rustfmt
   输出）；为避免无关 churn 未做全文件重排，新代码沿用文件现有紧凑风格。
4. 既有 `rate_is_clamped_to_1000ppm` 断言保留 ±1000ppm 限幅；规格锚点 ±500ppm 由
   `DriftController`/`DriftCfg` 承担（未改）。
