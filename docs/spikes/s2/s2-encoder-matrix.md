# S2 Spike 报告：FFmpeg LGPL 构建「编码器能力与许可矩阵」

- 日期：2026-10-15（测试当日）
- 执行者：S2 spike（仅验证实验，无生产代码）
- 对应验收：specs/001-screen-recorder.md §8「S2 编码能力与许可（最先）」——产出「枚举/初始化/并发/关键帧控制/吞吐/许可」矩阵；达到产品承诺并发（§9 T1：保底 2 路 1080p30）；无硬件场景有已验证路径或明确不支持契约。
- 结论速览：四候选（NVENC/QSV/AMF/`h264_mf`）**全部编入本构建**；本机可初始化且通过单/双/三路并发与关键帧验证的是 **h264_qsv** 与 **h264_mf**；NVENC 因**构建所用 NVENC API 头文件（13.1）新于本机驱动（596.36 → API 13.0）**初始化失败；AMF 因无 AMD 硬件（`amfrt64.dll` 缺失）失败。许可核验：**LGPL v3，未发现 GPL 组件**。`h264_mf` 存在且可用——满足 D4 入链前提，但实测走的是**微软软件 MFT**。

---

## 1. 构建与下载事实【实测】

| 项 | 值 |
|---|---|
| 仓库 Release | BtbN/FFmpeg-Builds，tag `latest`（published 2026-10-15T13:25:21Z） |
| 资产 URL | https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-lgpl.zip |
| 资产文件名 | ffmpeg-master-latest-win64-lgpl.zip（static，非 shared） |
| 资产更新时间 | 2026-10-15T13:24:09Z（构建日期 20261015，与版本串一致） |
| 大小 | 177,800,800 字节 |
| SHA256 | `F899D80C186178E9F84928593413A2C1F90ECDBBB3B0389351B6C92BAE06C3E8` |
| ffmpeg 版本串 | `ffmpeg version N-127564-g3c92f003c7-20261015`（gcc 16.2.0，crosstool-NG 1.29.0.7_b1a94f6） |
| 上游源码定位 | `g3c92f003c7` → FFmpeg 上游 commit `3c92f003c7`（"avformat/iamf_parse: reject a zero parameter rate"，已核验存在于 github.com/FFmpeg/FFmpeg） |
| 解压目录（后续所有调用使用） | `C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\tools\ffmpeg\ffmpeg-master-latest-win64-lgpl\bin\`（该目录已加入 .gitignore，二进制不入库） |
| 压缩包内许可文件 | 仅 `LICENSE.txt`（7,651 字节，GNU LGPL v3 全文）；无逐组件归因文件 |

## 2. 环境记录【实测】

原始输出：`docs/spikes/s2/raw/environment.txt`

| 项 | 值 |
|---|---|
| OS | Windows 11 Pro 25H2（`cmd /c ver`：10.0.26200.9457；注册表 CurrentBuild=26200 UBR=9457；ProductName 字段为遗留的 "Windows 10 Pro"） |
| CPU | Intel(R) Core(TM) Ultra 7 270K Plus，24 核 / 24 线程 |
| GPU 1 | Intel(R) Graphics（Arrow Lake-S iGPU，PNP `PCI\VEN_8086&DEV_7D67`），驱动 32.0.101.6874（2025-05-25） |
| GPU 2 | NVIDIA GeForce RTX 5070 Ti（Blackwell），驱动 32.0.15.9636（= 596.36，2026-04-23）；`nvidia-smi`：596.36，16303 MiB |
| GPU 0 | VirtualMonitor Device（虚拟适配器，`ROOT\DISPLAY\0000`，排最前——多适配器环境注意点） |

## 3. 能力清单（构建内枚举）【实测】

原始输出：`raw/buildconf.txt`、`raw/encoders.txt`、`raw/encoder-help-<名>.txt`

| 编码器 | `-encoders` 中存在 | `-h encoder=<名>` | 备注 |
|---|---|---|---|
| h264_nvenc | ✓ | ✓（131 行帮助，支持 nv12/p010le/cuda/d3d11 等） | "Supported hardware devices: cuda cuda cuda d3d11va d3d11va" |
| h264_qsv | ✓ | ✓（93 行；像素格式 `nv12 qsv`） | Supported hardware devices: qsv |
| h264_amf | ✓ | ✓（124 行；像素格式 `nv12 yuv420p d3d11 dxva2_vld ...`） | Supported hardware devices: d3d11va dxva2 amf |
| h264_mf | ✓ | ✓（27 行；像素格式 `nv12 yuv420p d3d11`） | **NV12 为官方支持输入**（能力声明） |
| libvpx-vp9 | ✓ | （未单独取帮助） | 软编，用于能力确认 |
| libx264 / libx265 / libxavs2 / libxvid | **✗ 均不存在**（grep 无命中，`-buildconf` 为 `--disable-libx264 --disable-libx265 --disable-libxavs2 --disable-libxvid`） | — | GPL 组件排除的直接影响证据 |
| libopenh264 | ✓（`V....D libopenh264`） | — | buildconf `--enable-libopenh264`；OpenH264 源码为 BSD-2-Clause【文档记载，未逐文件核验】 |

buildconf 中与编码相关的启用项：`--enable-amf --enable-ffnvcodec --enable-cuda-llvm --enable-libvpl --enable-libvpx --enable-version3 --enable-libopenh264` 等（全文见 raw）。

## 4. 编码矩阵【实测】

测试输入（合成）：`-f lavfi -i testsrc2=size=1920x1080:rate=30 -t 10 -g 60`，输出 MKV。
ffprobe 校验：时长 + 关键帧落点（packet flags K）+ 最大关键帧间隔。

| 编码器 | 存在 | 初始化 | 单路 | 双路并发 | 三路并发 | 关键帧控制（`expr:gte(t,n_forced*5)`） | 吞吐粗测（lavfi 合成，非生产结论） | 备注 |
|---|---|---|---|---|---|---|---|---|
| **h264_qsv** | ✓ | ✓ Intel iGPU | ✓ exit 0，2.13 s，6,255,805 B；duration 10.000000；5 关键帧，max gap 2.0 s（`-g 60` 生效） | ✓ 2/2（2.14 s / 2.27 s，speed≈4.9x） | ✓ 3/3（2.91–3.16 s，speed≈3.5x） | ✓ 强制关键帧精确落在 0/2/4/**5.000000**/7/9 s，max gap 仍 2 s | 单路 4.92x；三路各 3.5x | 默认 CQP 档位有提示日志；生产需显式指定 RC/bitrate（S5/S6 校准） |
| **h264_mf** | ✓ | ✓ MFT name: `'H264 Encoder MFT'`（微软**软件** MFT，见 §6） | ✓ exit 0，0.66 s，3,562,766 B；duration 10.000000；5 关键帧，max gap 2.0 s | ✓ 2/2（0.99 s / 1.14 s，speed≈11.3x） | ✓ 3/3（1.45–1.83 s，speed≈7.5x） | ✓ 同上（0/2/4/**5**/7/9） | 单路 17.7x；三路各 7.5x | 未接 GPU 依赖即可用；编码输出 yuv420p；NV12 输入为能力声明（真实 NV12 管道待 S1） |
| **h264_nvenc** | ✓ | **✗** | ✗（0 B，立即失败） | N/A（初始化失败） | N/A | N/A | — | 失败原因明确：`Driver does not support the required nvenc API version. Required: 13.1 Found: 13.0` / `The minimum required Nvidia driver for nvenc is 610.00 or newer`。本机驱动 596.36 = API 13.0。**是"master 构建头文件过新"，非 GPU/驱动故障** |
| **h264_amf** | ✓ | **✗** | ✗（0 B，立即失败） | N/A | N/A | N/A | — | `DLL amfrt64.dll failed to open`——机器无 AMD GPU/驱动，属预期不支持场景；失败信号明确（利于 T1「明确报错」契约） |
| libvpx-vp9（参考行） | ✓ | ✓ | ✓ exit 0，5.24 s（`-cpu-used 4 -row-mt 1` 加速参数），9,145,629 B；duration 10.000000 | 未测 | 未测 | 未测 | speed≈1.95x（仅能力确认，**不下性能结论**） | 按 §9 T6，v1 不作为兜底输出 |

要点核对：
- **产品承诺并发（T1：保底 2 路 1080p30）**：qsv 与 mf 均以 3/3 通过，超出承诺【实测】。
- **NVENC 旧卡 2 会话上限**：本机 RTX 5070 Ti 非旧卡，且初始化即失败，**会话上限未测得**【未验证】（见 §9 问题 1）。
- 关键帧控制同时验证了 `-g 60`（2 s GOP）与 5 s 强制点可共存，两编码器行为一致——与 D5「5s 分段 + 闭合 GOP/IDR」方向一致（分段行为本身归 S5）。

## 5. 复现命令原文

环境（PowerShell，ffmpeg 取自解压目录绝对路径）：

```
$ff = "C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\tools\ffmpeg\ffmpeg-master-latest-win64-lgpl\bin\ffmpeg.exe"
$fp = "C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\tools\ffmpeg\ffmpeg-master-latest-win64-lgpl\bin\ffprobe.exe"
```

能力清单：

```
ffmpeg -hide_banner -version
ffmpeg -hide_banner -buildconf
ffmpeg -hide_banner -encoders
ffmpeg -hide_banner -h encoder=h264_nvenc
ffmpeg -hide_banner -h encoder=h264_qsv
ffmpeg -hide_banner -h encoder=h264_amf
ffmpeg -hide_banner -h encoder=h264_mf
```

单路（每编码器同参数；输出路径仅为示例变量 $out，实际写入临时目录 `%TEMP%\opencode\s2out`）：

```
ffmpeg -hide_banner -nostdin -f lavfi -i testsrc2=size=1920x1080:rate=30 -t 10 -g 60 -c:v h264_qsv -y $out
ffmpeg -hide_banner -nostdin -f lavfi -i testsrc2=size=1920x1080:rate=30 -t 10 -g 60 -c:v h264_mf -y $out
ffmpeg -hide_banner -nostdin -f lavfi -i testsrc2=size=1920x1080:rate=30 -t 10 -g 60 -c:v h264_nvenc -y $out   # 失败
ffmpeg -hide_banner -nostdin -f lavfi -i testsrc2=size=1920x1080:rate=30 -t 10 -g 60 -c:v h264_amf -y $out    # 失败
```

ffprobe 校验：

```
ffprobe -v error -show_entries format=duration -of csv=p=0 $out
ffprobe -v error -count_packets -select_streams v:0 -show_entries stream=nb_read_packets,codec_name,width,height,avg_frame_rate -of csv=p=0 $out
ffprobe -v error -select_streams v:0 -show_entries packet=pts_time,flags -of csv=p=0 $out   # 过滤 flags 含 K 的行
```

双路/三路并发（两/三个进程同时各编一路；同编码器、同参数；由 `tools/spikes/s2/run_s2_tests.ps1` 以 `Start-Process` 近同时启动并逐进程等待、记录 exit/耗时/大小）：

```
powershell -NoProfile -ExecutionPolicy Bypass -File "tools\spikes\s2\run_s2_tests.ps1" -Phase concurrent -Encoders h264_qsv -N 2
powershell -NoProfile -ExecutionPolicy Bypass -File "tools\spikes\s2\run_s2_tests.ps1" -Phase concurrent -Encoders h264_mf -N 2
powershell -NoProfile -ExecutionPolicy Bypass -File "tools\spikes\s2\run_s2_tests.ps1" -Phase concurrent -Encoders h264_qsv -N 3
powershell -NoProfile -ExecutionPolicy Bypass -File "tools\spikes\s2\run_s2_tests.ps1" -Phase concurrent -Encoders h264_mf -N 3
```

关键帧控制：

```
ffmpeg -hide_banner -nostdin -f lavfi -i testsrc2=size=1920x1080:rate=30 -t 10 -g 60 -force_key_frames expr:gte(t,n_forced*5) -c:v h264_qsv -y $out
ffmpeg -hide_banner -nostdin -f lavfi -i testsrc2=size=1920x1080:rate=30 -t 10 -g 60 -force_key_frames expr:gte(t,n_forced*5) -c:v h264_mf -y $out
```

VP9 能力冒烟（非默认加速参数，仅能力确认）：

```
ffmpeg -hide_banner -nostdin -f lavfi -i testsrc2=size=1920x1080:rate=30 -t 10 -g 60 -deadline good -cpu-used 4 -row-mt 1 -c:v libvpx-vp9 -y $out
```

## 6. `h264_mf` 关键观察

- **存在性**：本构建编入 `h264_mf`【实测】→ 研究文档 §8 的假设成立。
- **初始化与质量**：1080p30 × 10 s 成功，三路并发通过，吞吐粗测充足【实测】→ **满足 D4 入链前提（构建编入 + 可初始化 + 关键帧 + 并发）**。
- **NV12**：`-h encoder=h264_mf` 声明支持 `nv12`【实测（能力声明）】；本测试输入为 testsrc2 产出的 yuv420p（输出流亦为 yuv420p）。真实 NV12 命名管道帧输入由 S1 覆盖【未验证】。
- **走的是软件 MFT**：日志 `MFT name: 'H264 Encoder MFT'` 为微软操作系统自带**软件**编码器 MFT【实测】。推断：ffmpeg mfenc 在未提供 D3D 设备上下文时选择软件 MFT；即使系统存在 Intel/NVIDIA 硬件 MFT 也未被选用【推断】。含义：
  - `h264_mf` 可作为**不依赖 GPU 的 CPU 兜底**（23/24 核机器上三路 7.5x 实时，余量充足）；
  - 若想经 MF 用 Intel/NV 硬件 MFT 需 d3d11 hwdevice 管线，复杂度高，而 Intel 硬件路径已有 h264_qsv 覆盖——不建议【推断，待评审】。
- 与 D4 的对照：`h264_mf` **不得作为无条件兜底**的要求仍然成立——建议以「启动时探测 + 明确报错（§9 T1）」纳入回退链末位，而非静默降级。

## 7. 许可结论

【实测】核验链：
1. `-buildconf`：`--enable-version3`（→ LGPL v3 而非 2.1）；`--disable-libx264 --disable-libx265 --disable-libxavs2 --disable-libxvid`。
2. `-encoders` 全量：**无** libx264/libx265/libxavs2/libxvid 编码器（grep 零命中）。
3. 随包 `LICENSE.txt`（7,651 B）为 GNU LGPL v3 全文；压缩包内**无其他许可/归因文件**。
4. 结论：**本构建为 LGPL v3，未发现 GPL 组件**。用于「每路一个 LGPL ffmpeg sidecar」的 D3 架构无许可冲突。

【文档记载】补充（影响分发合规清单，未逐文件核验）：
- `libopenh264` 源码许可为 BSD-2-Clause（Cisco OpenH264）；注意 Cisco 的**预编译二进制**另有专利条款——BtbN 从源码编译，走 BSD，但产品分发时应保留归因。
- LGPL §4 对应源码获取方式【文档记载】：BtbN README 说明 master 构建出自 FFmpeg 上游 git master（版本串中的 `g3c92f003c7` 即上游 commit，**已实测核验存在**）；依赖清单以 BtbN 仓库 `scripts.d/` 目录为准（"For a list of included dependencies check the scripts.d directory. Every file corresponds to its respective package."）。产品分发时应：随附 LICENSE.txt + 提供上游 commit 与 BtbN scripts.d 链接（或自行重构建）。
- NVENC/QSV/AMF/MF 均经由本机驱动/OS 运行时（nvenc 经 nvEncodeAPI、amf 需 AMD 驱动 DLL、qsv 经 Intel 运行时、mf 经 Windows MFT），构建内不打包其 SDK 二进制。

## 8. 推荐回退链（供评审冻结，顺序按探测结果）

启动时逐个探测初始化（失败信号均明确、快速，<0.2 s），选定第一个可用者；全部失败 → §9 T1 明确报错并列出可用编码器，不静默降级：

1. **h264_nvenc**（NVIDIA）——但**必须**换用与本机驱动兼容的构建（见 §9 问题 1）；本构建在本机驱动 596.36 下不可用。
2. **h264_qsv**（Intel iGPU/ARC）——本机实测最优硬件路径之一，三路并发通过。
3. **h264_amf**（AMD）——有 AMD 硬件的机器上探测；无硬件时失败信号明确（`amfrt64.dll` 缺失）。
4. **h264_mf**（OS 自带，实测为软件 MFT）——**无条件可用**（Win10 2004+ 自带软件 MFT），作为链末兜底并明示用户当前为软件编码【推断：软件 MFT 在支持的最低 OS 上可用性待 S3/最低 OS 矩阵复核】。

本机（iGPU + RTX 5070 Ti，本构建）实际生效链：`h264_qsv → h264_mf`。
VP9（libvpx-vp9）能力在构建内确认，但按 §9 T6，v1 不入链。

## 9. 未决问题（不猜测，逐条列出）

1. **NVENC API 版本 vs 驱动**：master-latest 构建（nv-codec-headers 13.1）要求驱动 ≥ 610.00，本机 596.36 不可用。BtbN 同 Release 的 `ffmpeg-n8.1-latest-win64-lgpl-8.1.zip`（release 分支构建，可能编译于较旧 API 头）能否在 596.36 上初始化【未验证】。产品应钉住 release 分支构建并文档化最低驱动；S2 后续可补测一次 n8.1 构建以闭合该问题。
2. **NVENC 会话上限**（旧卡 2 路）未测得：本机初始化即失败且非旧卡。需在驱动兼容的构建+硬件组合上补测【未验证】。
3. **h264_mf 硬件 MFT 路径**（d3d11 hwdevice）未验证；当前建议不采用（QSV 已覆盖 Intel 硬件路径）【未验证/推断】。
4. **真实 NV12/f32le 命名管道输入**未在本 spike 覆盖（本测试为 lavfi 合成）；吞吐数字不能外推为产品性能，归 S1/S6 校准【未验证】。
5. **多适配器序**：本机存在 VirtualMonitor 虚拟适配器排首，QSV 自动设备选择仍成功；其他多 GPU 机器（如 NVIDIA 独显 + 无 iGPU）上 QSV/MF 的默认适配器行为未验证【未验证】。
6. **音频（AAC）编码、分段（`-f segment`）、concat 收尾**不在本 spike 范围（S5/S6）。
7. 压缩包仅含 LICENSE.txt 单一许可文件；逐组件归因清单需在产品分发前整理（依赖 scripts.d）【文档记载，未逐项核验】。

## 10. 产物与原始日志路径

- 本报告：`docs/spikes/s2/s2-encoder-matrix.md`
- 测试脚本：`tools/spikes/s2/run_s2_tests.ps1`（spike 工具，非生产代码）
- 二进制（gitignore，不入库）：`tools/ffmpeg/ffmpeg-master-latest-win64-lgpl/`
- 原始日志（`docs/spikes/s2/raw/`）：
  - `version.txt`、`buildconf.txt`、`encoders.txt`
  - `encoder-help-h264_nvenc.txt` / `-h264_qsv.txt` / `-h264_amf.txt` / `-h264_mf.txt`
  - `environment.txt`
  - 单路：`single_h264_nvenc.log`、`single_h264_qsv.log`、`single_h264_amf.log`、`single_h264_mf.log`（各含 CMD 原文、START 时间、完整 stderr、RESULT 行、ffprobe 段）
  - 双路：`conc2_h264_qsv_p1.log`、`conc2_h264_qsv_p2.log`、`conc2_h264_mf_p1.log`、`conc2_h264_mf_p2.log`
  - 三路：`conc3_h264_qsv_p1..p3.log`、`conc3_h264_mf_p1..p3.log`
  - 关键帧：`kf_h264_qsv.log`、`kf_h264_mf.log`
  - VP9：`vp9_smoke.log`
- 编码输出样本（临时目录，不入库、可随时复现）：`%TEMP%\opencode\s2out\*.mkv`
