# S1b Spike 报告：命名管道协议故障注入实测（Screen_rec）

- 日期：2026-10-16（测试当日）
- 前置：S1 已完成（`docs/spikes/s1/s1-pipeline-throughput.md`；harness `tools/spikes/s1/pipebench/`，其管线在本 spike 中原样复刻，S1 代码与报告未改动）。
- 对应验收：S1 报告 §9 未决问题 4（缓冲校准）与 6（慢读/阻塞/取消/EOF 压力，§8「S1 管道/转换/启动」验收中不可用合成帧验证的部分）；M3 管道协议（specs/001 §3.3「命名管道协议」、§3.5 错误路径）的故障面证据。

**结论速览**：【实测】阻塞式整帧 WriteFile 即天然背压——慢读（0.25×）下写等待=消费节拍（avg ~113ms）、240/240 帧零丢失、写侧内存全程平坦（22.7MB 不增长）；对端被 `taskkill /F` 时**被阻塞/下一次 WriteFile 在进程死亡后 ≤13ms 内报 `ERROR_NO_DATA(232)`**，不会卡死写侧；「对端活着但不读」用 **CancelIoEx** 主动解除（写与 ConnectNamedPipe 均验证可行）；阻塞 ConnectNamedPipe 无内建超时、**必须配 CancelIoEx（或 OVERLAPPED 等待）**，取消后 `DisconnectNamedPipe` 原位复用监听实例可行；EOF 关闭是唯一干净的停止路径（ffmpeg 101ms 收尾、exit 0、段完整），`taskkill /F` 的未封口段必烂；24 次 start/stop 无句柄/线程/内存泄漏；缓冲定稿建议 **8MiB**（16MiB 仅把 qsv 冷启动阻塞从 ~600ms 缩到 ~450ms，稳态无差）。

---

## 1. 环境与工具链

- 硬件/OS：与 S1/S2 相同（Windows 11 build 26200；Intel Ultra 7 270K Plus；raw 目录 `environment.txt`）。测试期间无其他录制负载。
- ffmpeg：仓库内 `tools/ffmpeg/ffmpeg-master-latest-win64-lgpl/bin/`（N-127564-g3c92f003c7-20261015，与 S1 同一构建，未改动）。
- Rust：标准工具链 `%USERPROFILE%\.cargo\bin`（rustc/cargo 1.99.0，stable-x86_64-pc-windows-msvc）；依赖 windows 0.61 + fast_image_resize 5.5（cargo registry 本地缓存命中）。
- harness：`tools/spikes/s1b/pipefault/`（独立 crate，不入产品 workspace，`[workspace]` 空表隔离）。管线与 S1 pipebench 完全一致：合成 BGRA 1600×900 → bilinear → 1920×1080 画布（letterbox 黑边）→ BT.601 limited NV12 → 命名管道（Rust=server，`PIPE_ACCESS_OUTBOUND`、BYTE 模式、单实例、`PIPE_REJECT_REMOTE_CLIENTS`、阻塞式 `ConnectNamedPipe` + 逐帧阻塞 `WriteFile`，帧 3,110,400B）→ ffmpeg `rawvideo nv12 1080p30 → h264_qsv -g 60 → segment 5s MKV`。默认缓冲 4MiB（除 bufsz 实验外）。
- 故障注入手段（全部进程内/用户态，无系统设置改动）：ffmpeg `-readrate/-readrate_catchup`（慢读）、Toolhelp32+OpenThread+SuspendThread/ResumeThread（读端挂起/恢复）、`taskkill /F /T /PID`（写端取消）、`CancelIoEx`（连接/写取消）、提前 `CloseHandle`（EOF）。
- 运行方式：`tools/spikes/s1b/run_case.ps1 -Case <case> -Rep <n> -ExtraArgs "<opts>"`，每次运行在 `docs/spikes/s1b/raw/` 留存 `CMD/START/RESULT/PROBE` 文本日志（`.log`）、完整结果 JSON（`.json`）与 ffmpeg stderr 副本（`*_ffmpeg_stderr.log`）。冒烟调试记录保留为 `smoke*.json`（非正式数据）。
- 每配置运行次数：见各节（全部 ≥2 次）。

## 2. 实验 1：慢读背压【实测，两种方法各 2 次】

### 2a. 方法一：`-readrate 0.25 -readrate_catchup 0.25`（持续慢消费，240 帧=8s@30fps）

命令原文（driver 日志 `s1b_slowread_rep*.log`）：
`pipefault --case slowread --label s1b_slowread_rep1 --seconds 8 --readrate 0.25 --readrate-catchup 0.25`
（ffmpeg 侧：`ffmpeg -hide_banner -nostdin -readrate 0.25 -readrate_catchup 0.25 -f rawvideo -pix_fmt nv12 -s 1920x1080 -framerate 30 -i \\.\pipe\screenrec_s1b_<label> -c:v h264_qsv -g 60 -f segment -segment_time 5 -reset_timestamps 1 -segment_format matroska -y <tmp>\out_%05d.mkv`）

| rep | 写完全部帧耗时 | 迟帧最大 | 迟帧数 | 写等待 avg/p95/max (ms) | 段完整性 | ffmpeg 退出 | 写侧私有内存 |
|---|---|---|---|---|---|---|---|
| 1 | 29.25s | **21,248ms** | 237/240 | 113.6 / 131.6 / 640 | 240/240 包，exit 0 | 0 | 8.7→22.7MB 平坦 |
| 2 | 29.25s | **21,248ms** | 237/240 | 113.5 / 131.1 / 613 | 240/240 包，exit 0 | 0 | 8.7→22.6MB 平坦 |

- ffmpeg stderr 实测输入速率 `speed=0.256x–0.268x`（目标 0.25×），写侧等效吞吐 240/29.25 = 8.2 帧/s —— **写节奏被读端完全限住**：稳态写等待 ≈ 1/7.5fps−本帧生成时间 ≈ 113ms，与实测一致。
- **背压持续且可检测**：迟帧单调增长至 21.2s；任何合理阈值（如 slot 超 500ms 或连续 N 帧迟）都会在背压发生后 ~1s 内触发。前 3 帧（管道+ffmpeg 输入队列吸收 ~4 帧）不迟，之后持续迟。
- **积压/内存有界**：写侧无队列（帧直接 WriteFile），250ms 采样私有内存全程平坦（22.7MB 为初始缓冲常量）；积压仅存在于管道系统缓冲（4MiB≈1.35 帧）+ ffmpeg `thread_queue`（默认 8 包，精确吸收量未单独测量【未验证】）。
- **零丢失**：写完全部 240 帧后 EOF，ffmpeg 慢速消化完所有帧（总墙钟 29.7s），240/240 包完整、段可读、exit 0 —— 慢消费不丢帧，只拖长录制实时进度。

### 2b. 方法二：读端线程挂起 3s（硬停顿，300 帧=10s@30fps）

命令原文：`pipefault --case suspend-resume --label s1b_suspend-resume_rep1 --seconds 10 --suspend-at 4 --resume-after 3`

| rep | 迟帧最大 | 迟帧数 | 写等待 max (ms) | 段完整性 | ffmpeg 退出 |
|---|---|---|---|---|---|
| 1 | 2,905ms | 143/300 | 2,931 | 300/300 包，exit 0 | 0 |
| 2 | 2,888ms | 140/300 | 2,915 | 300/300 包，exit 0 | 0 |

- t=4s 挂起 ffmpeg 全部 45 个线程，写侧在下一次 WriteFile 即阻塞（叠加初始 qsv 冷启动，`blocked_first_frame=3`；停顿期单次写等待最大 2.93s ≈ 3s 挂起 − 缓冲余量 ~70ms）。
- 恢复后写侧立即排空积压并回到稳态（后续帧按时隙完成），300/300 帧完整 —— **短暂停顿只表现为一次性迟帧峰，不丢帧、不累积**。
- 写侧内存峰值 22.7/22.8MB，与无故障运行相同。

## 3. 实验 2：写端取消（录制中 `taskkill /F` ffmpeg）【实测，各 2 次】

命令原文：`pipefault --case kill --label s1b_kill_rep1 --seconds 10 --kill-at 4.5`；变体 `--case kill-stalled ... --kill-at 4.5 --suspend-before-kill 1.2`（先把读端挂起 1.2s，使写侧**已阻塞在 WriteFile 中**时进程被杀）。

| rep | 场景 | 写侧报错 | 检测时延（进程死亡→写返回） | taskkill 开销 | 死亡前写入帧数 | 段结果 | ffmpeg 退出 |
|---|---|---|---|---|---|---|---|
| 1 | 读端活跃 | `ERROR_NO_DATA (232)` | **−0.008s（轮询分辨率 ±13ms，即时）** | 1,793ms | 190 | 1 段未封口：97 包可读（写入 190 帧）、无有效时长 | 1 |
| 2 | 读端活跃 | `ERROR_NO_DATA (232)` | **+0.003s** | 1,858ms | 192 | seg0 恰在死亡前越过 5s 边界（关键帧对齐裁剪至 6s）封口：180 包+估算时长 6.067s；seg1 空壳不可读 | 1 |
| 1 | **写阻塞中被杀** | `ERROR_NO_DATA (232)` | **−0.007s** | 1,575ms | 109 | 1 段未封口：49 包可读（写入 109 帧）、无有效时长 | 1 |
| 2 | **写阻塞中被杀** | `ERROR_NO_DATA (232)` | **−0.013s** | 1,629ms | 110 | 1 段未封口：49 包可读（写入 110 帧）、无有效时长 | 1 |

关键结论：

1. **写侧永不卡死**：无论写侧此刻是空闲还是在阻塞写，对端进程死亡后内核立即让 WriteFile 返回错误（检测时延 ≤13ms ≈ 轮询分辨率）。「被阻塞的写在对端死亡后解除」这一性质意味着**纯死亡场景无需写超时兜底**。
2. **错误码实测为 `ERROR_NO_DATA(232)`**（非 `ERROR_BROKEN_PIPE(109)`）；产品错误映射应以 232 为主、109 兼容（不同断开时序可能产生 109，本次 4 次均为 232）。
3. **`taskkill /F` 自身开销 1.6–1.9s**（进程树枚举等），期间写侧继续写入 50–57 帧才收到错误——按「下发 kill」计时会把检测时延误判为 ~1.7s；产品进程内 `TerminateProcess` 无此开销（未单独测【未验证】）。
4. **段封口**：被杀时刻未封口的段必然未写 duration/Cues——表现为 ffprobe 可线性读出部分包（本次 49–180 包，对应写入 109–192 帧，数量随机，疑似收尾簇被截断）但无有效时长；恰在死亡前越过 5s 段边界（关键帧对齐裁剪到 6s）时，已封口段完全可读（rep2 seg0 180 包+估算时长）。**已封口段完好、未封口段按 `E_SEGMENT_CORRUPT` 处理**，与 S5 实验 1 结论一致（concat 收尾需剔除烂段）。
5. ffmpeg 被 `taskkill /F` 终止的退出码实测为 1（Rust `try_wait` 视角）。

## 4. 实验 3：连接超时（spawn 后客户端不连接）【实测，2 次】

命令原文：`pipefault --case connect-timeout --label s1b_connect-timeout_rep1 --cancel-connect-after-ms 5000`
（诱饵进程：`ffmpeg -hide_banner -nostdin -f lavfi -i testsrc2=size=1920x1080:rate=30 -t 60 -c:v h264_qsv -y <tmp>\decoy_%05d.mkv`——模拟「sidecar 启动后从不打开管道」，qsv 60s 编码正常退出 exit 0，证明诱饵真实运行且从未碰管道。）

| rep | 阻塞 ConnectNamedPipe 行为 | 取消 | 原位复用 |
|---|---|---|---|
| 1 | **阻塞满 5.000s**（无客户端不返回，无内建超时） | `CancelIoEx` 立即解除，错误码 `0x800703E3`（=HRESULT_FROM_WIN32(`ERROR_OPERATION_ABORTED` 995)） | `DisconnectNamedPipe`（未连接实例）→ ok；新 ffmpeg 15–18ms 完成连接；30 帧+EOF → exit 0 |
| 2 | 同上（5.005s） | 同上 | 同上（15ms，exit 0） |

结论：

1. **阻塞式 `ConnectNamedPipe` 对「永不连接」没有超时**——若不处理，启动序列会无限期卡死（这正是「启动期卡死」的直接成因）。**CancelIoEx 是必要且充分的取消手段**：另一线程对同一 HANDLE 调用即可让阻塞中的连接调用立即返回 995。
2. 取消后的**原位复用可行**：对「取消时尚无客户端」的监听实例先 `DisconnectNamedPipe`（实测返回 ok），再次 `ConnectNamedPipe` 能正常接受新客户端并完成一次完整写/EOF/退出。备选路径「关闭句柄后同名重建管道」本次未触发（复用成功），**未验证**【未验证】。
3. 备选方案 OVERLAPPED+事件+带超时等待未实测【未验证】；两者效果等价（都能给连接加超时），产品可任选，CancelIoEx 方案改动最小（与写取消共用同一机制）。
4. 启动顺序数据：真实 ffmpeg 客户端连接耗时 15–29ms（全部实验一致），远小于超时预算；qsv 初始化（0.6–0.8s）发生在连接之后、表现为首写阻塞（见 §6），不占连接超时预算。

## 5. 实验 4：关闭语义与重复启停

### 5a. writer 提前 EOF（90 帧=3s 后关写端）【实测，2 次】

命令原文：`pipefault --case eof --label s1b_eof_rep1`

| rep | EOF→ffmpeg 退出 | ffmpeg 退出码 | 段结果 |
|---|---|---|---|
| 1 | 0.101s | **0** | 1 段：duration 3.067s、90/90 包、h264 |
| 2 | 0.101s | **0** | 1 段：duration 3.067s、90/90 包、h264 |

**EOF 是唯一干净的停止路径**：关闭写句柄后 ffmpeg 立即收尾（~100ms），未满 5s 的当前段被正确封口（时长 3.067s=90 帧/30fps），退出码 0。对比 §3 的 taskkill：语义差异完全来自「封口 vs 不封口」。

### 5b. 重复 start/stop ×12（×2 reps = 24 次）【实测】

命令原文：`pipefault --case restart --label s1b_restart_rep1 --restart-iters 12`
每次迭代完整走：`CreateNamedPipeW` → spawn ffmpeg → 连接（实测 19.8–24.0ms）→ 定速写 45 帧（1.5s）→ EOF → 等 ffmpeg 退出 → ffprobe 校验 → 清理。

| 指标 | before | mid(6 次) | after(12 次) |
|---|---|---|---|
| 写侧句柄数 rep1/rep2 | 60/60 | 68/68 | 66/66 |
| 写侧线程数 rep1/rep2 | 4/4 | 4/4 | 4/4 |
| 私有内存 rep1/rep2 | 0.6/0.6MB | 0.8/0.7MB | 0.8/0.7MB |
| 系统 ffmpeg 进程数 | 0/0 | — | 0/0 |

- **24/24 次迭代全部完美**（45/45 包、duration 1.567s、exit 0、连接 19.8–24.0ms、无一次强杀）。
- 句柄数 60→68→66 非单调、无逐迭代增长趋势（波动来自 tasklist/ffprobe 子进程瞬时句柄等，未逐个定位【未验证】）；线程数与内存平坦；**无僵尸 ffmpeg**。本粒度下无资源泄漏。

## 6. 实验 5：管道缓冲 8MiB vs 16MiB（qsv 冷启动吸收）【实测，各 2 次】

命令原文：`pipefault --case bufsz --label s1b_bufsz_rep1 --seconds 10 --pipe-buf 8388608`（16MiB 同法 `--pipe-buf 16777216`，rep3/rep4）；对照 S1 的 4MiB 数据（paced 30fps，同管线）。

| 缓冲 | rep | 吸收帧（>500ms 阻塞前） | 冷启动期迟帧 | 迟帧最大 | 写等待 max | 稳态写等待 avg/max (ms) | 完整性 |
|---|---|---|---|---|---|---|---|
| 8MiB | 1 | 4 帧（第 5 写起阻塞） | 23 | 588ms | 615ms | 0.54 / 1.19 | 300/300，exit 0 |
| 8MiB | 2 | 4 帧 | 25 | 610ms | 636ms | 0.56 / 2.80 | 300/300，exit 0 |
| 16MiB | 3 | ≥5 帧（单次阻塞 <500ms） | 18 | 466ms | 490ms | 0.52 / 1.33 | 300/300，exit 0 |
| 16MiB | 4 | ≥5 帧 | 16 | 427ms | 454ms | 0.50 / 1.30 | 300/300，exit 0 |
| 4MiB（S1 §3b 对照） | ×3 | 1–2 帧 | 23–25 | — | 首写阻塞 620–664ms | — | 300/300，exit 0 |

- **缓冲大小只能修边、不能吸收冷启动**：qsv 初始化 0.6–0.8s ≈ 19–24 帧 ≈ 59–75MB，远超任何档位；写阻塞（冷启动的本质）在所有档位都发生，缓冲只决定「阻塞前先吸收几帧」。
- 16MiB 相对 8MiB 的实测收益：迟帧 23–25 → 16–18（少 ~7 帧），单次阻塞 588–636ms → 427–490ms（少 ~145ms）。稳态（init 之后）三档完全一致（avg 0.50–0.56ms）。
- 慢读/停顿场景（§2）下缓冲档位差异会等比放大吸收窗（8MiB≈2.7 帧≈90ms），但均不改变「零丢失」结论（4MiB 已验证）。

## 7. M3 管道协议建议（汇总）

1. **写模型**：维持「阻塞式整帧 WriteFile」——它同时是流量控制（背压）与完整性机制。慢读/停顿下无丢失、内存有界、迟帧可测；无需 OVERLAPPED 复杂化（与 S1 §6 结论一致，并在故障面复证）。
2. **迟帧/停滞告警**：写侧逐帧记录 slot 迟迟（S1 已有）+ 写等待时长；建议两级：①迟帧持续超阈值 → 告警（`route_level`/告警通道）；②写等待 > 档位（如 3–5s，即「对端活着但不读」）→ 判定 `E_PIPE_STALL`，走主动取消停该路。
3. **写超时/取消机制**：死亡类故障无需超时（内核即时解除，232/109）；「活但不读」必须主动取消——watcher 线程对写句柄 `CancelIoEx`（实测解除阻塞、返回 995、可干净收尾）。连接阶段同理：`ConnectNamedPipe` 必须配超时+取消（无内建超时；CancelIoEx 已验证，OVERLAPPED 等待为等价备选【未验证】）。
4. **错误码映射（实测）**：`232 ERROR_NO_DATA` / `109 ERROR_BROKEN_PIPE` → 对端退出（`E_SIDECAR_EXIT`），保留已封口段；`995 ERROR_OPERATION_ABORTED` → 自主取消路径（`E_PIPE_STALL` 停路）；`535 ERROR_PIPE_CONNECTED` → 客户端抢先连接（正常成功）。
5. **启动顺序**：create pipe → spawn sidecar → **带超时（建议 ≥5s）的 ConnectNamedPipe**（实测连接 15–29ms；qsv init 不占连接预算）；超时后可 `DisconnectNamedPipe` 原位复用监听实例重试（已验证），或销毁重建（未验证备选）。冷启动首写阻塞 0.6–0.8s 按 S1 §6 计入「冷启动窗口」，不计入停滞判定。
6. **停止协议**：**优雅停止只走 EOF**（关闭写句柄，ffmpeg ~100ms 收尾、exit 0、段完整）；`taskkill /F` 仅作为崩溃恢复兜底，其未封口段必烂（可读包数随机、无时长）→ 标 `E_SEGMENT_CORRUPT` 并从 concat 清单剔除（S5 §5）。注意 `taskkill` 外部命令开销 1.6–1.9s，产品用进程内 TerminateProcess。
7. **缓冲定稿：8MiB（视频管道）**。16MiB 的唯一收益是冷启动期 ~145ms/7 帧（且冷启动本就该被排除统计），稳态零差异；音频 f32le 管道（384KB/s）单独设小缓冲（本次未测【未验证】）。
8. **资源管理**：管道句柄/线程随启停正确回收（24 次启停零泄漏、零僵尸）；server 侧关闭句柄即 EOF 的语义已验证，收尾路径无需额外协议帧。

## 8. 未决问题

1. **OVERLAPPED 连接等待**（事件+带超时 WaitForSingleObject）未实测；与 CancelIoEx 方案二选一即可，均已论证可行。
2. **取消后「销毁重建管道」备选路径**未触发（原位复用成功被跳过），未验证。
3. **`CancelIoEx` 中止写时的半帧字节语义**（已传输字节数不确定）未测量：本次两次 ffmpeg 均容忍（exit 0、包数=完整帧数），但产品停滞收尾应 kill sidecar，不应依赖对端容忍 EOF 半帧。
4. **慢读时管道+ffmpeg `thread_queue` 的精确吸收量**未测量（观察 ~3–4 帧超前不迟）；有界性由构造保证+内存平坦佐证。
5. **音频管道（f32le）故障行为**未测（归 S5/S6）；**4KiB~16MiB 之外档位**与 `-thread_queue_size` 调优未测。
6. **进程内 TerminateProcess 与 taskkill 的写侧行为差异**未单独测（内核层语义相同，预期一致）。
7. 单机（Win11 26200/24 核）单配置结果；Win10/最低硬件矩阵未验证。
8. 冒烟期间发现 `-readrate 0.25` 不带 `-readrate_catchup` 时 ffmpeg 会以追赶速度（~11fps）消化积压——正式实验已加 `-readrate_catchup 0.25` 封顶；该追赶行为本身对产品「欠账追赶」策略有参考价值，未深入。

## 9. 产物清单

- 报告：`docs/spikes/s1b/s1b-pipe-faults.md`（本文件）
- crate：`tools/spikes/s1b/pipefault/`（`Cargo.toml`、`src/main.rs`；`target/` 由既有 `target/` gitignore 规则覆盖）
- 脚本：`tools/spikes/s1b/run_case.ps1`（单用例驱动，CMD/START/RESULT/PROBE 留痕）
- 原始数据（`docs/spikes/s1b/raw/`，每 run 三件套 `.log/.json/*_ffmpeg_stderr.log`，诱饵另存 `*_decoy_stderr.log`）：
  - 慢读背压：`s1b_slowread_rep{1,2}.*`
  - 停顿/恢复：`s1b_suspend-resume_rep{1,2}.*`
  - 停顿/取消：`s1b_suspend-cancel_rep{1,2}.*`
  - 写端取消：`s1b_kill_rep{1,2}.*`、`s1b_kill-stalled_rep{1,2}.*`
  - 连接超时：`s1b_connect-timeout_rep{1,2}.*`
  - EOF：`s1b_eof_rep{1,2}.*`
  - 重复启停：`s1b_restart_rep{1,2}.{log,json}`
  - 缓冲对比：`s1b_bufsz_rep{1,2}.*`（8MiB）、`s1b_bufsz_rep{3,4}.*`（16MiB）
  - 环境：`environment.txt`；调试冒烟：`smoke*.json`（非正式数据）
- 编码输出样本：`%TEMP%\opencode\s1bout\`（临时目录，不入库）
- 未改动：`tools/spikes/s1/`（pipebench 与报告）、`docs/spikes/s1|s2|s3|s5/`、`tools/ffmpeg/`
