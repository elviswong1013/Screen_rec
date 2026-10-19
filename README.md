# Screen_rec

**一句话定位**：Screen_rec 是一个 Windows 本地录制工具——把多个窗口/显示器分别录成独立成品，按来源保留独立音轨，进程崩溃后可扫描恢复，并在录制结束后用流拷贝把多路产物封装成一个多轨 MKV。

它同时保留截图、录屏、录音的基础能力；重点不是直播或剪辑全家桶，而是本地“多目标、各自成品、可恢复、可归档”的录制流程。基础能力包括全屏/窗口/区域截图、屏幕/区域视频录制、麦克风/系统/按应用录音；下面的比较重点放在多目标录制这条主线上。

以下能力快照截至 2026-10-19，基于已交付的 M17 开发版，并包含 M18 区域视频的源码验收：MP4 的系统播放器人工项仍待签，P5 内容区尚未实施。M18 尚未重新打包，当前安装包仍为 M17e；详见「项目状态」。

**结论先说**：这些能力拆开看并非市场独有。OBS 可以通过 Application Audio Capture、多音轨、Source Record 插件或多实例做出相近的独立输出；Cap、ShareX、Game Bar、Snipping Tool 各有更强的场景。Screen_rec 的差异在**单 GUI 整合**：多目标独立成品 + 每路按进程树选音源 + 每路独立生命周期 + 分段恢复 + 事后多轨封装。

当交付目标是“每个目标都保留一份完整、可独立停止和恢复的素材”，而不是把多个源实时合成到同一张画布时，一张合成画布的输出与逐目标成品是两种不同的交付。Screen_rec 的集成价值在于按目标管理音源、文件、停止/恢复状态，用户不必自行编排多实例或插件。这里说的是工作流定位，不是资源占用或性能优劣的实测结论。

## 为什么选择 Screen_rec

### 1) 多目标录制 = 每路一个独立成品，而不是一个画布里的多个区域

- **价值**：同时录 2 个及以上窗口/显示器时，每一路有自己的视频、所选音源形成的独立音轨、独立文件和独立生命周期；可以只停某一路；某一路目标丢失等路内故障可以独立处理，不影响其它路。
- **典型场景**：一边录浏览器里的课程、一边录本地播放器，两个文件分开交付；中途只停其中一个，另一个继续。
- **证据**：`tests/sr-itests/tests/multi_route.rs` 覆盖停一路、停中间一路、单路故障（路内）隔离；`tests/sr-itests/tests/audio_m6.rs` 覆盖多音轨探测与 FFT。`m13-report.md` 记录了 610s 双路 1080p30 同步测试（逐路 max 99.6/93.3ms）和 620s 双路 soak；`m16-report.md` 的 96k 双路 max 123.2ms；`m17-report.md` 的 MP4 双路 max 106.3ms。概括为：已测配置 610s 双路逐路 max ≤150ms。
- **边界**：这是已测配置的结果，不是全硬件性能承诺；更高路数受编码器与磁盘吞吐限制。QPC 统一时基 + 漂移重采样是降低累计漂移的机制，不是“永久不漂”的保证；整个宿主崩溃仍会影响全部路。纯录音的多个音源在同一个会话里完成，不提供“每条音轨单独停止”。

### 2) 音轨按来源拆开，应用隔离的粒度是“进程树”

- **价值**：每一路可以选择自己的音源（目标应用、系统、麦克风、额外应用），输出为独立音轨、互不混音；纯录音也支持多来源。
- **典型场景**：A、B 两个不同应用同时播放，各自录一路、各自出文件，并排除第三应用（只有未选择系统混音时）；应用音频和麦克风各占一条轨，后期可分别处理。
- **证据**：`crates/sr-audio/src/wasapi.rs` 的进程环回按 `INCLUDE_PROCESS_TREE` 捕获；`audio_m6.rs` 验证多音轨；`multi_route.rs` 验证逐路音源隔离。
- **边界（进程树）**：进程树是系统 API 的粒度——同一 Chrome 进程树内的窗口/标签无法分别隔离音频。
- **边界（系统音轨）**：系统音轨按设计混入所有送入该音频输出端点的应用，设备混音不等于应用隔离；要 A/B 分轨并排除第三应用，必须不选系统混音/系统音轨。

### 3) 面向失败的 5s 分段、恢复与收尾清理

- **价值**：录制按约 5s 的 MKV 分段；重启后扫描会话并收尾；成品校验并提交成功后才默认清理原段；失败保留恢复资料；可设置保留原始分段用于排查。
- **典型场景**：应用或编码进程崩溃、误关程序、收尾失败后重试；需要事后检查原段或重新收尾。
- **证据**：`specs/001-screen-recorder.md` §2.5（分段与收尾，含 REQ-21/REQ-22）写明损失上界、恢复覆盖范围与完成段保留；`docs/spikes/s5/s5-crash-recovery.md` 是既有强杀/恢复实测记录；`m14-report.md` 记录成功后清理与 `keep_segments` 策略。
- **边界（损失与恢复）**：损失预算是 **5s + δ**（δ 来自编码器/关键帧/封口），末尾未封口内容可能丢失；**不是零损失，也不承诺严格的“最多 5 秒”**。只对已验证的进程级 crash 场景负责，不保证断电或文件系统损坏。
- **边界（清理）**：尽力清理；删除失败可能留下部分原段/目录，不影响已提交成品。
- **边界（非独有）**：崩溃恢复不是本项目独有（OBS 等也有各自的防护）；差异在它与多目标独立成品、收尾清理的整合，而不是“独有”。

### 4) 事后多轨封装：把分离录制变成可归档的多轨 MKV

- **价值**：合并时对每个输入显式 `-map i` 全轨，再用 `-c copy` 封装；异构分辨率、帧率、时长、轨数的输入保留原编码，输出包含多视频/音轨的 MKV。支持 MP4/MKV 输入，输出固定 MKV。
- **典型场景**：多窗口/多显示器分别录制，事后封装为一个包含全部轨道的归档文件，不重新编码。
- **证据**：`crates/sr-merge/src/merge.rs`；`tests/sr-itests/tests/merge.rs` 的异构多视频 MKV 用例。
- **边界**：这是**容器封装**，不是再编码；不等于“无损采集”，也不等于拼画面/画中画/时间线串接/自动时间对齐/混音。播放器通常需要手动选轨，不能保证同时显示多路视频。

### 5) 本地录制与设备选择：不用为每个目标另搭一套流程

- **价值**：目标选择、按来源音轨、独立文件、恢复收尾、合并都在一个本地 GUI 里完成；显示器可按 ContainerId 关联 HDMI/DP 端点；输出可选 44.1k/48k（默认）/88.2k/96k。
- **典型场景**：本地课程、演示、素材归档，不希望把内容上传到云；外接 HDMI/DP 显示器时希望录该屏对应的声音；需要统一采样率。
- **证据**：`m16-report.md` 的端点枚举/ContainerId 匹配/运行时失效；`m15-report.md` 的四档率；`ui/endpoint-match.js` 的匹配与歧义状态。
- **边界（端点）**：端点匹配依赖驱动元数据，是便利取舍，不保证任意屏都精准匹配；未匹配/歧义时提示确认或明确改用系统混音，**不静默选枚举第一项**；录制中已选音频输出端点失效会停该路并提示，不静默切到别的设备。
- **边界（采样率）**：应用进程环回固定 48k，升采样不增加来源信息；真实 96k 捕获设备尚未验证。高采样率 AAC/MP4 存在 Windows 系统播放器兼容风险，但这不等于证明本机必不支持，AC-8 人工项仍待。
- **边界（容器与编码）**：默认 MKV；MP4 是“内部 MKV 分段 → 收尾 stream-copy remux”；纯录音输出 WAV。H.264/AAC，编码器按 NVENC→QSV→AMF→MF 候选探测，本机已测 QSV/MF，不保证所有机器/品牌可用。
- **边界（本地优先）**：本地录制/合并不依赖云，但这不构成对 OBS/ShareX/Cap 的独占优势（Cap 也支持 local/selfhost）。

## 竞品对比：什么时候选它，什么时候选本项目

> 资料核对日期：2026-10-19；依据官方/上游资料与项目既有报告。非同机性能或操作步数 benchmark。竞品“官方资料未确认”不等于“不支持”，不把未知项当作劣势。

| 产品 | 对方更强的地方 | 本项目更适合时 | 何时选对方 |
|---|---|---|---|
| **OBS Studio** | 场景合成、直播、滤镜、虚拟摄像头、跨平台、成熟插件生态。原生 Application Audio Capture（Beta，28+）按进程树捕获；多音轨录制（6 轨）；30.1 起 Window/Game Capture 可附音频；Source Record 插件或 `--multi` 多实例可做相近的独立输出。 | 需要围绕多个目标的一体化独立成品流程：逐路生命周期、逐路恢复、事后合并，且不想维护多实例/插件/场景配置。 | 直播、场景切换、滤镜、虚拟摄像头、跨平台，或已有 OBS 工作流。 |
| **ShareX** | 截图/滚动截图/标注/OCR/上传/工作流/FFmpeg 录屏；桌面截图与分享自动化成熟。 | 核心需求是多目标同时独立录制、按来源音轨、恢复和合并，而不是截图/上传流水线。 | 截图、标注、OCR、上传/工作流自动化，或轻量单画面录制。 |
| **Xbox Game Bar** | Win+G 内置游戏捕获/Xbox 生态；无需额外安装。 | 多应用/窗口/显示器独立文件、按来源音轨、恢复与合并。 | 快速游戏片段、内置便利、Xbox 集成。 |
| **Snipping Tool** | 系统内置截图、矩形 VideoSnip、可衔接 Clipchamp 编辑。 | 多目标长时录制、逐路独立音轨/文件、恢复与多轨封装。 | 快速截图/短片段、系统内置、轻编辑。 |
| **Cap** | macOS/Windows；屏幕/摄像头/麦克风；编辑/分享；local/cloud/selfhost/S3；异步沟通、轻编辑、跨平台。 | Windows 本地多目标独立成品 + 进程树音源 + 恢复 + 多轨封装，不需要摄像头或云分享。 | 需要摄像头/画中画、轻编辑、分享链接、云或自托管、跨平台团队异步沟通。 |

补充说明，避免误读：

- **OBS 不是“做不到多路独立输出”**：Source Record 插件和 `--multi` 多实例都是可行路径。本项目的差异是单 GUI 的一体化流程，不宣称实测资源更省，也不宣称 OBS 录 MP4 必然整段丢失。
- **ShareX**：官方 `FFmpegOptions.cs` 暴露的是设备级音频选项；本次官方资料未确认本项目式 per-app 多路独立成品流程，**未确认 ≠ 不支持**。
- **Cap**：官方文档本次未确认 per-app 进程树多路独立成品；同样不能据此断言它绝对没有该能力。Cap 也支持 local/selfhost，不能把“本地/不上云”说成对它的独占优势。
- **Game Bar / Snipping Tool**：Game Bar 的定位是游戏快速捕获与 Xbox 集成；Snipping Tool 的定位是截图与矩形短片段。两者与多目标长时录制不是同一类工作流。

## 适合 / 不适合

### 适合

- Windows 10 2004+ / Windows 11（x64）上，需要同时录制多个窗口/显示器，且每路要独立文件、独立音轨、独立停止。
- 需要按应用（进程树）选择声音，同时保留麦克风/系统/额外应用为独立轨道。
- 接受“独立文件 + 事后合并”的归档方式，希望合并阶段不重新编码、保留全部轨道。
- 需要崩溃后重启扫描、收尾重试、成功后自动清理，或保留原始分段排查。
- 本地录制/本地合并为主，不想依赖云服务；但这不是对竞品的独占优势。

### 不适合

- 直播推流、场景切换、滤镜、虚拟摄像头、成熟插件生态：优先 OBS。
- 截图标注、OCR、上传/分享自动化：优先 ShareX 或系统截图工具。
- 摄像头/画中画、轻编辑、分享链接、云或自托管协作：优先 Cap。
- macOS/Linux 跨平台：本项目当前不支持。
- 需要同一 Chrome 进程树内按窗口/标签拆音轨：系统 API 粒度不支持。
- 需要断电/文件系统损坏下的零损失保证：本项目只覆盖已验证的进程级崩溃恢复。
- 需要画中画、时间线、自动时间对齐、混音或“一个画面里同时呈现多路视频”：本项目只做多轨封装，不负责多画面排布。

## 限制与已知边界

- **平台**：未跨平台；目标 Windows 10 2004+ / Windows 11 x64，主要在 Windows 11 26200 实测。per-app 还依赖 ProcessLoopback 的激活支持与权限；OS 版本达标不保证所有功能都可用。
- **音频粒度**：按进程树隔离；同一进程树的窗口/标签共享声音。系统音轨按设计混入所有送入该音频输出端点的应用；设备混音不等于应用隔离。要排除其它应用，必须不选系统混音/系统音轨。
- **纯录音**：多来源在同一会话内完成，不能宣传“每条音轨可单独停止”。
- **恢复损失**：5s + δ 预算，末尾未封口内容可能丢失；不是零损失，也不是严格“最多 5 秒”。只对已验证的进程级 crash 场景负责，不保证断电/文件系统损坏；清理是 best effort。
- **合并语义**：`-map` 全轨 + `-c copy` 的容器封装，不是无损采集，不是拼画面/画中画/时间线/自动对齐/混音；播放器通常要选轨，不能保证同时显示多视频。
- **端点**：ContainerId 关联依赖驱动元数据；未匹配/歧义时确认或改用系统混音；录制中失效会停该路。作为便利取舍，不保证任意屏精准匹配。
- **采样率与编码**：44.1/48/88.2/96k 可选；应用进程环回固定 48k，升采样不增加来源信息；真实 96k 捕获设备尚未验证。高率 AAC/MP4 有 Windows 播放器兼容风险，AC-8 人工项待签。编码器按 NVENC→QSV→AMF→MF 候选探测，本机已测 QSV/MF，不保证全部机器/品牌可用。
- **容器**：默认 MKV；MP4 为内部 MKV 分段 → 收尾 stream-copy remux；纯录音 WAV。不是所有播放器/编辑链都适合多轨 MKV。
- **安全与分发**：尚未做安全审计；NSIS 安装包未签名；不把 Rust/Tauri 当作性能证据。FFmpeg 的 chromaprint/FFTW GPL 冲突已移除；Rust 第三方通知、静态组件通知、编解码专利与正式发行条款仍未完成清权；当前安装包（M17e）仅供内部测试，不应作为正式发行版。
- **验收范围**：L3 的负坐标显示器、真实麦克风、多视频轨播放器兼容矩阵仍未签；本机无麦克风设备导致的 BLOCKED 不算通过（见 `l3-signoff.md` 与 `specs/002-recording-modes.md` 第 184–187 行）。

## 项目状态

| 阶段 | 范围 | 状态 |
|---|---|---|
| M0–M14 基础 | 截图/录屏/录音、分段、恢复、合并、成功后清理 | 已按项目已声明配置验收；仍有 L3 人工项待签（见 `l3-signoff.md`） |
| P1 / M15 | 采样率可选（44.1/48/88.2/96k） | **acc-8 ACCEPTED**；详见 `m15-report.md` |
| P2 / M16 | 全屏音轨、HDMI/DP 端点关联、端点失效 | **acc-10 ACCEPTED**；详见 `m16-report.md` |
| P3 / M17 | MP4 输出（MKV 分段 → 收尾 stream-copy remux） | 已实现；acc-13 对 acc-12 两项 P2 代码修复的复审通过；**AC-8 系统播放器人工记录仍待用户**；详见 `m17-report.md` |
| P4 / M18 | 区域视频录制 | **acceptor ACCEPTED（2026-10-19，源码验收）**；M18 尚未重新打包；麦克风、真实 96k 捕获、多视频轨播放器矩阵仍 BLOCKED/待签；详见 `m18-report.md` |
| P5 | Chrome/Edge 内容区 | 仅 spike 先行，未实施；不作为现有卖点 |
| 安装包 | M17e NSIS | 2026-10-19 重建并完成安装/自检；**仅为打包证据，不代表运行时播放器已验收**；见 `m17e-package-report.md` |

规格状态：[PRD 001](docs/prd/001-screen-recorder.md)、[PRD 002](docs/prd/002-recording-modes.md) 已批准；[SDD 001](specs/001-screen-recorder.md) 已批准；[SDD 002](specs/002-recording-modes.md) 按阶段推进，P1–P4 详规已批准，P5 尚待 spike/详规评审。批准不等于实现或验收，实际状态以上表为准。

## 文档导航

- [PRD 001：截图/录屏/录音基础与多路录制](docs/prd/001-screen-recorder.md)
- [PRD 002：录制模式扩展（采样率/全屏音轨/MP4/区域视频/内容区）](docs/prd/002-recording-modes.md)
- [规格工作流说明](specs/README.md)
- [SDD 001：基础规格](specs/001-screen-recorder.md)
- [SDD 002：录制模式增量规格](specs/002-recording-modes.md)
- [可行性研究（含 Rust 生态选型）](docs/research/feasibility-2026-10-15.md)
- [SDD 评审记录](docs/reviews/001-sdd-review-2026-10-15.md)
- [里程碑证据目录](docs/milestones/)
- [Spike 报告目录](docs/spikes/)

## 证据与官方引用

### 本项目证据（既有报告记载，不是本次新测）

- 多路与故障隔离：[multi_route.rs](tests/sr-itests/tests/multi_route.rs)、[audio_m6.rs](tests/sr-itests/tests/audio_m6.rs)
- 合并实现与异构多轨：[merge.rs](crates/sr-merge/src/merge.rs)、[merge 测试](tests/sr-itests/tests/merge.rs)
- 进程环回与端点：[wasapi.rs](crates/sr-audio/src/wasapi.rs)、[endpoint-match.js](ui/endpoint-match.js)
- 里程碑报告：[M13](docs/milestones/m13-report.md)、[M14](docs/milestones/m14-report.md)、[M15](docs/milestones/m15-report.md)、[M16](docs/milestones/m16-report.md)、[M17](docs/milestones/m17-report.md)、[M18](docs/milestones/m18-report.md)
- 人工签核与最新安装包：[L3 签核](docs/milestones/l3-signoff.md)、[M17e 安装包报告](docs/milestones/m17e-package-report.md)
- FFmpeg LGPL 整改：[FFmpeg LGPL 整改报告](docs/milestones/ffmpeg-lgpl-remediation.md)
- 崩溃恢复机制与实测：[S5 报告](docs/spikes/s5/s5-crash-recovery.md)

### 官方与上游资料（2026-10-19 核对）

- OBS：[Application Audio Capture](https://obsproject.com/kb/application-audio-capture-guide)、[多音轨录制](https://obsproject.com/kb/multiple-audio-track-recording-guide)、[Source Record 插件](https://github.com/exeldro/obs-source-record)、[`--multi` 多实例入口](https://github.com/obsproject/obs-studio/blob/master/frontend/obs-main.cpp)
- ShareX：[官网](https://getsharex.com/)、[FFmpegOptions.cs](https://github.com/ShareX/ShareX/blob/develop/ShareX.ScreenCaptureLib/ScreenRecording/FFmpegOptions.cs)
- Xbox Game Bar：[官方说明](https://support.xbox.com/en-US/help/games-apps/game-setup-and-play/get-to-know-game-bar-on-windows-10)
- Snipping Tool：[官方说明](https://support.microsoft.com/en-us/windows/apps/use-snipping-tool-to-capture-screenshots)
- Cap：[官方文档](https://cap.so/docs)、[GitHub](https://github.com/CapSoftware/Cap)
