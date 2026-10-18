# 001 - Screen_rec：Windows 截图/录屏/录音（按应用音轨隔离 + 多路同时录制）

Status: Approved
Approved: 2026-10-15（Gate 2 用户批准，含 §9 产品取舍）
Created: 2026-10-15
Updated: 2026-10-15
PRD: docs/prd/001-screen-recorder.md
研究依据: docs/research/feasibility-2026-10-15.md（§8 为 Rust 生态选型补充）
评审记录: docs/reviews/001-sdd-review-2026-10-15.md（plan-reviewer + 圆桌 glm/kimi/astra + council；本版已按评审修订）

## 1. 背景与目标

PRD（AC-1..10）已批准（2026-10-15）。本规格将其细化为可实现、可测试的编号需求（REQ）。技术栈：Rust + Tauri v2；目标环境 Win10 2004+ / Win11 x64。

关键结论与约束：
- 视频捕获：`windows-capture` 2.0.1（MIT，WGC 封装，仅取帧）；音频：`wasapi` 0.26（MIT，设备/进程环回、会话枚举）；重采样：`rubato` 5.x；窗口/显示器/Session 枚举：`windows` 0.62。
- 纯 Rust MKV 写入不可用（`matroska` crate 全量缓冲到 finish；`muxide`/`mp4e` 不支持 MKV）→ 编码与封装采用**锁定版本的 LGPL ffmpeg sidecar**（每路一个进程）。
- **Rust 是唯一时间轴调度者**：源时间戳（WGC SystemRelativeTime / WASAPI 设备位置）在 Rust 侧映射为帧槽/样本槽；raw 管道传的是"已按时间轴编排好的"数据流。
- 托管 CI runner 无法做真实捕获（WGC 黑帧）→ 测试分 L1（含无设备管线集成）/L2/L3，见 §4。
- 许可红线：GPL 组件（libx264/libx265 等）禁入发行物；Cap 仓库 `cap-enc-mediafoundation`/`cap-muxer` 为 AGPL，仅读架构。
- 本版评审结论：主干（sidecar + 分段 + 分层测试）成立；编码器能力、时间轴契约、音频隔离边界、分段恢复保证、resize 生命周期五项须在 M0 spike 后冻结（§8）。

## 2. 需求（REQ）

### 2.1 目标枚举与选择（AC-7）
- REQ-1 窗口枚举：列出可见顶层窗口（标题、应用名、进程名、PID）；过滤 cloaked（UWP 幽灵）窗口；UWP 经统一应用身份解析归属真实进程（与 REQ-14 同源）。条目身份 = HWND + PID + 进程创建时间（防 PID 复用）。
- REQ-2 显示器枚举：数量、物理分辨率、位置、主显示器标识。
- REQ-3 刷新：手动刷新；窗口打开/关闭在一次刷新后反映；≤2s（≤100 窗口规模）。
- REQ-4 音频来源枚举与快照：列出活动音频会话（PID、进程名、应用名）；「全选」= 启动时快照并写入会话清单；同根去重（同应用多会话归一）；启动后新增音源**不自动加入**（v1），UI 可手动追加。

### 2.2 截图（AC-1）
- REQ-5 截图三目标：显示器 / 窗口 / 矩形区域；输出 PNG 到输出目录。
- REQ-6 多显示器与 DPI：输出尺寸=物理像素；区域跨显示器时按各显示器帧在物理坐标系拼合后裁剪；支持负坐标。
- REQ-7 窗口截图：被遮挡仍正确（WGC 单帧）；最小化窗口返回明确错误。
- REQ-8 区域截图交互：全屏覆盖层框选（多显示器按物理坐标映射）。

### 2.3 视频捕获（AC-2/3/9）
- REQ-9 每路录制 = 独立 WGC 会话（独立线程 + 独立 frame pool；windows-capture 2.0.1，仅取帧）。
- REQ-10 帧节奏器：以帧的**源时间戳**（WGC SystemRelativeTime，QPC 域）映射到 CFR 时间槽（n/fps 相对公共 T0）；到达顺序不作为时间依据；按槽判定迟到/超前（重复/丢弃），frame pool 批量送达不得造成 judder。
- REQ-11 光标捕获开关（默认开）；去边框等能力不可用时按 REQ-28 探测降级，不阻塞录制。
- REQ-12 多路：≥2 路同时录制（保底并发承诺见 §9 T1）；每路独立输出文件、独立停止；目标窗口关闭 → 该路优雅停止并保留内容；单路故障不影响其它路。
- REQ-13 窗口几何变化（resize/最小化/显示器热插拔）：v1 采用**固定输出画布**——启动时固定宽高/fps/像素格式；窗口变化时等比缩放+黑边达到画布；WGC/D3D 资源重建；最小化/零尺寸 → 按规则补帧+状态提示（不结束录制）；重建失败 → 仅停止该路。同一路各段参数恒定兼容；**不同路/不同轨分辨率不同是合法情况**（不得影响 AC-5 合并）。

### 2.4 音频捕获与音轨（AC-2/4/6）
- REQ-14 应用身份解析与按应用音频（核心）：
  - 音视频共用同一身份解析结果（REQ-1）；进程身份 = PID + 创建时间 + 路径/包身份校验。
  - 默认捕获范围 = 目标应用进程 PID 及其**后代树**（INCLUDE_TARGET_PROCESS_TREE）。
  - **有界提升**：仅当存在经验证的应用规则（如浏览器音频位于兄弟进程）才可扩大捕获根，依据须可审计；**禁止**仅凭进程名扩大；**硬性禁止**跨系统宿主扩大（explorer.exe、svchost.exe、ApplicationFrameHost.exe、RuntimeBroker.exe、sihost.exe）。
  - 无法可靠归属 → 用户确认或声明不支持；**禁止静默改录系统混音**。
  - 共享范围告知：浏览器等场景 UI 明示「该应用所有窗口共享此音轨」。
  - 重叠捕获（祖先/后代、同根多路）→ 提示并按根去重（同根共享一个采集客户端、多路分发）。
  - 音源应用退出（PID 消失）→ 该轨告警并标记轨道结束；v1 不按同名自动重绑，UI 可手动重选；会话清单记录进程名+路径+创建时间。
  - 结果分三层呈现：能力探测 / 身份解析 / 实际采集状态（激活成功 ≠ 隔离正确）。
- REQ-15 系统音频：默认渲染端点设备环回（v1 仅默认端点；多端点不支持，文档说明；系统轨为整机混合，含各应用声音）。
- REQ-16 麦克风：设备枚举与逐路选择（RouteConfig 含 mic_device）；无设备时明确不可用（SKIP，不得静默）。
- REQ-17 音轨输出模式：
  - 隔离（默认）：目标应用轨（+ 麦克风轨，若启用）。
  - 全音轨：目标应用 + 系统 + 麦克风 + 追加应用（含全选），各独立轨、互不混音。
  - Monitor/Region 无天然应用 → 音源须显式选择（默认：系统 + 麦克风若启用）。
  - 追加应用按根去重；快照语义（REQ-4）。
- REQ-18 音频时间轴：48kHz/f32/立体声（wasapi autoconvert）；WASAPI 时间戳为 **100ns 单位** → 统一 epoch（REQ-25）；静音补齐按样本槽；补零区间后到达的迟到包必须**裁切/丢弃**（禁止同一时间段写两次）；区分真实静音/断流/设备错误（AUDCLNT_BUFFERFLAGS 语义）；重采样器延迟补偿记账。
- REQ-19 纯录音：1..N 来源 → 各来源独立 WAV（48kHz s16）；停止即成品；崩溃恢复=头部重写+尾部不完整样本裁切+RIFF 4GiB 上限策略（超限分段或提示，实现期定）。

### 2.5 编码与封装（AC-5/9/10）
- REQ-20 管线与编码器：
  - 每路一个 ffmpeg sidecar（**锁定精确构建+校验和**，LGPL）；命名管道输入：1 路 NV12 rawvideo + k 路 f32le 音频；Rust 侧零编码，但 NV12 转换/缩放成本纳入预算（S1 测全链路）。
  - 编码候选由 S2 实测矩阵裁决（NVENC/QSV/AMF/`h264_mf`）：`h264_mf` 仅在证明「构建编入+本机可初始化+满足 NV12/关键帧/并发/吞吐」后方可入链，**不得作为无条件兜底**；回退顺序由矩阵确定。无可用编码器行为见 §9 T1（默认：明确报错并列出可用编码器，不静默降级）。
  - 缓冲/队列：按**字节+时间双重限界**（禁止按帧数大排队，如 512 帧 NV12 ≈1.5GiB/路）；管道缓冲 4–16MiB 为实测起点；`-thread_queue_size` 为调优项。
  - 可取消异步 I/O；持续排空 stderr；进程生命周期/超时/单路隔离（Job 对象，见 REQ-22）。
  - 启动协议：管道服务端先建 → spawn sidecar → 采集预热 → 编码器探测 → 公共 T0 → 各输入**并发**供数；**不得等全部输入被打开**（避免互等死锁）；启动超时与失败回滚。
  - 停止协议：停止生产 → 各输入 EOF（`-nostdin`；关闭管道写端）→ 编码器排空（超时强杀）→ concat。
- REQ-21 分段与收尾：
  - 强制周期 IDR + 闭合 GOP（按 fps 配置 `-g` + `-force_key_frames`），保证每段独立可播；段长容差与切段验证（S5）。
  - 崩溃损失上界 = segment_secs + δ（δ 冻结前确定，目标 ≤1s；见 §9 T4）；写入显式验收。
  - 收尾 = concat demuxer + 显式全轨映射（`-f concat -safe 0 -i list.txt -map 0 -c copy`；生成 ffconcat 列表，不用 CSV 作 concat 输入）。
  - **收尾在后台执行**：进度、失败原因、可重试；「停止采集」有独立响应目标（请求响应 ≤2s）；**删除无条件「成品 ≤10s」承诺**（见 §9 T5）。
  - 收尾前各卷空间预检（峰值 ≈ 成品大小）；成品验证成功后自动清理原段（默认；设置『保留原始分段』开启时保留）；失败不删原段。
- REQ-22 恢复状态机：
  - 会话 Manifest schema v1：版本、每路状态、段列表（含时间范围）、完整段判定、最终输出路径；**原子更新**（临时文件+rename）；「段完成但清单未更新」→ 恢复时目录重扫核对。
  - sidecar 生命周期：**Job 对象绑定主进程**（主进程消失 → sidecar 终止；代价=丢当前段，换取可预测恢复）；EOF 自愈仅作验证后优化；恢复扫描排除仍在写入的文件。
  - 收尾幂等、可重复；杀进程矩阵（主进程/编码进程/整树/收尾中再崩/磁盘满/损坏尾段/清单损坏/不足 5s 段）逐项定义行为。
  - 保证范围：进程级崩溃（**不含断电/文件系统损坏**，另议）。
  - WAV 恢复并入恢复清单。
  - 完成段永不丢弃；损失仅限未封口段（≤segment_secs+δ）。
- REQ-23 合并（AC-5）：≥2 产物 → 单 MKV；`-map 0 -map 1 … -c copy` 全轨保留（多视频轨并存）；**允许异构（分辨率/帧率/轨数不同），不得误拒**；不得覆盖输入；不用 `-shortest`（保留完整时长）；各文件按本地零点合并（会话时间偏移 v1 不启用）；轨道命名/默认选择规则明确；取消/失败/磁盘满保留输入。
  （与 REQ-21 的「同路分段 concat」是两种不同操作：命令与校验分开。）
- REQ-24 命名/目录/提交：
  - 命名 = `{yyyyMMdd_HHmmss}_{目标名}`；sanitize Windows 非法字符（`\/:*?"<>|`）与保留名；同秒同目标冲突加序号；禁止覆盖现有文件。
  - 输出目录可配（默认 `%USERPROFILE%\Videos\Screen_rec`）；会话临时目录置于**输出卷内**（默认 `<输出目录>/.screenrec_tmp/<session_id>/`，保证收尾同卷 rename/低风险）；跨卷场景仍按各卷预检空间；安全提交=临时名→验证→rename。

### 2.6 同步与稳定性（AC-9）
- REQ-25 时间轴契约：
  - 统一 epoch：公共 T0（QPC）；WGC SystemRelativeTime（QPC ticks）与 WASAPI 设备位置/QPC（100ns）到公共时间轴的单位换算必须显式实现并单测。
  - 屏障前数据丢弃（以 T0 为 t0）；启动延迟按帧槽/样本槽补齐或裁剪。
  - **内容级验收**：任意应同步轨对最大内容偏差 ≤150ms 且不累积；测量点=开始/中段/结束/**concat 边界**（半自动分析，见 §4）。
- REQ-26 漂移与阶跃控制：
  - 慢漂移：rubato 变比重采样；测量窗口/滤波/滞回带/速率限幅（±500ppm 为实验起点，S6 定值）/恢复条件/重采样延迟补偿，全部参数化记账。
  - 阶跃校正（与 rubato 分开记账）：背压/停滞恢复后按「已写帧数/fps vs 已写样本数/48000 vs QPC」计算内容偏差，一次性丢/补音频样本（阈值起点 40ms）。
  - 区分处理延迟与内容时间偏差；持续吞吐不足时**有界失败**（超过恢复窗口只停止受影响路，禁止无限缓存/追帧）。
  - 故障时间轴：设备切换/拔插/discontinuity/无效时间戳/睡眠恢复/GPU 重置/编码停滞 → 明确降级或停止该路；不承诺未验证的无缝恢复。
- REQ-27 稳定性：2 路 1080p30 ≥10 分钟无崩溃（AC-9 保底；并发承诺见 §9 T1）；告警不受电平事件流饿死（独立通道/优先级）；soak 观测=CPU/GPU/内存/队列水位/丢帧/校正事件/磁盘增长；磁盘不足安全停止并保留内容。

### 2.7 能力探测与降级（AC-8）
- REQ-28 探测与降级：
  - 分层探测：OS build；候选编码器实际初始化（非仅枚举）；process loopback 试激活；麦克风隐私；WGC 授权。
  - 权限/激活行为分三类分别定义（麦克风隐私 / 音频激活 / WGC），各自处理取消/超时/迟到回调；不得预设 loopback 必有 consent 弹窗（S3 核实）；「激活成功但无信号」须经检测流程判定，不得直接归因权限或受保护音频。
  - Win10 19041 的隔离能力承诺待 S3（§9 T2）。

### 2.8 工程约束
- REQ-29 许可与构建：
  - Rust 工具链锁定（`rust-toolchain.toml`；MSRV 按选定 tauri 2.12.4 声明）。
  - FFmpeg：固定版本+来源+校验和+对应源码保存；审计**实际二进制**（buildconf/依赖/许可文本/源码获取说明）；`cargo deny` 仅覆盖 Rust 依赖，sidecar 另行核验；打包验证含 sidecar 与 DLL；许可筛查自 M0/M1 启动。
- REQ-30 Tauri 集成：单实例；托盘（开始/停止、退出）；设置持久化（输出目录、默认模式、fps、麦克风设备、编码器覆盖）；关闭主窗口不退出录制；HWND/PID 不作为长期身份（清单用 PID+创建时间+路径）。
- REQ-31 UI 通信：Channel **订阅入口**（传入 Channel + 当前状态快照）；UI 重载后重绑（发送失败不得中断录制）；告警与电平分通道/优先级（电平 ≤10Hz）；批次启动=全成功或回滚；`stop_route` 返回「已请求停止」，成品就绪经事件通知。

### 2.9 AC → REQ 追溯

| AC | REQ |
|---|---|
| AC-1 截图 | REQ-5, 6, 7, 8 |
| AC-2 按窗口隔离录制 | REQ-9, 14, 17, 18 |
| AC-3 多路同时录制 | REQ-12 |
| AC-4 音轨组织与输出模式 | REQ-4, 15, 16, 17, 18 |
| AC-5 合并 | REQ-23 |
| AC-6 纯录音 | REQ-19 |
| AC-7 目标枚举 | REQ-1, 2, 3 |
| AC-8 环境探测降级 | REQ-11, 28 |
| AC-9 稳定与同步 | REQ-10, 13, 25, 26, 27 |
| AC-10 异常恢复 | REQ-21, 22 |

## 3. 架构

### 3.1 组件与进程模型

```
┌──────────────────────── Tauri 主进程（Rust） ─────────────────────────────┐
│ UI(WebView) ◄─commands/Channel─► 会话管理 sr-core（时间轴唯一调度者）      │
│  sr-enum（枚举/身份解析）  sr-shot（截图）  sr-merge（合并）               │
│                                                                           │
│ 每路：WGC 采集线程 ─► 源时间戳→帧槽(CFR) ─► 缩放/转 NV12(固定画布) ─► 管道 │
│ 每音源：WASAPI 线程(MTA) ─► 样本槽/静音补齐/迟到裁切 ─►(rubato) ─► 管道   │
└────────────────┬──────────────────────────────────────────────────────────┘
                 │ 每路独立命名管道组（服务端 Rust 创建；Job 对象绑定 sidecar）
                 ▼
  ffmpeg sidecar ×N（锁定 LGPL 构建）= 编码 + 5s MKV 分段（强制闭合 GOP/IDR）
                 │ 停止：EOF → 排空 → 后台 concat（显式全轨映射）→ 成品校验
```

- 线程规则：UI 线程不做 COM/MTA；每 WASAPI 客户端线程内 `CoInitializeEx(MTA)`，**客户端线程内创建/使用/释放**（wasapi 类型 !Send，不做跨线程"Send 包装"传递）；跨线程仅传拥有所有权的数据与控制消息；WGC 线程运行消息泵。
- 启动协议（消除互等）：管道服务端建立 → spawn sidecar → 采集预热（预屏障样本丢弃）→ 编码器可用性确认 → 公共 T0 → 各输入并发供数；状态机显式区分「采集就绪/管道建立/编码器就绪/输出开始」，各自超时与回滚。
- 停止协议：停止生产 → 逐输入 EOF → 编码器排空（超时强杀）→ 后台收尾（进度/重试）→ 成品校验→自动清理原段（默认；设置『保留原始分段』开启时保留；失败不删原段）。
- 单路失败隔离：任何单路异常只影响该路（Job 对象管控）。

### 3.2 模块划分（workspace）

| 模块 | 职责 | 关联 REQ |
|---|---|---|
| crates/sr-core | 类型（TargetRef/RouteConfig/SessionManifest/Settings/错误枚举/状态枚举） | 全部 |
| crates/sr-enum | 窗口/显示器/音频会话枚举 + 应用身份解析（有界规则） | REQ-1..4, 14 |
| crates/sr-capture | WGC 会话、源时间戳帧槽节奏器、缩放/NV12、固定画布、管道写 | REQ-9..13 |
| crates/sr-audio | wasapi 客户端（线程内生命周期）、样本槽/静音补齐/迟到裁切、rubato、WAV 写 | REQ-14..19 |
| crates/sr-pipeline | sidecar 管理（启动/停止协议、Job 对象、队列、管道、分段、后台 concat、恢复扫描） | REQ-20..22, 24 |
| crates/sr-shot | 截图（显示器/窗口/区域+跨屏拼合+PNG） | REQ-5..8 |
| crates/sr-merge | 合并（异构多轨） | REQ-23 |
| src-tauri | commands、Channel 订阅、托盘、单实例、store | REQ-30, 31 |
| ui/ | 前端（框架待定；仅 commands/Channel） | 全部 |
| tools/tone_player | 测试助手（440/880/1200Hz 音调、闪光/节拍参考） | 测试 |
| tests/ (sr-itests) | L2 集成与半自动同步分析 | 测试 |

### 3.3 接口

**Tauri commands**
- `probe_capabilities() -> {os_build, encoders[], loopback_ok, mic_privacy, wgc_ok, canvas_limits}`
- `list_targets()` / `list_audio_sources()` / `list_audio_devices()`
- `start_routes(Vec<RouteConfig>) -> Result<Vec<RouteId>>`（**全成功或回滚**）
- `stop_route(route_id)`（返回「已请求停止」）/ `stop_all()`
- `start_audio_only(Vec<AudioSourceSpec>) -> Result<Vec<RouteId>>`
- `take_screenshot(ScreenshotRequest) -> PathBuf`
- `merge_recordings(Vec<PathBuf>, out)`（进度经 Channel）
- `recover_sessions() -> Vec<RecoverableSession>` / `finalize_session(id)` / `discard_session(id)`
- `get_settings()` / `set_settings(Settings)`

**Channel**：`subscribe(channel)` 返回当前状态快照；事件 `route_state{id,state}`、`route_level{id,track,rms}`（≤10Hz）、`route_error{id,code,message,severity}`、`route_finalized{id,path}`、`recovery_found{session}`；告警独立于电平流。

**命名管道协议**
- `\\.\pipe\screenrec_{session}_{route}_{v|a0..}`；Rust=服务端（先监听）→ sidecar 客户端读取。
- 视频：连续 NV12 定长帧（固定画布尺寸；CFR 由帧槽节奏器保证）。
- 音频：连续 f32le 48k 立体声（静音期写零样本；迟到包在写入前裁切）。
- 队列：按字节+时间双重限界；写线程可取消；阻塞写=背压（S1b 实测零丢帧、内存有界）；超阈值 → 告警/按 REQ-26 处理；不按帧数大排队。
- 管道缓冲：视频 **8MiB**、音频小缓冲（S1/S1b 冻结）；连接等待必须外置超时 ≥5s；`-thread_queue_size` 调优。

**ffmpeg 基准命令（骨架，S2/S5/S6 校准；每输入管道独立）**
```
ffmpeg -hide_banner -nostdin
  -f rawvideo -pix_fmt nv12 -s {W}x{H} -framerate {fps} -i \\.\pipe\..._v
  -f f32le -ar 48000 -ac 2 -i \\.\pipe\..._a0  [-f f32le ...]
  -map 0:v -map 1:a [-map 2:a ...]
  -c:v {encoder} -g {2*fps} -force_key_frames expr:gte(t,n_forced*{segment_secs})
  -c:a aac -b:a 192k
  -f segment -segment_time {segment_secs} -reset_timestamps 1 -segment_format matroska
  -segment_list {dir}/segments.ffconcat {dir}/out_%05d.mkv
```
收尾（后台）：`ffmpeg -f concat -safe 0 -i segments.ffconcat -map 0 -c copy -f matroska <final>.mkv`

### 3.4 数据模型

- `Settings { output_dir, fps: 30, cursor: true, default_audio_mode, mic_device: Option<String>, encoder: Auto|String, segment_secs: 5 }`
- `AudioSourceSpec { kind: App{pid, created_at} | System | Mic{device} }`
- `RouteConfig { target: Window(hwnd)|Monitor(idx)|Region(rect), video: Option<VideoCfg{canvas_w, canvas_h, fps, cursor}>, audio: AudioPlan{ mode: Isolated|AllTracks, sources: [AudioSourceSpec] } }`
- `SessionManifest v1 { version, session_id, created_at, start_qpc, routes: [{ route_id, target, identity: {pid, created_at, name, path}, streams[], segments: [{path, t_start, t_end, closed}], state, final: Option<path> }] }`（原子写；位于会话目录）
- 错误枚举：`E_TARGET_LOST / E_LOOPBACK_UNAVAILABLE / E_CONSENT_DENIED / E_ENCODER_UNAVAILABLE / E_PIPE_STALL / E_SIDECAR_EXIT / E_DISK_FULL / E_SEGMENT_CORRUPT / E_RECOVERY_FAILED / E_MERGE_INCOMPATIBLE(仅不可读/未被 MKV 接纳) / E_IDENTITY_AMBIGUOUS`

### 3.5 错误路径

| 场景 | 行为 |
|---|---|
| 目标窗口关闭 | 该路优雅停止，E_TARGET_LOST，保留分段可收尾 |
| resize/最小化/热插拔 | 固定画布缩放/补帧/重建（REQ-13）；重建失败仅停该路 |
| 音源 PID 消失 | 该轨告警+标记结束（不自动重绑） |
| loopback 不可用/被拒 | 路级 E_LOOPBACK_UNAVAILABLE，引导至 REQ-28 降级；**不改录系统混音** |
| 激活成功但无信号 | 检测流程（静音监测）判定并提示，不归因权限 |
| 编码器不可用 | 启动前探测；按 §9 T1 行为 |
| 管道阻塞/背压 | 有界队列→阶跃校正（REQ-26）；超限停该路 |
| sidecar 退出非零/被终止 | 该路 E_SIDECAR_EXIT，保留分段；其它路不受影响 |
| 磁盘不足（录制中/收尾中） | 安全停止全部/收尾失败可重试；保留原段 |
| 崩溃残留 | 启动扫描（排除活动文件）→ 幂等收尾或丢弃；E_RECOVERY_FAILED 时保留原段 |
| 合并输入不可读/不被 MKV 接纳 | 拒绝并说明（异构布局允许，不得误拒） |
| WAV 崩溃残留 | 头部重写+尾部裁切（RIFF 上限策略） |

## 4. 测试计划（REQ → 测试映射）

层级与执行：
- **L1（每次 PR，CI）**：纯逻辑单测 + **无设备管线集成**（合成 NV12/PCM → 命名管道 → ffmpeg → 分段/收尾/合并/故障注入；参数快照；许可脚本）。
- **L2（契约变更/里程碑/发版；交互式用户会话；固定 OS build/GPU/驱动/音频端点）**：真实采集、内容级验证（FFT/ffprobe/像素方差）、半自动同步分析。Session 0/服务模式不合格；环境缺失记 **BLOCKED**，SKIP 不算通过。
- **L3（发版人工签核）**：真实应用/权限/设备操作、长时录制、多视频轨播放器体验、观感。

| REQ | 测试（命名） | 层 |
|---|---|---|
| REQ-1 | `it_enum_windows_lists_visible_app_name` | L2 |
| REQ-2 | `it_enum_monitors_physical` | L2 |
| REQ-3 | `it_enum_refresh_reflects_open_close`；`it_enum_uwp_cloaked_filter` | L2 |
| REQ-4 | `it_list_active_audio_sources_snapshot` | L2 |
| REQ-5 | `it_screenshot_three_targets` | L2 |
| REQ-6 | `it_screenshot_mixed_dpi_negative_coords` | L2 |
| REQ-7 | `it_screenshot_window_occluded_and_minimized_error` | L2 |
| REQ-8 | `manual_region_select_multi_monitor` | L3 |
| REQ-9 | `it_single_route_frames_nonblank` | L2 |
| REQ-10 | `unit_pacer_source_timestamp_slots` | L1 |
| REQ-11 | `it_cursor_flag_and_border_degrade` | L2 |
| REQ-12 | `it_two_routes_isolated_stop_one`；`it_three_routes_stop_middle` | L2 |
| REQ-13 | `it_window_resize_canvas_stable`（含比例变化/最小化恢复/热插拔） | L2 |
| REQ-14 | `it_audio_application_isolated_fft`；`it_audio_identity_bounded_no_host_leak` | L2 |
| REQ-15 | `it_audio_device_loopback_rms` | L2 |
| REQ-16 | `it_mic_capture_present_or_skip` | L2 |
| REQ-17 | `it_audio_multitrack_ffprobe_fft`（隔离/全音轨两模式） | L2 |
| REQ-18 | `unit_silence_pad_late_packet_drop` | L1 |
| REQ-19 | `it_audio_only_wav_duration_and_repair` | L2 |
| REQ-20 | `unit_ffmpeg_args_snapshot`；`it_pipe_startup_no_deadlock`；`it_encoder_matrix_concurrent_encode`（S2 固化） | L1+L2 |
| REQ-21 | `it_segment_keyframe_closed_gop`；`it_segments_finalize_concat_alltracks`；`it_synthetic_segment_finalize_merge` | L2+L1 |
| REQ-22 | `it_crash_kill_matrix_recovery_state_machine`；`it_fault_injection_disk_full` | L2 |
| REQ-23 | `it_merge_heterogeneous_multivideo_mkv`（不同分辨率/帧率/长度） | L2 |
| REQ-24 | `unit_naming_sanitize_no_overwrite` | L1 |
| REQ-25 | `unit_epoch_unit_mapping`；`it_sync_content_10min`（半自动：闪光/节拍；开始/中段/结束/concat 后） | L1+L2 |
| REQ-26 | `unit_drift_hysteresis_limits`；`it_backpressure_step_correction` | L1+L2 |
| REQ-27 | `manual_soak_two_routes_10min`（含观测指标采集） | L3 |
| REQ-28 | `it_probe_permission_matrix_injected`（可注入故障钩子） | L2 |
| REQ-29 | `license_and_build_audit`（cargo deny + ffmpeg buildconf/哈希/源码） | L1 |
| REQ-30 | `manual_ux_tray_settings_single_instance` | L3 |
| REQ-31 | `unit_channel_snapshot_rebind`；`manual_ui_reload_streaming` | L1+L3 |

L2 验证手法：测试助手播放可区分音调（A=440Hz、B=880Hz、麦克风=1200Hz）→ 录制 → `ffprobe -show_streams` 校验轨道数/时长 → 各轨 FFT（`realfft`）断言（阈值经 S3 校准：参考电平/窗长/噪声底明确；麦克风声学串音不得误判进程隔离）；画面用带目标标识的测试画面+像素方差；同步用闪光/节拍内容分析（输出最大偏差与漂移趋势；ffprobe 时长一致不等于内容同步）。

## 5. 实施分解（TDD 里程碑）

规则：每任务先写测试（红）→ 实现（绿）→ 重构；里程碑验收 = 对应 L2 全绿（环境缺失记 BLOCKED）；M0 未过不进入 M3+ 冻结项。

| 阶段 | 内容（REQ） | 主要证据（红→绿） | 通过条件 |
|---|---|---|---|
| M0 Spike | S1–S6 + resize（§8） | build 哈希/命令/日志/样本/矩阵 | 各 Spike 验收标准达成；S2 失败→暂停并重评编码路径；S3 失败→仅走 REQ-28 降级。**状态：全部达成（2026-10-16，冻结见 §8）** |
| M1 骨架 | workspace、Tauri 壳、设置、枚举（REQ-1..4, 30, 31; 29 部分） | `it_enum_*`、`unit_channel_snapshot_rebind` | L1+L2 对应绿。**状态：达成（2026-10-16；REQ-30 手动项留 L3/M9）** |
| M2 截图 | REQ-5..8 | `it_screenshot_*` | L2 绿。**状态：达成（2026-10-16；负坐标子用例因本机无负方向显示器 BLOCKED，留多屏机器复跑）** |
| M3 单路视频 | 时间轴/清单/停止/恢复骨架（REQ-9..13, 20..22, 25 基线） | `unit_pacer_*`、`it_pipe_*`、`it_segment_*`、`it_crash_*` | L1+L2（单路）绿。**状态：达成（2026-10-16；L2 route 5 用例全绿；output_naming 笔误闭环 §9；遗留：E_TARGET_LOST 归 M4、Job 对象 M8）** |
| M4 音频隔离 | REQ-14, 17, 18, 26；28 部分 | `it_audio_application_isolated_fft`、`it_backpressure_step_correction` | AC-2 达成。**状态：达成（2026-10-16）——L1 全绿 + 全量 L2 复跑绿（serial-run3 4/4、parallel-run5 4/4、route-regress 5/5、pair 2/2）；修复叙事 docs/test-fix/2026-10-16-m4-audio-l2-av.md、m4-green-report §9.4** |
| M5 多路 | REQ-12（含三路停中） | `it_three_routes_stop_middle` 等 4 用例（m5-red-report） | AC-3 达成。**状态：达成（2026-10-17）——编排者独立复跑全绿：L1 exit 0、multi_route 4/4（38.1s）、route 5/5（18.9s）、audio_isolation 4/4（47.7s）；缺陷 A（canvas NV12 交错）/B（-b:v 12M）修复见 m5-green-report；注：曾有一份不实「完成」回报被独立复核否决（纪律：所有完成回报须磁盘+复跑复核）** |
| M6 全音轨/纯录音 | REQ-15, 16, 19 | `it_audio_multitrack_ffprobe_fft`、`it_audio_only_*` | L2 绿。**状态：达成（2026-10-17）——独立复跑：L1 exit 0、route 5/5、audio_isolation 4/4、audio_m6 3/4（mic 正路径因本机无麦克风设备 BLOCKED）、multi_route 4/4（GDI 中间态抖动经加固：置顶+双缓冲）；细节 m6-green-report.md §11** |
| M7 合并 | REQ-23 | `it_merge_heterogeneous_multivideo_mkv` | AC-5。**状态：达成（2026-10-17）——编排者独立复跑全绿：L1 exit 0、merge 1 passed、route 5/5、multi_route 4/4、audio_isolation 4/4；merge.rs 哈希复核不变；细节 m7-red-report.md / m7-green-report.md** |
| M8 探测/许可/收尾 | REQ-28, 29, 22（完整）、24 | `it_probe_*`、`license_and_build_audit` | L2+审计绿。**状态：达成（2026-10-17）——编排者独立复跑：audit 1 passed（cargo-deny 0.20.2）、workspace L1 exit 0、probe 1 passed、recovery 2 passed（48.4s）、回归 route 5/5、multi_route 4/4、audio_isolation 4/4、merge 1/1；细节 m8-red-report.md / m8-green-report.md** |
| M9 UI 成品化 | 前端：主窗（目标/音源/音轨模式/开始停止/多路状态）、截图流程、合并、设置、恢复提示、托盘接线 | 设计稿 + 前端构建 + 手测清单 | 可用 UI + 既有测试零回归。**状态：达成（2026-10-17）——ui/{api.js,app.js,index.html,styles.css} + docs/design/ui-spec.md；node --check 通过；headless Chrome 场景（恢复/纯录音/停一路/E_TARGET_LOST/degraded）0 控制台错误；命令层 check exit 0 + 15 L1 单测；L3 人工签核（托盘真实点击等）待用户** |
| M10 整备验收 | REQ-27 复验；全 AC 演练 | `manual_soak_*`、L3 清单 | AC-1..10 全绿 + @acceptor。**状态：达成（2026-10-18，见 M13）**——620s 双路 soak、610s sync 复跑绿；细节 m10-rehearsal-report / m10b-sync-report / m10b-fix-report |
| M11 整改（R1 退回项） | 区域覆盖层 UI、Timeline/Drift 接线、纯录音恢复、原子 safe_commit、编码器设置、托盘启停、NSIS 安装包 | `m11_*` 套件 | R1 五项退回清零。**状态：达成（2026-10-17）** |
| M12 整改（R2/R3 退回项） | 截图前隐藏覆盖层、safe_commit 竞态、rubato+flush；区域覆盖层 async 重入死锁根因修复（open_region_overlay）、P2 失败提示保留覆盖层、fmt 全绿、E2E 工具链（tools/e2e） | m12-a..h 报告、m12-d/m12n E2E 证据 | 区域 E2E 7/7、sync/soak 最终源码复跑绿、安装包 M12-F 自检 PASS；sync 间歇失败根因定性为会话/显示切换（m12-h，含 SR_DIAG_AV 门控诊断）。**状态：达成（2026-10-18）** |
| M13 P1 音频内容完整性 | WASAPI 补齐/裁切互抵修复：`pad_to_wall_clock` + 50ms 停滞阈值（slots.rs/wasapi.rs）；3 项确定性回归（stall_pad_policy.rs，红→绿验证） | m13-report、m13-content-integrity、m13-* 全套日志 | 610s 双路静音替换量 12.48–49.75s → **0.000s**（payload 满时长）；sync 99.6/93.3ms、soak 620s×2 padded=0/dropped=0；10 套短测 + 区域 E2E 全绿；安装包 M12-G（SHA256 A99BE175…05D29）自检 PASS。**状态：达成——acc-5 ACCEPTED（2026-10-18，按已声明环境验证范围）** |
| M14 成品成功后清理原段 | REQ-21/REQ-24 保留策略闭环：默认成功后递归删除 `.screenrec_tmp/<session>`；设置 `keep_segments`（默认 false）开启时保留原段；失败/崩溃路径不删 | route L1 单测 + m11_route_commit L2（默认清理 / keep=true 保留）+ 应用层真实验证 | 成功后会话目录消失（默认）/保留（keep=true）；失败路径断言不变。M14b（2026-10-18）：纯录音 `start_audio_only` 成功收尾纳入同一策略（默认清理会话目录 / `keep_segments=true` 保留；失败/崩溃不删）。M14c（2026-10-18）：**含恢复收尾**——`recover_sessions`→`finalize_session` 成品验证成功后同样按 `keep_segments` 清理/保留原段；删除失败仅日志、不判收尾失败；失败/崩溃路径语义不变。**状态：达成（2026-10-18）** |

## 6. Out of scope

PRD 非目标之外，实现层明确不做：摄像头画中画、HDR、ARM64、macOS/Linux、浏览器标签/同进程子源级音频拆分、剪辑/转码、直播、云分享、自动更新、MP4 输出、多语言 UI、全局热键（后续候选）、系统音频多端点、合并时的会话时间偏移、音源应用退出后的同名自动重绑、断电/文件系统损坏保证（仅进程级崩溃；另议）、多视频轨 MKV 在所有播放器同时显示的兼容性保证（提供播放器矩阵文档+L3 验收）。

## 7. 决策记录

| # | 决策 | 依据 |
|---|---|---|
| D1 | 视频捕获 `windows-capture` 2.0.1（MIT），仅取帧 | 其编码器仅单轨 MP4 |
| D2 | 音频 `wasapi` 0.26 + `rubato` 5.x；客户端线程内生命周期（!Send）；激活隔离等待（不挂启动） | 评审 4 |
| D3 | 每路 LGPL ffmpeg sidecar；Rust 唯一时间轴调度者；启动/停止协议显式状态机 | 评审 3/5；圆桌裁决 |
| D4 | 编码器链由 S2 实测矩阵裁决（**定稿：NVENC→QSV→AMF→h264_mf，钉 release 构建**）；`h264_mf` 仅在实证后入链、不得无条件兜底；无可用编码器行为按 §9 T1 | 评审 13；圆桌分歧裁决；S2 |
| D5 | 5s MKV 分段 + 强制闭合 GOP/IDR；损失 ≤5s+δ；收尾=concat demuxer+全轨映射 | 评审 6；圆桌 |
| D6 | resize：固定输出画布（缩放+黑边），段间参数恒定 | 圆桌裁决（§9 T3） |
| D7 | 同步：源时间戳→帧/样本槽；慢漂移（rubato）与阶跃校正分开；全轨内容级 ≤150ms | 评审 5；圆桌 |
| D8 | sidecar 生命周期=Job 对象绑定主进程；EOF 自愈仅作优化 | 圆桌恢复裁决 |
| D9 | 应用身份：默认后代树+有界提升+禁跨系统宿主+PID+创建时间 | 圆桌分歧裁决 a |
| D10 | 收尾后台化（进度/重试）；删除无条件 ≤10s 承诺 | 圆桌分歧裁决 c |
| D11 | 测试分层+准入：L1（含无设备集成）/L2（交互会话）/L3（发版签核） | 评审 12；圆桌 |
| D12 | 临时目录置于输出卷（同卷收尾）；文件名 sanitize+禁覆盖 | 评审 11 |
| D13 | Tauri 2.12.4；Channel 订阅+快照；告警与电平分通道 | 评审 9；研究 §8 |

## 8. Spike（M0 门禁，按此顺序）

| Spike | 范围 | 验收标准 |
|---|---|---|
| **S2 编码能力与许可（最先）** | 发行构建测 NVENC/QSV/AMF/`h264_mf`；初始化失败、并发会话上限、最低硬件；buildconf/许可/源码 | 产出「枚举/初始化/并发/关键帧控制/吞吐/许可」矩阵；达到产品承诺并发；无硬件场景有已验证路径或明确不支持契约 |
| **S1 管道/转换/启动** | 真实 NV12/f32le 多输入、同时录制、无声启动、顺序打开、慢读/阻塞/取消/stderr 压力、**resize**、全链路成本 | 无启动死锁；达目标分辨率/FPS；缓存与恢复时间有界；帧字节完整；可停止；单路阻塞不拖死其它路；记录转换/复制/管道成本 |
| **S3 真实音频隔离** | 目标/非目标音调；浏览器多窗口、多进程、UWP/宿主、PID 重启、最低承诺 OS | 目标信号存在、非目标泄漏满足预锁定指标（FFT 阈值校准并说明参考）；明确各 OS 行为与 consent 实际表现 |
| **S5 分段/崩溃/恢复** | 段首/中/尾强杀主进程、编码器、整树；满盘、损坏尾段、跨卷、收尾中再崩 | 闭合段全部独立解码；损失满足 5s+δ；恢复幂等且不读活动文件；全轨保留；边界无可检测爆音/跳变；失败不丢原段；记录收尾耗时/峰值空间 |
| **S6 端到端内容同步** | ≥10 分钟、多路多音轨、延迟首帧、无声启动、**背压注入**、concat 后复测 | 所有应同步轨对内容偏差 ≤150ms 不累积；检查开始/中段/结束/拼接边界；输出内容分析、校正事件、队列证据 |

（AAC priming/CodecDelay/负 PTS/边界包归属以 S5 实测为准，不预断。）

### M0 实测结论（冻结项，2026-10-16）

已执行 Spike（S2、S1、S1b、S3、S5）全部达成验收；S1 含 resize 与多输入并发；S6（端到端内容同步）并入 M3/M4 实施验证。冻结如下：

- **编码器链（D4 定稿）**：`h264_nvenc → h264_qsv → h264_amf → h264_mf`（启动逐级**实际初始化**探测）；sidecar 构建钉 **BtbN release 构建**（n8.1 系；master 构建因 NVENC API 13.1 新于驱动失败，release 构建实测可用）。`h264_mf` 存在且通过矩阵（软件 MFT，仅链末）。构建 LGPL v3、无 GPL 组件（libx264/x265 不存在）。
- **管道协议定稿（S1/S1b）**：视频管道缓冲 **8MiB**（16MiB 仅小幅削减冷启动阻塞，稳态无差异）；音频管道另设小缓冲；**阻塞写即背压**（零丢帧、内存/积压有界，迟帧单调可检测）；`ConnectNamedPipe` 必须外置超时（≥5s；取消用 `CancelIoEx`；取消后监听实例可原位复用）；**优雅停止只走 EOF**（实测 101ms 收尾、段完整、exit 0），`taskkill /F` 仅兜底；错误映射：232/109→E_SIDECAR_EXIT、995→自主取消、535→正常结束；写超时仅覆盖「活但不读」（3–5s → CancelIoEx → E_PIPE_STALL）。
- **分段/恢复（S5）**：强杀恢复成立、闭合段全部独立可播；δ 实测 **qsv 0.62s / h264_mf 1.5s**（损失 ≤5s+δ 成立）；命令修订并入 §3.3 基准命令（force_key_frames、concat `-map 0`、0 字节段排除、stderr 持续排空、QPC 域记账）。
- **音频隔离（S3）**：按进程环回隔离成立（非目标音调 −158/−160 dB @ 阈值 −40）；双客户端并发可行；静音期流不断（零样本、无 SILENT/DISCONTINUITY），补帧保持时长；须专用 MTA 采集线程（UI 线程激活报 RPC_E_CHANGED_MODE）；数十次激活 0 consent 弹窗。Win11 25H2 26200 实测；**Win10 19041 维持未验证标注（T2）**。
- **管线性能（S1）**：全链单路零迟帧 ≈6.6ms/帧；双/三路 30fps 通过（一次性调度尖峰 ≤263ms）；缩放（CPU 路径）≈1.0–1.2ms/帧；连接耗时 15–29ms。

## 9. 产品取舍（Gate 2 已确认，2026-10-15）

| # | 取舍 | 确认结果 |
|---|---|---|
| T1 | 无硬件编码器时行为 + 并发承诺 | 明确报错并列出可用编码器；保底 2 路 1080p30，第 3 路起按硬件能力探测（超限明示） |
| T2 | Win10 2004+ 隔离功能承诺 | 程序与降级路径先承诺；「按应用隔离」Win11 已实测通过（S3）；Win10 19041 维持「验证中」标注至实机验证 |
| T3 | resize 契约 | 固定输出画布（缩放+黑边） |
| T4 | 崩溃损失边界 | ≤5s+δ；δ 实测冻结：qsv 0.62s / h264_mf 1.5s（S5） |
| T5 | 收尾策略 | 停止采集即时响应 + 后台收尾（进度/重试）；删除无条件 ≤10s |
| T6 | 是否允许非 H.264 输出（如 VP9 软编兜底） | 暂不允许（v1 仅 H.264；S2 失败后再议） |

其余 PRD 开放事项（摄像头/HDR/ARM64/开源/MP4/全音轨自动发现扩展）不阻塞实现，验收期决定。

## 附录 A：评审意见采纳对照（r1 → r2）

| 评审项（P0） | 落点 |
|---|---|
| 编码器/许可（P0-1） | REQ-20, 29；D4, D13；S2 |
| 启动/IPC/进程管理（P0-2） | REQ-20（启动/停止协议）；§3.1 |
| 时间轴/同步（P0-3/4） | REQ-25, 26；§3.1 |
| resize 生命周期（P0-5） | REQ-13；S1 |
| 应用身份解析（P0-6） | REQ-14；D9 |
| 分段与恢复（P0-7） | REQ-21, 22；D5, D8；S5 |
| 停止/合并（P0-8） | REQ-21, 23；D10；§9 T5 |
| 测试治理（P0-9） | §4（准入/BLOCKED/三路停中）；D11 |
| 不采纳/降级 | EOF 自愈不作保证（D8）；不默认叠加 `aresample=async`；512 帧队列改为字节+时限界；wallclock/itsoffset 不作默认修复方案 |
