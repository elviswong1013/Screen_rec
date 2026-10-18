# M10 整备验收演练报告（全量回归 + 10 分钟双路 soak + L3 清单盘点）

- 日期: 2026-10-17（所有命令/采样均为本机实际执行；证据文件见 §9）
- 角色: **executor-backup 承担演练**（沿 M5–M9 先例；本轮**无生产代码改动**）
- 依据: specs/001-screen-recorder.md §5 M10 行、§4（测试映射与 BLOCKED 机制）、§9 T1（保底 2 路 1080p30）、
  REQ-27（2 路 1080p30 ≥10 分钟）；docs/milestones/m9-commands-report.md（命令契约/未决项）、
  docs/design/ui-spec.md（UI 状态机/L3 交互面）
- 约束: 未修改任何既有测试；**新增** `tests/sr-itests/tests/soak.rs`（任务点名的 L3 用例）；
  未改 `crates/`、`specs/`、`docs/spikes/`；未 git 提交。为 GUI 验证重建了 `target/debug/screen-rec.exe`
  （构建产物，不入库）。

## 0. 结论摘要

| 项 | 结论 |
|---|---|
| L1 `cargo test --workspace` | **exit 0；24 passed / 0 failed / 31 ignored**（31 = 既有 30 个 L2 门 + 新增 soak） |
| L2 全量（10 套件 / 30 用例，串行） | **28 passed + 2 环境性 BLOCKED**（audio_m6 麦克风、shot 负坐标），与任务预告一致，未伪绿 |
| 10 分钟双路 soak（REQ-27） | **通过**：2 路 × 620s、1080p30；两路成品 620.300s / 620.933s、18606 / 18625 包、收尾 0.71s / 0.68s；无崩溃、无 sidecar 残留 |
| 独立 ffprobe 复核（锁定 n8.1） | 两路均 1 条 h264 1920x1080@30 轨；整文件解码 exit 0（stderr 空） |
| L3 清单 | **2 项本机已自动化验证**（关闭窗口隐藏、单实例聚焦）+ **1 项截图判读**（真实 UI 渲染/后端连接）；其余 **7 项**需用户人工 |
| 验收判定 | AC-1..8、AC-10 绿（含 2 项环境 BLOCKED 记录）；**AC-9 仅稳定性达成，内容级同步 ≤150ms 未验证**（REQ-25 两个测试缺失，见 §7-1） |
| 遗留 | 1 项实质缺口（REQ-25 同步测试）+ 1 项测试映射缺口（REQ-13 resize L2）+ 2 项环境 BLOCKED + M9 结转项（§8） |

> **关于本报告中的数字**：soak 日志由测试进程以 UTF-8 写出；若经 PowerShell 控制台转显，中文相邻字节可能
> 吞掉紧随其后的 1 位 ASCII 数字（本次表现为日志行显示的 `6850709` 实际文件内容为 `26850709`）。
> 本报告一律采用**日志文件原文 + 磁盘实测 + 独立 ffprobe** 三方一致的值（见 §4.3）。

## 1. 环境（固定并记录）

| 项 | 值 | 证据 |
|---|---|---|
| OS | Windows 11 25H2，`Microsoft Windows [Version 10.0.26200.9711]` | 各 L2 日志 `[env] OS:` 行 |
| 会话 | 交互式控制台会话：`qwinsta` → `>console elvis 1 Active`；进程 SessionId=1；`winsta=WinSta0\Default`；`USEROBJECTFLAGS.dwFlags=1 (WSF_VISIBLE)` | 本次演练前独立核验 |
| `SESSIONNAME` | **agent 进程未继承该环境变量**（Process/User/Machine 三作用域均为空）→ 为满足 §4 前置检查，L2 运行时显式注入 `SESSIONNAME=Console`。注入前已用上表证据确认确为交互式会话；测试内部仍自行做 WGC/编码器/设备真实检查（probe=Ok、route 实际录制佐证），不构成伪绿 | 同上；`m10-verify-probe.log` |
| 显示器拓扑（DEVMODE 物理像素） | `(0,0) 3840x2160 主`；`(3840,442) 1440x2560`（无负方向显示器） | `m10-verify-shot.log` `[mixdpi] 拓扑` |
| DPI 缩放 | 主屏 **150%**、副屏 **125%**（由 DEVMODE 物理分辨率与 PowerShell Screen API 逻辑坐标之比得出：3840/2560、1440/1152） | `[System.Windows.Forms.Screen]` 对照 |
| 编码器 | D4 链首个可用 = **h264_nvenc**；canvas 上限 7680x4320 | `m10-verify-probe.log` |
| 音频 | process loopback **Ok**；麦克风 **Unavailable（无采集设备）**；WGC **Ok** | `m10-verify-probe.log` |
| 锁定 ffmpeg | `ffprobe version n8.1.3-18-g46d899d7a0-20261015`（BtbN release，§8 冻结） | `m10-soak-ffprobe.log`（`ffprobe -version` 行） |

## 2. 全量回归（全串行，逐套件独立 cargo 调用，`--test-threads=1`）

### 2.1 L1（每次 PR 门）

| 命令 | 结果 | 日志 |
|---|---|---|
| `cargo test --workspace` | **exit 0；24 passed / 0 failed / 31 ignored**（`screen_rec_lib` 15、drift 1、silence 1、pacer 1、channel 1、ffmpeg_args 1、audit 1、naming 2、safe_commit 2） | `m10-verify-l1-workspace.log` |

唯一 warning 为既有 `tests/sr-itests/tests/probe.rs` 的 `unused import: EncoderProbe`（M8 绿报告 §5-9 已记录，
非本轮引入；`tests/` 不在本轮改动范围）。

### 2.2 L2（逐套件，`-- --ignored --nocapture --test-threads=1`）

| # | 套件 | 用例数 | 结果 | 耗时 | 日志 |
|---|---|---|---|---|---|
| 1 | route | 5 | **5 passed** | 60.38s | `m10-verify-route.log` |
| 2 | multi_route | 4 | **4 passed** | 37.49s | `m10-verify-multi_route.log` |
| 3 | audio_isolation | 4 | **4 passed** | 45.88s | `m10-verify-audio_isolation.log` |
| 4 | audio_m6 | 4 | **3 passed + 1 BLOCKED**（麦克风） | 34.76s（exit 101=BLOCKED 门） | `m10-verify-audio_m6.log` |
| 5 | merge | 1 | **1 passed** | 0.89s | `m10-verify-merge.log` |
| 6 | shot | 4 | **3 passed + 1 BLOCKED**（负坐标） | 14.64s（exit 101=BLOCKED 门） | `m10-verify-shot.log` |
| 7 | enum_targets | 4 | **4 passed** | 0.73s | `m10-verify-enum_targets.log` |
| 8 | enum_audio | 1 | **1 passed** | 1.27s | `m10-verify-enum_audio.log` |
| 9 | probe | 1 | **1 passed** | 1.88s | `m10-verify-probe.log` |
| 10 | recovery | 2 | **2 passed** | 46.90s | `m10-verify-recovery.log` |

合计：**30 用例 = 28 passed + 2 BLOCKED（环境）**。逐用例名与 §4 映射见 §2.4 与 §7。

### 2.3 两项环境性 BLOCKED（如实记录，不伪绿）

| 用例 | 实际发生 | 已完成的部分 | 结论 |
|---|---|---|---|
| `audio_m6::it_mic_capture_present_or_skip` | `BLOCKED: 本机当前无麦克风设备（list_capture_devices 为空）` | 该用例先验证了 REQ-16 的**显式不可用语义**（无设备/设备不存在 → `MicUnavailable` 且 ≤5s 快速返回，不静默成功），再 BLOCKED | 麦克风正路径（设备存在 → 1200Hz 声学捕获）`需用户` |
| `shot::it_screenshot_mixed_dpi_negative_coords` | `BLOCKED: 本机拓扑无负方向显示器（全部显示器位于桌面原点 (0,0) 的正方向）` | **跨屏子用例已实际通过**：A=3840x2160@(0,0) × B=1440x2560@(3840,442) → rect=(3640,1151) 400x300，输出物理尺寸/回执一致 | 负坐标子用例 `需用户`（多屏负方向布局机器） |

其余 8 套件 28 用例全绿，含：REQ-12 三路停中/单路故障隔离/批量回滚、REQ-14 应用隔离 FFT + 有界提升无宿主泄漏、
REQ-15 设备环回、REQ-17 多轨、REQ-19 WAV 崩溃修复、REQ-22 强杀矩阵/Job 回收/磁盘满注入、REQ-23 异构合并、
REQ-28 注入矩阵 + 真实探测。

### 2.4 关键实测值（L2）

- **probe**：`os=10.0.26200.9711 supported=true selected=Some("h264_nvenc") loopback=Ok mic=Unavailable wgc=Ok canvas=7680x4320`。
- **recovery**：崩溃会话收尾成品 10.100s 可解码、原段保留、幂等、丢弃；编码进程被杀 → `E_SIDECAR_EXIT` → 同进程恢复收尾；磁盘满注入 → `E_DISK_FULL`（stderr 含 Permission denied），解除后可重试收尾。
- **multi_route**：两路隔离/停一路、三路停中、批量回滚、单路 sidecar 强杀隔离全部通过（标记色串扰 0.0000）。
- **route**：关键帧/闭合 GOP、闭合段独立可播、强杀段可播、光标开关降级。

## 3. 全量回归的判定

- L1：**全绿**（任务验收口径）。
- L2：**全部绿或按 §4 记录 BLOCKED**（2 项，均属环境缺失，且各自的可行部分已执行）。
- 既有测试零回归：本轮未改任何既有测试；L1 数量与 M9 结束时一致（24 passed），L2 逐套件数量与 M8/M9 记录一致。

## 4. 10 分钟双路 soak（REQ-27 / §9 T1）

### 4.1 设计与执行

- 新增 `tests/sr-itests/tests/soak.rs::manual_soak_two_routes_10min`（`#[ignore]`，L3）：2 路窗口目标
  （自建动画窗口，自带泵线程 33ms 重绘）、固定画布 **1920x1080@30**、纯视频（REQ-27 不要求音轨）、
  各自独立输出目录；录制 **620s**（≥600s 保底 + 余量）。
- 运行方式：直接执行测试二进制（`soak-df5b5d834c2a79f4.exe --exact manual_soak_two_routes_10min --ignored --nocapture --test-threads=1`），
  由外部 PowerShell 采样器每 30s 记录测试进程 RSS/CPU/线程/句柄、2 个 ffmpeg sidecar 的 RSS/CPU、两路输出目录体积。
- 测试自身判据：两路 `wait_finalized` 成功；成品存在/MKV/恰 1 条 h264 轨/画布与帧率恒定；时长≈录制段（±3s）；
  视频包数≈时长×fps；整文件解码 stderr 为空；起始/中段/收尾各取一帧（均非黑 + 至少一对帧差 ≥2，证明内容贯穿）；
  收尾后本进程无 ffmpeg 子进程残留。

### 4.2 soak 结果（测试进程自测，`m10-soak.log`）

| 指标 | 路 A | 路 B |
|---|---|---|
| 录制段（request_stop 时刻） | **620.092s** | **620.801s** |
| 成品容器时长 | **620.300s**（偏差 +0.208s） | **620.933s**（偏差 +0.132s） |
| 视频包数 / 期望（时长×30） | **18606 / ≈18609** | **18625 / ≈18627** |
| 收尾耗时（含 concat） | **0.71s** | **0.68s** |
| 成品字节数（日志原文） | **26,850,709** | **26,883,699** |
| 起始/中段/收尾 luma std | 58.81 / 57.87 / 46.10 | 57.90 / 59.23 / 60.35 |
| 起/中/尾最大帧差 | 16.33 | 21.42 |
| 成品路径 | `%TEMP%\sr_m10_soak_a_23828\20261017_115454_SR-M10-SOAK-A-23828.mkv` | `%TEMP%\sr_m10_soak_b_23828\20261017_115454_SR-M10-SOAK-B-23828.mkv` |

- 两路成品合计 **51.25 MiB**（26,850,709 + 26,883,699 B）。
- 测试结果行：`test result: ok. 1 passed; 0 failed; 0 ignored; finished in 631.48s`；测试进程完成并退出，无 panic
  （启动器脚本未记录到 exit code（PowerShell 进程对象取值为空），以测试结果行与 `WaitForExit` 返回为准）。
- 收尾后 10s 内 `ffmpeg` 子进程清空（无孤儿）；输出目录各 ≈51.3 MiB（成品 + 会话段；REQ-22 原段永不删除）。

### 4.3 独立复核（锁定 ffmpeg/ffprobe，`m10-soak-ffprobe.log`）

| 项 | 路 A | 路 B |
|---|---|---|
| 流数 | **1**（仅视频） | **1**（仅视频） |
| 编码 / 分辨率 / 帧率 | **h264 / 1920x1080 / 30/1** | **h264 / 1920x1080 / 30/1** |
| 容器时长 | **620.300000s** | **620.933000s** |
| 文件大小 | **26,850,709 B** | **26,883,699 B** |
| 整文件解码（`-v error -f null -`） | **exit 0，stderr 空**（3.97s） | **exit 0，stderr 空**（4.04s） |
| SHA256 | `55E431D8D6F3B8B2756F17B9527BC0D4F2F60401FC50AEF761F5D56E4CDAC510` | `51EF59AF74DA1636B79F43CC59C64B2965D587B7D0A937165BFF30A014211565` |

> **数字口径说明**：soak 日志行中的字节数经 PowerShell 控制台转显时，UTF-8 中文与相邻 ASCII 数字被
> 误按 GBK 解码，`26850709` 显示为 `6850709`（首位 `2` 被吞）。**日志文件原文、磁盘 `Get-Item`、
> ffprobe `format.size`、测试总计 `51.25 MiB` 四方一致**，以 26,850,709 / 26,883,699 B 为准。

### 4.4 资源采样趋势（22 次采样，30s 间隔，`m10-soak-samples.csv`）

| 指标 | 稳态（60.1s–601.1s，19 样本） | 判读 |
|---|---|---|
| 测试进程 WorkingSet | **122.3 → 126.8 MB**（区间 114.7–126.8；30s 样本 114.2） | 无单调增长（601s 较 60s 仅 +4.5 MB，属帧池/管道缓冲与堆碎片抖动） |
| 测试进程 Private | **129.3 → 133.5 MB**（区间 121.7–133.5；30s 样本 121.3） | 同上，无泄漏趋势 |
| 测试进程 CPU | **avg 75.6%** 单核（min 67.7 / max 83.9） | WGC 捕获 + NV12 转换 + 管道写；平稳 |
| 测试进程线程 / 句柄 | **99–103 / 730–742**（首样本 106 线程为启动态） | 无持续增长 |
| 2 个 ffmpeg sidecar 合计 RSS | **625.8 → 628.9 MB**（≈313 MB/进程；区间 625.8–628.9） | 平稳（编码器 + 8MiB 管道缓冲） |
| 2 个 ffmpeg sidecar 合计 CPU | **avg 7.1%** 单核（6.3–8.0；60.1–571.0 差分） | NVENC 硬件编码，负载低 |
| 每路输出目录体积 | 0 → **51.3 / 51.4 MiB** | 线性增长；60s 时 2.3 MiB/路（静态内容低码率） |
| 停录后样本（631.1s） | WS 39.0 MB、线程 31、ffmpeg 1 个（一个已完成收尾） | 资源随收尾释放 |

**未发现异常**：无掉帧告警（包数≈期望）、无内存持续增长、无句柄/线程泄漏、无 sidecar 残留、收尾耗时 <1s。

### 4.5 soak 的边界（如实说明）

- 纯视频路：REQ-27 只要求 2 路 1080p30 稳定性；**A/V 内容级同步（REQ-25 ≤150ms）不在本 soak 覆盖范围**，
  且该同步测试本身尚未实现（§7-1）。
- 采样为 30s 粒度，可能错过亚秒级尖峰；CPU 为「单核百分比」（可 >100%）。
- 采样器读取的 RSS 为 Windows WorkingSet，未做堆级分析。

## 5. L3 清单盘点（本机可验 vs 需用户人工）

| # | L3 项 | 本机可验？ | 结果 / 证据 | 责任 |
|---|---|---|---|---|
| 1 | **关闭主窗口隐藏到托盘且进程存活**（录制不中断的结构前提） | 本机可验（Win32 自动化） | **已验**：向主窗口 `WM_CLOSE` 后 `process_alive=True`、`window_visible=False`；stderr 0 行（`m10-gui-l3.log` 12:06:38） | 已完成 |
| 2 | **单实例：二次启动聚焦已有窗口** | 本机可验（Win32 自动化） | **已验**：第二实例 20s 内退出 `exit=0`，第一实例存活，`GetForegroundWindow()==instance1 hwnd`（同为 12:06:36） | 已完成 |
| 3 | **真实 UI 渲染与后端连接**（状态/能力/列表来自真实命令） | 本机可验（截图判读） | **已验（截图）**：`m10-gui-main-window-full.png` 显示「已连接后端」、OS build、`编码器 h264_nvenc`、`麦克风不可用` 降级横幅（AC-8）、`窗口 38 / 显示器 2`、设置值 1920x1080/30fps/隔离/输出目录；两次启动窗口计数 42→38（实时枚举） | 已完成（判读） |
| 4 | 托盘图标真实点击（显示/退出） | 需用户 | 托盘图标交互无法可靠自动化；本轮未点击。注：`quit` 直接退出（M9 遗留的「录制中退出分级」未实现） | 需用户 |
| 5 | 真实 GUI 操作全链路（选目标→开始→停止→成品卡片→截图→合并→设置保存→恢复横幅两段确认） | 需用户 | 本轮仅启动并截图（#3），**未执行点击/键盘交互**（避免在验收机留下录制产物并保持人工签核语义） | 需用户 |
| 6 | 负坐标截图（REQ-6 子用例） | 需用户（需多屏负方向拓扑） | `shot` 子用例 BLOCKED；跨屏部分已通过 | 需用户 |
| 7 | 麦克风采集（REQ-16 正路径） | 需用户（需采集设备 + 声学路径） | `audio_m6` mic BLOCKED；`probe.mic=Unavailable`；UI 正确显示降级提示 | 需用户 |
| 8 | 多视频轨 MKV 播放器兼容性（播放器矩阵/默认轨选择/观感） | 需用户 | `merge` 已产出异构多轨 MKV（L2 绿）；播放器兼容性与观感属主观/设备相关验收 | 需用户 |
| 9 | 录制中关闭主窗口不中断录制（端到端手测） | 需用户 | 结构前提已验（#1 + 命令层后台线程模型），但「录制中关闭」需真实录制交互 | 需用户 |
| 10 | 托盘退出时的录制中行为分级 | 需用户 | 当前 `quit` 直接退出（sidecar 由 Job 回收、残留会话可恢复）；分级交互未实现 | 需用户 |

**本机自动化脚本**（`C:\Users\elvis\AppData\Local\Temp\sr_m10_gui_l3.ps1` / `sr_m10_gui_capture.ps1`，一次性工具，未入库）：
启动实例 1 → 等主窗口 → 截图（DPI-aware 与普通两种）→ 启动实例 2 → 校验退出与前台句柄 → `WM_CLOSE` → 校验隐藏/存活 → 强杀清理。
结果：`instance1 stderr lines=0`、`leftover screen-rec processes=0`；另经独立进程检查 `ffmpeg=0`（无 sidecar/编码残留）。

## 6. AC → 证据矩阵（演练判定）

| AC | 覆盖 | 判定 |
|---|---|---|
| AC-1 截图 | `shot` 3 passed（三目标/窗口遮挡+最小化错误/显示器物理尺寸）+ 跨屏区域实测；负坐标子用例 BLOCKED；区域框选手测属 L3 | **绿（含 1 项环境 BLOCKED）** |
| AC-2 按窗口隔离录制 | `audio_isolation` 4 passed（应用隔离 FFT / 身份有界无宿主泄漏 / 多轨 / 背压阶跃）；`route` 5 passed | **绿** |
| AC-3 多路同时录制 | `multi_route` 4 passed（两路停一/三路停中/批量回滚/单路故障隔离）+ soak 2 路 620s | **绿** |
| AC-4 音轨组织与输出模式 | `audio_isolation::it_audio_multitrack_ffprobe_fft`、`audio_m6` 3 passed（设备环回/全音轨/纯录音） | **绿** |
| AC-5 合并 | `merge::it_merge_heterogeneous_multivideo_mkv` passed | **绿** |
| AC-6 纯录音 | `audio_m6::it_audio_only_wav_duration_and_repair` passed | **绿** |
| AC-7 目标枚举 | `enum_targets` 4 passed + `enum_audio` 1 passed | **绿** |
| AC-8 探测与降级 | `probe` 1 passed（注入矩阵 ×4 + 真实 smoke）；UI 降级横幅截图；麦克风正路径 BLOCKED（环境） | **绿（设备项 BLOCKED）** |
| AC-9 稳定与同步 | 稳定性：soak 2×620s 通过；**内容级同步 ≤150ms：无测试（REQ-25 两个用例缺失）**；resize L2 亦缺失（REQ-13） | **不完整 / 未验证** |
| AC-10 异常恢复 | `recovery` 2 passed（强杀矩阵/Job 回收/幂等/磁盘满）+ 真实子进程强杀 | **绿** |

> **验收结论**：AC-1..8、AC-10 达成；**AC-9 仅完成稳定性面，内容级同步未验证** → 按 M10 行「AC-1..10 全绿」
> 的门槛，本轮**不能宣称为全绿**，需补 REQ-25 测试后再验收（§7-1/§8-P0）。

## 7. §4 测试映射 vs 实际测试清单（M10 盘点）

| # | 差异 | 说明 / 影响 |
|---|---|---|
| 7-1 | **REQ-25 的 `unit_epoch_unit_mapping` 与 `it_sync_content_10min` 不存在** | 全仓检索无该测试实现；`pacer_cfr.rs` 文档明确把它们列为范围外。**AC-9 内容级同步无证据**（最高优先级缺口） |
| 7-2 | **REQ-13 的 `it_window_resize_canvas_stable` 不存在** | 无 L2 resize/最小化恢复/热插拔测试；S1 spike 曾实测 resize（固定画布+黑边），但无回归门 |
| 7-3 | 命名差异（覆盖等价） | §4 `unit_pacer_source_timestamp_slots` → 实为 `unit_pacer_cfr_dup_drop`；`it_cursor_flag_and_border_degrade` → `it_cursor_toggle_degrades`；`unit_naming_sanitize_no_overwrite` → `unit_output_naming` |
| 7-4 | 同名测试缺失但能力被其它用例覆盖 | REQ-20 无死锁启动、REQ-21 关键帧/闭合 GOP/独立可播由 `route` 的 `it_route_pipeline_mkv_valid` / `it_segments_finalize_concat` / `it_crash_kill_segments_playable` 覆盖；`it_segments_finalize_concat_alltracks` 由 `multi_route` 音频轨断言覆盖 |
| 7-5 | §4 手动项未落地为自动化 | `manual_region_select_multi_monitor`、`manual_ui_reload_streaming`、`manual_ux_tray_settings_single_instance` 属 L3（§5 已盘点） |

## 8. 遗留问题（按优先级）

| # | 优先级 | 事项 | 影响 / 建议 |
|---|---|---|---|
| 8-1 | **P0** | REQ-25 同步测试缺失（`unit_epoch_unit_mapping` + `it_sync_content_10min`） | AC-9 内容级同步无法验收。需 test-writer 按 §4 手法（闪光/节拍 + 内容分析）补 L1+L2；另需覆盖 concat 边界与 10 分钟不累积 |
| 8-2 | P1 | REQ-13 resize L2 缺失 | 固定画布/最小化补帧/热插拔无回归门；S1 结论不足以作为验收证据 |
| 8-3 | P1 | 麦克风正路径环境缺失 | 需有采集设备（且扬声器—麦克风声学耦合）的机器复跑 `audio_m6` mic 用例；本机 `probe.mic=Unavailable` |
| 8-4 | P1 | 负坐标截图环境缺失 | 需负方向布局的多屏机器复跑 `shot` 子用例；跨屏路径已通过 |
| 8-5 | P2 | `route_level` 无发布者（M9 §8-1） | 电平事件契约已定但管线无 RMS 读数；UI 已显示「电平待管线接入」。需管线 API |
| 8-6 | P2 | 区域录制未实现（M9 §8-2） | `kind:"region"` 透传 `E_TARGET_LOST`；UI 不提供入口。属 M3 遗留范围 |
| 8-7 | P2 | 合并无进度事件（M9 §8-3） | UI 使用不定态进度条 |
| 8-8 | P2 | 托盘「开始/停止」与退出分级未接线（M9 §8-10） | 托盘仅「显示/退出」；`quit` 直接退出（sidecar 由 Job 回收，残留会话可恢复） |
| 8-9 | P3 | 显示器 `name` 为合成值、恢复 UX 边界（回滚空会话/已收尾会话仍列出/输出目录变更）、订阅线程生命周期、合并父目录需存在（M9 §8-4/6/7/8/9/11） | 详见 m9-commands-report.md；不影响本轮验收，UI 已有对应规则/提示 |
| 8-10 | P3 | L3 人工项 7 项（§5 #4/#5/#6/#7/#8/#9/#10） | 需用户在真实桌面完成点击/设备/播放器矩阵/负坐标多屏验证 |

## 9. 证据路径（全部在 `docs/milestones/`）

| 文件 | 内容 |
|---|---|
| `m10-verify-l1-workspace.log` | L1 全量（exit 0；24 passed） |
| `m10-verify-route.log` / `-multi_route.log` / `-audio_isolation.log` / `-audio_m6.log` / `-merge.log` / `-shot.log` / `-enum_targets.log` / `-enum_audio.log` / `-probe.log` / `-recovery.log` | L2 逐套件原始输出（含 BLOCKED 原文） |
| `m10-soak.log` / `m10-soak.stderr.log`（空） | soak 测试输出（620s 打点、两路指标、结论） |
| `m10-soak-launcher.log` | soak 启动/结束（pid 23828，wall 661.3s 含采样粒度） |
| `m10-soak-samples.csv` | 22 次 30s 采样（RSS/Private/CPU/线程/句柄/ffmpeg/磁盘） |
| `m10-soak-ffprobe.log` | 独立 ffprobe 复核 + 解码 + 版本 + SHA256 |
| `m10-gui-l3.log`、`m10-gui-instance1.stderr.log`（空） | GUI L3 自动化（单实例/焦点/关闭隐藏/清理/截图记录） |
| `m10-gui-main-window.png` / `m10-gui-main-window-full.png` | UI 截图（普通 / DPI-aware 完整窗口） |

soak 产物保留（未删除，供独立复核）：`%TEMP%\sr_m10_soak_a_23828\`、`%TEMP%\sr_m10_soak_b_23828\`
（各含成品 MKV + `.screenrec_tmp/<session>` 原始段；REQ-22 原段不删除）。

## 10. 证据分级

- **实际执行（observed）**：§2 全部 cargo 命令与计数、§4 soak 测试与外部采样、§4.3 独立 ffprobe/解码/HASH、
  §5 #1–#3 的 Win32 自动化与截图、进程/残留检查。
- **文档记载（documented）**：§1 编码器/环回/WGC 结论来自本轮 `probe` 实测输出；UI 设计/命令契约引自
  docs/design/ui-spec.md 与 m9-commands-report.md。
- **推断（inferred）**：§6 AC 矩阵的覆盖关系（由用例断言内容映射，非形式化追溯）；§5 #9「录制中关闭不中断」
  的结构性前提（未做录制中的 GUI 端到端手测）；§8 的优先级排序。

## 11. 本轮变更

| 文件 | 变更 |
|---|---|
| `tests/sr-itests/tests/soak.rs`（新增） | L3 `manual_soak_two_routes_10min`（2 路 1080p30 × 620s，自带动画窗口/ffprobe/解码/内容采样/sidecar 残留断言） |
| `docs/milestones/m10-*` | 本报告 + 上述日志/CSV/PNG |
| `target/debug/screen-rec.exe` | 仅重新构建（embed 当前 UI），未改源码 |

未修改：既有测试、`crates/`、`src-tauri/` 源码、`specs/`、`docs/spikes/`。未 git 提交。
