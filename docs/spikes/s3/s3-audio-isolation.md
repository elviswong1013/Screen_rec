# S3 Spike：WASAPI 按进程音频隔离实测报告

- 日期：2026-10-16（本机，Asia/Shanghai）
- 课题：验证「两个应用同时播放时，按进程（application loopback）只录到目标应用的声音」在本机是否成立；确定技术路径（wasapi crate vs windows-rs 直实现）、consent/线程要求、格式与静音行为；为 M4 提供可复用最小样例。
- 规格依据：`specs/001-screen-recorder.md` §8（S3）；`docs/research/feasibility-2026-10-15.md` §8（wasapi 0.26）
- 产物：
  - 最小样例（独立 crate，非生产代码）：`tools/spikes/s3/apploop/`（`record` / `analyze` / `probe` 三个子命令，wasapi 0.26.0）
  - 测试矩阵脚本：`tools/spikes/s3/run_s3_tests.ps1`
  - 原始证据（日志/FFT/频谱图）：`docs/spikes/s3/raw/`
- 标记说明：**【实测】**=本机实际执行所得；**【未验证】**=本次未覆盖或无法覆盖。

## 0. 结论速览

| # | 结论 | 等级 |
|---|---|---|
| 1 | **按进程隔离成立**：A(440Hz)+B(880Hz) 同时经默认输出设备播放时，录 A 的非目标 880 分量 ≤ −158 dB、录 B 的非目标 440 分量 ≤ −160 dB（预锁定阈值 −40 dB）；整机混音对照确认两声确实同时在混音里（相差 0.62 dB） | 【实测】 |
| 2 | **双客户端并发可行**：同一进程内 2 条采集线程、2 个 loopback 客户端同时录两个目标，均激活成功且各自隔离；并发上限未测 | 【实测】(2 路)；上限【未验证】 |
| 3 | **静音行为**：目标进程停止（或无渲染会话）后流**不会断**——引擎继续以 480 帧/10 ms 供数，内容为数字零，**不置 SILENT / DATA_DISCONTINUITY 旗标**；停止点 QPC 时间戳有一次 ~13 ms 跳变；补静音策略可把时长精确补到请求值 | 【实测】 |
| 4 | **激活/consent/线程**：激活全程无任何 consent 弹窗（本 spike 累计数十次激活）；`initialize_mta()` 在已 STA 线程返回 `0x80010106`（RPC_E_CHANGED_MODE）；采集必须在专用 MTA 工作线程（勿在 UI 线程同步等待） | 【实测】（本机） |
| 5 | **技术路径**：采用 **wasapi 0.26 crate**——其内部完整实现 `ActivateAudioInterfaceAsync + VT_BLOB PROPVARIANT` 流程（代码审读 + 实测），无需自行用 windows-rs 重写该路径 | 【实测】+ 源码审读 |
| 6 | Win10 2004 (19041) 及更老系统的行为与承诺 | 【未验证】（本机为 Win11 build 26200） |

> 复验说明：全部【实测】项已在标准工具链（用户级 rustup `stable-x86_64-pc-windows-msvc` + VS Build Tools 2022）下重建重跑，**27/27 通过**；首轮（便携 GNU 引导工具链）与末轮（标准 MSVC）关键数值一致，详见 §1「工具链沿革」。

## 1. 环境（复现前提）

| 项 | 值 | 来源 |
|---|---|---|
| OS | **Windows 11 25H2，build 26200.9457**（`OSVersion=10.0.26200.0`；注册表 `ProductName` 仍显示 "Windows 10 Pro"，属已知显示怪癖，按 build≥22000 判定为 Win11） | `raw/environment.txt`、注册表 |
| 默认输出端点 | `LS24A600N`，48000 Hz / 2ch（设备属性格式 int16；引擎 `GetMixFormat` 实为 float32——见 §5.3） | `raw/probe.log`、`raw/b_control_devmix.run.log` |
| 默认输入端点 | 不存在（`0x80070490`），本机无麦克风——S3 不涉及 | `raw/probe.log` |
| wasapi crate | 0.26.0（crates.io，2026-10-11 发布） | `apploop/Cargo.toml` |
| 音频工具 | ffmpeg/ffplay `N-127564`（仓库内 LGPL 构建） | `raw/environment.txt` |
| apploop.exe | sha256 `033F963AD8403C98B26DFCEE65A7A778B7F8392F0338851904CF75C7F09D284C`（MSVC 构建，1,223,680 B） | `raw/environment.txt` |
| Rust 工具链 | **标准环境**：用户级 rustup + VS Build Tools 2022；仓库根 `rust-toolchain.toml` 锁定 `1.99.0`（与 `stable` 同一发布，构建产物 sha256 一致）。本文全部证据由该环境构建的 exe 产出 | `%USERPROFILE%\.cargo\bin`、vswhere |

工具链沿革（说明，不影响结论）：S3 首轮验证时本机尚无 Rust/MSVC，经批准临时使用了仓库内便携 GNU 工具链（`tools/spikes/s3/toolchain/`；GNU host 下 `windows` crate 的 raw-dylib 需 dlltool，而 rust-mingw 的 dlltool 缺 `as.exe`，见 rust-lang/rust#140704，当时以 MSYS2 binutils 2.46 补齐）。标准环境（用户级 rustup + VS Build Tools 2022）就绪后，**同一源码已用 `stable-x86_64-pc-windows-msvc` 全量重建并重跑矩阵：27/27 通过，关键数值与首轮一致**（a 泄漏 −158.18 dB；b 两向 −158.19/−160.84 dB；路径电平差 −5.41 dB；c 的 QPC 阶跃 16–33 ms 属运行时抖动）。便携工具链自此停用（保留在 gitignore 目录内，不再参与构建）。构建命令（标准环境，无需任何特殊 PATH 处理）：

```
cargo build --release --manifest-path tools/spikes/s3/apploop/Cargo.toml
```

## 2. 方法

- **信号**：ffmpeg 生成 120 s 正弦 WAV（A=440 Hz、B=880 Hz，48 kHz 立体声 f32）；各由一个独立 **ffplay 进程**循环播放到默认输出设备（共享模式）。
- **录制**：`apploop record --pid <PID>`——application loopback（INCLUDE tree）；请求格式 **48 kHz / 2ch / f32**，`StreamMode::EventsShared{ autoconvert: true }`。对每包记录帧数、QPC 时间戳、`SILENT`/`DATA_DISCONTINUITY`/`TIMESTAMP_ERROR` 旗标与 device position；`--pad-silence` 可选地按墙钟补齐到请求时长。
- **FFT（analyze 子命令，rustfft）**：nfft=32768（0.6827 s，bin=1.4648 Hz），Hann 窗，hop=nfft/2，帧间平均功率谱；跳过首尾 0.25 s；目标频带 ±5 Hz 取最大 bin；噪声底=100–6000 Hz 去掉目标 ±10 Hz 后的中位数。dBFS 标定：f32 数字满幅=0 dBFS；本次播放声的绝对电平 ≈ −21.9 dBFS（只影响绝对值，不影响判定）。
- **判定（预锁定）**：参考音存在（rel≈0 dB），非目标音 ≤ **−40 dB**（相对参考音）。另加两项对照：①整机混音（device loopback）必须同时含两声；②同一会话下 device loopback 与 process loopback 的电平差。
- **矩阵**：a 单音隔离；b 双音隔离+混音对照；c 目标停止/无流时的静音与时基行为（自然 + 补静音）；d 双客户端并发（同进程）；e include_tree/exclude_tree 语义（父 cmd + 子 ffplay）；extra：phantom PID、静默 PID、STA/pre-STA 套间、probe。

## 3. 实测结果（a–e）

### 3.1 场景 a：仅播 A，录 A【实测】

证据：`raw/a_onlyA.analysis.log`（8.0 s，384000 帧）

| 项 | 值 |
|---|---|
| 440 Hz（参考） | −21.86 dBFS，峰值 bin 439.45 Hz（bin≈1.46 Hz，无音高偏移） |
| 880 Hz 非目标 | −158.18 dB 相对参考（绝对 −180.04 dBFS） |
| 判定 | **PASS**（阈值 −40 dB） |
| 频谱底 | 100–6000 Hz 中位数 −176.40 dB 相对参考 |

说明：−158 dB 已接近分析器数值底（非目标带内容为数字零），即目标频带之外**测不到任何泄漏**。

### 3.2 场景 b：A+B 同播（核心隔离证据）【实测】

证据：`raw/b_control_devmix.*`、`raw/b_ab_recordA.*`、`raw/b_ab_recordB.*`

| 录音 | 参考音 | 非目标（相对参考） | 判定 |
|---|---|---|---|
| 对照：设备环回（整机混音） | 440 = **−27.45** dBFS；880 = **−26.83** dBFS（差 0.62 dB） | — | 两音**同时存在**✔ |
| 录 A（进程环回） | 440 = −21.86 dBFS | 880 = **−158.19 dB** | **PASS** |
| 录 B（进程环回） | 880 = −21.44 dBFS | 440 = **−160.84 dB** | **PASS** |

对照的意义：设备环回证明两个 ffplay 的声音确实同时混入默认输出（各自可闻）；而两个进程环回录音各自只含自己的音调——**隔离成立**。

补充对照（同一会话、只播 A）【实测】：设备环回 −27.27 dBFS ↔ 进程环回 −21.86 dBFS，**两条路径电平差 −5.41 dB**（`raw/ctl_single_devmix.analysis.log`、`raw/ctl_single_proc.analysis.log`）。成因未进一步验证（推断：设备环回位于端点/会话音量之后，进程环回在其之前）。M4 若跨路径比较电平需注意该偏差。

### 3.3 场景 c：目标停止后的静音行为与时长【实测】

证据：`raw/c_stop_natural.*`、`raw/c_stop_padded.*`、`raw/c_stop_natural.key.log`、`raw/c_stop_padded.key.log`

- 在录制开始后约 3.03 s 强杀 ffplay A（目标进程消失）：
  - **流不断**：包仍以 480 帧/10 ms 节奏供给至录制结束，总帧数 383040（7.98 s，请求 8 s）；
  - `silent_packets=0，data_discontinuity_packets=0，timestamp_error_packets=0`——**没有 SILENT 旗标，也没有 DATA_DISCONTINUITY**；停止后的内容是**数字零**（CSV：t≥3.75 s 的 440/880 块电平均为 −300 dB 钳位值=零）；
  - 停止时刻 QPC 时间戳有一次跳变：`TS STEP delta=+12.605 ms`（1 次 >5 ms），其余节奏 10 ms。
- 内容过渡（`c_stop_natural.csv`，0.5 s/块）：t≤2.75 s 时 440 ≈ −21.3 dBFS → t=3.25 s 过渡块 −78.7 dBFS → t≥3.75 s 数字零。
- 时长保持：
  - 自然模式 7.98 s（墙钟 8.000 s，末包 7.98 s）；补静音模式补 960 帧（0.02 s）后 **8.000 s**（PAD 日志：`added 960 frames (0.020s)`）。
- **M4 结论**：不应把「目标停止」当作「无包/需要检测补帧」——引擎会持续供零；补帧（按 QPC 或末尾填零）成本极低且能把时长与时间轴钉到请求值，建议保留；同时把 ~10–20 ms 的时间戳阶跃记入漂移/对齐监控（S6）。

### 3.4 场景 d：双客户端并发（同一进程）【实测】

证据：`raw/d_dual.console.log`、`raw/d_dual_A.run.log`、`raw/d_dual_B.run.log`

- 单进程内两条采集线程（每线程独立 `initialize_mta()` + 独立 `AudioClient` 生命周期）同时录 A、B：
  - 两个客户端均激活成功（`ACTIVATE ok ×2`），全程无错误；
  - 录 A：880 分量 −158.18 dB；录 B：440 分量 −160.84 dB（均 PASS）。
- 并发上限：本机仅验证 2 路；更高并发与跨设备并发【未验证】。

### 3.5 场景 e：include_tree / exclude_tree 语义【实测】

证据：`raw/e_tree_include.*`、`raw/e_tree_exclude.*`

- 目标=父进程 `cmd.exe`（其子进程为播放 A 的 ffplay）+ 另一个独立播放 B 的 ffplay。
- `include_tree=true` → crate 映射 `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE`：录到**子进程 A 的 440**（−21.86 dBFS），B 泄漏 −158.20 dB → **目标树包含性成立**；
- `include_tree=false` → 映射 `PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE`（注意：**不是「仅目标进程自身」**，而是「除目标树以外的一切」）：录到 B 的 880，A 树（含子进程）泄漏 −160.84 dB。

→ M4 必须按 D9 用「父 PID + INCLUDE 树」捕获应用全部后代；EXCLUDE 语义仅适合反向排除场景。

### 3.6 附加实验（extra）【实测】

| 实验 | 结果 |
|---|---|
| `probe` | MTA 初始化 OK；render 端点格式如上；capture 端点不存在（0x80070490） |
| **Phantom PID**（999999，不存在） | `ActivateAudioInterfaceAsync` **成功**（1.2 ms），随后 8 s 全程零包、无任何错误信号。→ M4 不能靠激活校验 PID；必须自行校验进程存在性/创建时间（D9） |
| **静默 PID**（存在但无渲染会话，powershell sleep） | 仍然 800 包/384000 帧（8.000 s），`silent_packets=0`，峰值 −200 dB（数字零）。→ 「目标无渲染流→静音」成立，但表现是**零数据而非无包** |
| pre-STA（模拟 UI 线程已拥有 STA） | `initialize_sta()` OK 后 `initialize_mta()` 返回 **0x80010106 (RPC_E_CHANGED_MODE)**；其后激活/采集在本控制台场景仍成功。→ 套间冲突可检测；但 UI 线程同步等待采集不可接受，M4 必须用专用 MTA 线程 |
| STA-only | 控制台环境下可激活、可采集（同样不得用于真实 UI 线程） |

## 4. 激活流程与线程/consent 要点（M4 输入）

- **激活方式**【实测+源码审读】（wasapi 0.26.0 `src/api.rs`，`new_application_loopback_client`，约 L809–887）：
  1. 构造 `AUDIOCLIENT_ACTIVATION_PARAMS{ ActivationType: PROCESS_LOOPBACK, TargetProcessId, ProcessLoopbackMode: INCLUDE/EXCLUDE_TARGET_PROCESS_TREE }`，包进 `PROPVARIANT{ vt: VT_BLOB }`；
  2. `ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, IID_IAudioClient, &propvariant, &completionHandler)`；completionHandler 用 `Mutex+Condvar` 等待完成；
  3. `GetActivateResult()` → `result.ok()?` → `IAudioClient`；
  4. `Initialize(SHARED|LOOPBACK|EVENTCALLBACK|AUTOCONVERTPCM|SRC_DEFAULT_QUALITY, 自建格式)` → `IAudioCaptureClient`。
  - 实测激活耗时 1–6 ms；**consent 弹窗：0 次**（本机、普通桌面进程；受策略管控环境【未验证】）。
- **线程套间**：COM 必须在**采集线程**初始化，推荐 MTA；本程序默认 `initialize_mta()`，并把 HRESULT 写入日志（`RPC_E_CHANGED_MODE=0x80010106` 表示线程已有其它套间）。`AudioClient`/`AudioCaptureClient` 为 `!Send`：客户端生命周期必须留在单线程内（本 spike 即「每路一个采集线程」）。
- **格式**：process loopback **无物理端点**，`GetMixFormat()` 返回 `0x80004001 (E_NOTIMPL)`（实测，符合预期）→ 必须自建格式。本次请求 48 kHz/2ch/f32 成功，且 440 Hz 录音峰值 bin=439.45 Hz（无重采样错位）。
- **时间基准**：device position 恒为 0（每包），**不可**用于连续性判断；QPC timestamp 以 10 ms/包推进（`ts_vs_frames_ratio≈0.999–1.001`），偶发 7–20 ms 阶跃（启动/停止/缓冲），M4 需以 QPC 为主时基并按 §3.3 处理阶跃。
- **错误信号一览**（本机实测）：`E_NOTIMPL (0x80004001)`=GetMixFormat 不可用；`RPC_E_CHANGED_MODE (0x80010106)`=套间已占用；phantom PID **无错误**；OS 不支持（如 Win10 旧版）【未验证】。

### 4.1 最小可复用代码路径（M4）

```
initialize_mta();                                   // 采集线程（非 UI 线程）
let mut client = AudioClient::new_application_loopback_client(pid, true)?;   // INCLUDE tree
match client.get_mixformat() { /* 期望失败：process loopback 无 mix format */ }
let fmt = WaveFormat::new(32, 32, &SampleType::Float, 48000, 2, None);
client.initialize_client(&fmt, &Direction::Capture,
    &StreamMode::EventsShared{ autoconvert: true, buffer_duration_hns: 0 })?;
let ev = client.set_get_eventhandle()?;
let cap = client.get_audiocaptureclient()?;
client.start_stream()?;
loop { ev.wait_for_event(100).ok();                                 // EventTimeout=暂无新数据
       while let Some(n) = cap.get_next_packet_size()? { if n==0 {break}
           let (frames, info) = cap.read_from_device(&mut buf)?;    // info.flags.silent/…
           /* SILENT 时缓冲区内容未定义→按静音写入；QPC=info.timestamp */ } }
client.stop_stream()?;
```

完整实现与实验开关（`--exclude-tree`/`--pad-silence`/`--sta`/`--pre-sta`/`--device-loopback`/多目标并发）见 `tools/spikes/s3/apploop/src/main.rs`；用法见 `tools/spikes/s3/apploop/README.md`。

## 5. 风险与未决【未验证】

1. **Win10 2004 (19041) 及更老系统**：本机 Win11 26200，无法验证；官方文档最低版本说法不一（19041 与 20348/20438），按 §9 T2 继续标注「验证中」。
2. **应用形态覆盖**：本次仅覆盖两个独立桌面进程与「父 cmd + 子 ffplay」；UWP/宿主进程、浏览器多窗口/标签、Steam/游戏启动器等复杂进程树未测。
3. **PID 生命周期**：PHANTOM PID 表明激活不校验进程；PID 复用、应用退出后同名重绑（D9 创建时间校验）未做端到端验证。
4. **并发上限**：仅验证 2 路同进程；更高并发、多设备、长时（10 min+）漂移、【设备切换/睡眠】未测。
5. **受保护内容**（DRM）与**独占模式**应用的捕获行为未测（后者为通用已知限制，业界一致，但本机未复核）。
6. **consent 变体**：本机普通桌面无弹窗；组策略/隐私设置管控、非交互会话下的行为未验证。
7. **路径电平差成因**（−5.41 dB）未定论（推断与端点音量阶段有关），M4 跨路径电平对齐（若有）需单独校准。

## 6. 复现

1. 环境：用户级 rustup + VS Build Tools 2022（仓库根 `rust-toolchain.toml` 锁定 `1.99.0`，rustup 自动选用；无需特殊 PATH 处理）。
2. 构建：`cargo build --release --manifest-path tools/spikes/s3/apploop/Cargo.toml`
3. 运行矩阵（会自动生成音调、启停 ffplay、录制、FFT、写证据到 `docs/spikes/s3/raw/`）：
   `powershell -NoProfile -ExecutionPolicy Bypass -File tools/spikes/s3/run_s3_tests.ps1 -Phase all -Seconds 8 -KillAtS 3`
4. 结果：`docs/spikes/s3/raw/_summary.csv` → 本轮 **27/27 通过，0 失败**；控制台全量输出 `raw/_suite_console.txt`。

## 7. 证据索引（docs/spikes/s3/raw/）

| 文件 | 内容 |
|---|---|
| `environment.txt` | OS build、exe 哈希、crate/ffmpeg 版本 |
| `_summary.csv` / `_suite_console.txt` | 27 项断言结果（本轮全绿） |
| `_evidence_fft.txt` | 全部 FFT 摘要（TONE/LEAK/FLOOR/VERDICT） |
| `a_onlyA.*` | 场景 a：录音日志 + FFT + 频谱图 PNG |
| `b_control_devmix.*`、`b_ab_recordA.*`、`b_ab_recordB.*` | 场景 b：混音对照 + 双音隔离（含频谱图 PNG） |
| `ctl_single_devmix.*`、`ctl_single_proc.*` | 设备/进程两条路径电平对照（−5.41 dB） |
| `c_stop_natural.*`、`c_stop_padded.*`（含 `.key.log`） | 场景 c：静音行为、TS 阶跃、补帧时长 |
| `d_dual*` | 场景 d：并发客户端（console + 每目标日志 + 双 FFT） |
| `e_tree_include.*`、`e_tree_exclude.*` | 场景 e：树包含/排除语义 |
| `extra_*` | phantom/静默 PID、pre-STA/STA、probe |
| `probe.log` | COM/端点探测 |
