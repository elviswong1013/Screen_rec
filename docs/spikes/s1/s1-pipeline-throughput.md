# S1 Spike 报告：命名管道实时管线吞吐 + 固定画布缩放成本

- 日期：2026-10-16（测试当日）
- 前置：S2 已完成（`docs/spikes/s2/s2-encoder-matrix.md`）；本报告附其未决问题①的复测结论（§7）。
- 对应验收：specs/001-screen-recorder.md §8「S1 管道/转换/启动」——无启动死锁；达目标分辨率/FPS；帧字节完整；可停止；记录转换/复制/管道成本。本 spike 覆盖其中可用合成帧验证的部分（真实 WGC 帧归 S4/M3；取消/慢读/阻塞压力归后续 spike）。

**结论速览**：全链（合成 BGRA→缩放→NV12→命名管道→ffmpeg h264_qsv→5s 分段 MKV）单路 30fps **稳态零迟帧**（每帧链路成本 ~6.6ms，预算 33.3ms，余量 ~5x）；双路、三路 30fps 均完整跑通（帧数 300/300 每路、无持续积压，仅存一次性瞬时调度尖峰 ≤263ms）；缩放净成本 **~1.0–1.2ms/帧**【实测】；管道方案定稿建议见 §6；**h264_nvenc 在 n8.1 release 构建上本机可用**（§7），S2 的失败确认为 master 构建头文件问题。

---

## 1. 环境与工具链

- 硬件/OS：与 S2 相同（Windows 11 25H2 build 26200.9457；Intel Ultra 7 270K Plus 24C/24T；Intel iGPU + RTX 5070 Ti 驱动 596.36；VirtualMonitor 虚拟适配器排首）。
- ffmpeg：S2 下载的 BtbN `ffmpeg-master-latest-win64-lgpl`（版本 N-127564-g3c92f003c7-20261015，SHA256 `F899D80C…E06C3E8`），全部调用使用 `tools/ffmpeg/ffmpeg-master-latest-win64-lgpl/bin/` 绝对路径；未改动。
- **Rust 工具链（环境阻塞与解决）**：本机原无 rustc/cargo/MSVC-vswhere/MinGW，且约束禁止安装系统软件。采取：`rustup-init`（用户级）将工具链解包至**仓库内** `tools/rust/`（RUSTUP_HOME/CARGO_HOME 均指向仓库内，`--no-modify-path` 不改任何系统设置；`tools/rust/` 已加入 .gitignore）。GNU 目标因 rust-mingw 缺 `as.exe` 导致 dlltool 失败（binutils 依赖链问题，详见 §9 问题 8）；最终使用 `stable-x86_64-pc-windows-msvc`（本机存在 MSVC BuildTools 2022 14.44 + Windows SDK 26100，rustc 1.99.0 编译）。
- 测试期间使用 `subst X:` 盘符别名规避路径空格问题（会话级映射，已 `subst /D` 移除）。
- pipebench：`tools/spikes/s1/pipebench/`（独立 crate，不入产品 workspace；依赖仅 `windows` 0.61 + `fast_image_resize` 5.5）。

## 2. 管线实现（pipebench）【实测】

每路一个 worker 线程，复刻 v1 架构：

```
合成 BGRA 帧(1600x900) → fast_image_resize Bilinear → 固定画布 1920x1080（letterbox 黑边=RGB16,16,16）
  → 手工 BGRA→NV12（BT.601 limited，2x2 均色度） → WriteFile 整帧阻塞写 → 命名管道 → ffmpeg
ffmpeg: -f rawvideo -pix_fmt nv12 -s 1920x1080 -framerate 30 -i \\.\pipe\screenrec_s1_<label>_r<N>
        -c:v h264_qsv -g 60 -f segment -segment_time 5 -reset_timestamps 1 -segment_format matroska out_%05d.mkv
```

- 命名管道（Rust=server）：`CreateNamedPipeW`（windows-rs 0.61），`PIPE_ACCESS_OUTBOUND`、字节模式、`PIPE_REJECT_REMOTE_CLIENTS`、单实例、**in/out 缓冲各 4MiB**（≈1.35 帧 NV12）；同步阻塞 `ConnectNamedPipe` 等 ffmpeg 连接，再逐帧 `WriteFile`（3,110,400 B/帧）。
- 定速：paced 模式按 `t0 + i/fps` 调度写完成时刻，记录每帧 late（超过槽位 deadline 的毫秒数）；freerun 模式不睡眠测最大吞吐。
- 阶段计时：gen（合成专用）/resize（含 letterbox compose 拷贝）/convert/pipe_write 各自独立统计（avg/p50/p95/max）。
- 完整性校验：每段 ffprobe（duration + nb_read_packets + codec_name）；进程退出码。
- 变体：`scale`（1600x900→1080p，16:9 无黑边）、`noscale`（1920x1080 直转）、`letterbox`（1600x1000→1728x1080 居中，左右各 96px 黑边）。

## 3. 测量矩阵与结果

### 3a. 纯 ffmpeg 基线（testsrc2 1080p30 ×10s，`-g 60`，5s 分段 MKV）【实测，每配置 3 次】

脚本：`tools/spikes/s1/run_baseline.ps1`；命令原文（各进程）：
`ffmpeg -hide_banner -nostdin -f lavfi -i testsrc2=size=1920x1080:rate=30 -t 10 -g 60 -c:v <enc> -f segment -segment_time 5 -reset_timestamps 1 -segment_format matroska -y <dir>\out_%05d.mkv`

| 编码器 | 并发 | 超实时倍数（3 次 reps，多进程分别计） | 帧完整性 |
|---|---|---|---|
| h264_qsv | 1 | 5.88x / 4.37x / 5.94x | 300/300，2 段，Σ10.067s |
| h264_qsv | 2 | 4.68x,4.58x / 3.99x,4.11x / 4.16x,4.05x | 300/300 ×2 路 |
| h264_qsv | 3 | 3.17–3.86x（9 个进程值区间） | 300/300 ×3 路 |
| h264_mf | 1 | 14.8x / 13.0x / 15.5x | 300/300，2 段，Σ10.000s |
| h264_mf | 2 | 8.41–9.30x | 300/300 ×2 路 |
| h264_mf | 3 | 6.51–6.94x | 300/300 ×3 路 |

全部 exit=0；单编码器三路并发仍超实时 3 路以上——编码环节不是瓶颈。

### 3b. 全链单路（scale 变体，10s）【实测，各 3 次】

| 模式 | 帧数 | 稳态 late_max | 稳态超预算帧 | write_wait(avg/p95/max) | 管道速率 | ffmpeg CPU | 退出 |
|---|---|---|---|---|---|---|---|
| paced 30fps ×3 | 300/300 ×3 | **0.00ms ×3** | **0 ×3** | 0.55–0.59 / 0.91–0.97 / 1.26–1.48ms | 92–93MB/s | 0.15–0.16 核 | 0，2 段 300 包 |
| freerun ×3 | 300/300 ×3 | —（不限速） | — | 0.41–0.46ms avg | 349–366MB/s | 0.43–0.55 核 | 0 |

- **自由跑最高 fps：116.6 / 121.4 / 122.3 fps（≈4.1x 实时）**——30fps 目标的 ~4x 吞吐余量。
- 阶段成本（paced avg）：gen 3.30–3.37ms（合成专用）+ resize 1.24–1.49ms + convert 1.72–1.79ms + write ~0.55ms ≈ **6.6ms/帧**（预算 33.3ms）。
- **启动时延**：管道创建→ffmpeg 连接（即首读前一刻）**~29ms**；首帧写完成 ~37ms。qsv 会话初始化使首写阻塞 **620–664ms**（3 次），表现为前 ~23–25 帧迟（之后稳态零迟帧）——这是**冷启动一次性成本**，运行期无积压。
- CPU：pipebench 进程 0.20–0.21 核（其中管线 worker 线程单线程占用 ~21%）；`system_busy_pct` 36–45%（桌面机含其他进程，仅参考）。

### 3c. 全链双路 + 第三路探测（paced 30fps ×10s）【实测，各 3 次】

| 配置 | 帧完整性 | 稳态迟帧（每路） | 稳态迟帧瞬时最大 | write_wait(avg) | pipebench CPU |
|---|---|---|---|---|---|
| 双路 ×3 | **600/600 全部 ✓** | 0–4 帧 | 0–85.7ms（一次性） | 0.65–0.71ms | 0.43–0.45 核 |
| 三路 ×3 | **900/900 全部 ✓** | 1–11 帧 | 85–263ms（一次性尖峰，无累积） | 0.83–0.91ms | 0.70–0.76 核 |

- **三路 30fps 稳定成立**：每路 300/300 帧无丢失、分段完整、退出码 0；迟帧为孤立调度尖峰（Windows 线程调度 + 3×qsv init 竞争期），迟后立即恢复，**无持续积压**（10s 内迟帧不随时间增长）。
- 对比 §3b：双/三路的 per-frame 阶段成本几乎无退化（resize 1.4→2.2ms、convert 1.8→2.0ms），写等待仅 +0.3ms——**CPU 余量巨大**（三路全链不足 1 核 pipebench + 3×0.15 核 ffmpeg）。
- NVENC 式「第三路失败」场景在本机未出现（qsv 会话上限未触及）。

### 3d. 缩放成本对比（freerun 单路，各 3 次 + letterbox 1 次）【实测】

| 路径 | 生成(gen) | 缩放(resize) | 转换(convert) | 合计（不含 gen） |
|---|---|---|---|---|
| 无缩放直转（1920x1080 直转 NV12） | 3.93–4.04ms | — | 1.62–1.68ms | ~1.65ms |
| 缩放路径（1600x900→1080p bilinear→NV12） | 2.76–2.82ms | **1.01–1.18ms** | 1.59–1.66ms | ~2.7ms |
| letterbox（1600x1000→1728x1080+compose+黑边→NV12） | 3.71ms | 1.42ms（含 compose 拷贝） | 1.74ms | ~3.2ms |

- **缩放净成本 ≈ 1.0–1.2ms/帧**（bilinear，fast_image_resize 5.5 SIMD）；letterbox compose 拷贝再加 ~0.2–0.4ms。对 1080p30 预算（33.3ms）占比 ~3–4%。
- letterbox 黑边已用 signalstats 验证：左侧栏 YAVG=**16.0**（精确 studio black），中心内容 YAVG≈138.5。
- **CPU 路径实测；D3D11 VideoProcessor GPU 路径未验证**（见 §6 建议）。

## 4. 阶段成本汇总（1080p30 预算 33.3ms/帧）

| 阶段 | 实测（avg） | 备注 |
|---|---|---|
| 合成 gen（测试专用） | 2.8–4.0ms | 产品无此阶段 |
| 缩放 1600x900→1920x1080（bilinear） | 1.0–1.5ms | fast_image_resize SIMD |
| letterbox compose（有黑边时） | +0.2–0.4ms | 整行 memcpy |
| BGRA→NV12 转换（BT.601 limited，纯 Rust） | 1.6–2.1ms | 1920x1080 canvas |
| 管道写（阻塞，整帧 3,110,400B） | 0.4–0.9ms 稳态 | 1/2/3 路分别约 0.5/0.7/0.9ms |
| ffmpeg h264_qsv（每进程 CPU） | 0.13–0.19 核（30fps 定速） | 编码环节自身超实时 ≥3.2x |

**结论**：CPU 全链单路 ~8ms/帧、三路合计 <1.8 核——24 核机器余量 >10x；30fps 三路为「轻松区间」，真正的实时风险在捕获端（WGC）与音频，不在本链路。

## 5. 启动行为与背压【实测】

- 时序：pipe 创建 → spawn ffmpeg → **29ms 后 ffmpeg 连接管道** → 首帧写（37ms）→ qsv 初始化期写阻塞 ~0.6–0.8s（一次性）→ 稳态零积压。
- 4MiB 缓冲 ≈ 1.35 帧：不足以吸收 init 期，写阻塞即自然背压，**无帧丢失**（rawvideo `-framerate 30` 由 ffmpeg 定时打戳，迟到帧表现为晚写而非丢弃；段 duration/packets 完整）。
- 无启动死锁：所有 run（含三路同时启动）均正常连接、编码、收尾退出。

## 6. 管道与缩放方案定稿建议

**管道**：
1. 维持「Rust=server、BYTE 模式、单实例、阻塞写整帧、`PIPE_REJECT_REMOTE_CLIENTS`」。阻塞写在实测中即天然背压，无需有界队列即可保证实时性（消费不过来时写侧先感知）；产品可在其上加「迟帧告警」而非丢帧。
2. 缓冲取 **8MiB**（规格 4–16MiB 中点）：4MiB 实测已够（1.35 帧），8MiB（2.7 帧）可多吸收 ~90ms 抖动，代价可忽略；不建议 <4MiB。
3. 启动协议建议：spawn sidecar → `ConnectNamedPipe` 返回（连接）即开始供帧；把 qsv/mf 初始化期（~0.6–0.8s）的写阻塞视为**冷启动**并从积压统计中排除（本 spike 已区分）。可选优化【未验证】：先写静音/黑帧预热或监听 ffmpeg stderr 首条 `Input #0` 再启动真实采集。
4. 逐帧 3.11MB 阻塞 WriteFile 的系统调用开销可忽略（0.4–0.9ms）；无需 `OVERLAPPED` 复杂化（S1 范围结论）。

**缩放/转换**：
1. v1 采用 **CPU 路径：fast_image_resize(Bilinear/U8x4) + 手工 BGRA→NV12**——合计 ~2.5–3ms/帧，预算余量 10x，实现简单、无 GPU 依赖（无独显/核显异常机器也成立）。
2. D3D11 VideoProcessor（GPU scaling+CSC，一步出 NV12）：**可行但未验证**。windows-rs 可达（ID3D11VideoDevice/VideoProcessorEnumerator/VideoProcessorBlt，驱动线程零 CPU）；瓶颈在于输出 NV12 后仍需回读系统内存喂管道（staging Map ~0.5–1ms【未验证】），或改用 d3d11 共享句柄直喂 ffmpeg 的 `-init_hw_device d3d11va`（复杂度高、h264_qsv 侧兼容性需专项验证）。**v1 不采用；列为 CPU 路径饱和后的优化项。**

## 7. 附录：S2 未决①——h264_nvenc 在 n8.1 release 构建上的复测【实测】

- 资产：`ffmpeg-n8.1-latest-win64-lgpl-8.1.zip`（BtbN latest release，170,540,858 B，SHA256 `337B85465B9CD0CFCB3376B84988912C3999E3FB69BC025F0B4360B4B1C8C240`），解包于 `tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/`（gitignored，未改动 master 构建）。
- `-h encoder=h264_nvenc`：exit 0（`raw/nvenc_n81_help.txt`）。
- 10s 短编码（`-f lavfi -i testsrc2=size=1920x1080:rate=30 -t 10 -g 60 -c:v h264_nvenc -y <out>.mkv`）：**exit 0，0.94s（10.6x 实时），2,835,026 B，300 帧完整，duration 10.000000**（`raw/nvenc_n81_shortencode.log`）。
- **结论**：本机驱动 596.36（NVENC API 13.0）与 n8.1 release 构建完全兼容；S2 的 NVENC 失败确认为 master-latest 构建所用 nv-codec-headers 13.1（要求驱动 ≥610.00）所致。**产品 sidecar 应钉住 release 分支构建（如 n8.1），推荐回退链首 h264_nvenc 在本机成立**；旧驱动会话上限问题仍未测（见 §9）。

## 8. 产物清单

- 报告：`docs/spikes/s1/s1-pipeline-throughput.md`（本文件）
- crate：`tools/spikes/s1/pipebench/`（`Cargo.toml`、`src/main.rs`；`target/` 由既有 .gitignore `target/` 规则覆盖）
- 脚本：`tools/spikes/s1/run_baseline.ps1`
- 原始数据（`docs/spikes/s1/raw/`）：
  - 基线日志 ×36：`baseline_h264_{qsv,mf}_{1,2,3}x_rep{1..3}_p*.log`（含 CMD 原文/RESULT/PROBE）
  - pipebench JSON：`s1_paced_r{1..3}.json`、`s1_freerun_r{1..3}.json`、`s1_dual_r{1..3}.json`、`s1_triple_r{1..3}.json`、`s1_noscale_free_r{1..3}.json`、`s1_letterbox_r1.json`（含每路全部统计与分段 ffprobe）
  - 冒烟记录：`smoke{,2..5}.json`（脚本调试过程，非正式数据）
  - nvenc 附录：`nvenc_n81_version.txt`、`nvenc_n81_help.txt`、`nvenc_n81_shortencode.log`
- 编码输出样本：`%TEMP%\opencode\s1out\`（临时目录，不入库）

## 9. 未决问题

1. **freerun 上限 ~120fps 的构成**：gen+convert 为合成专用成本（产品无 gen），真实帧源下的链路上限归 S4/M3 实测；本 spike 不下结论。
2. **三路瞬时迟帧尖峰（85–263ms，一次性、无累积、无丢帧）**来源未定位（线程调度/qsv init 竞争/驱动）。产品侧缓解（独立进程 per route、线程优先级、启动预热）未验证。
3. **D3D11 VideoProcessor 路径**未实现未验证（§6 建议：v1 不采用）。
4. **8/16MiB 管道缓冲对比**未做（4MiB 已满足实时性；规格要求 S1 校准，正式校准建议并入后续 spike）。
5. **音频（f32le 管道）**不在本 spike（S5/S6）。
6. **慢读/阻塞/取消/EOF/压力**（§8 S1 的其余验收）不在本 spike，需独立故障注入 spike。
7. 单机单配置结果；最低硬件矩阵（旧 iGPU/NVIDIA 旧卡/Win10 2004）未验证（归 M3/最低承诺 OS 矩阵）。
8. 工具链注记：rust-mingw（GNU 目标）缺少 `as.exe` 导致 windows crate 编译失败（dlltool 生成导入库需调用 GNU as）；本机已有 MSVC BuildTools + Windows SDK，故切 MSVC 目标解决。若 CI/其他机器无 MSVC，需另配 MinGW binutils——产品工具链文档需记录此前提。
9. qsv 会话上限（≥4 路）未测——第三路承诺已验证，更多路数无产品需求（v1 上限 3 路），不阻塞。
