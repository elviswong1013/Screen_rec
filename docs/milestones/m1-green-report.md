# M1-C 绿实现报告（TDD 绿阶段收尾：目标枚举 REQ-1..4 + Channel 语义 REQ-31）

- 日期: 2026-10-16
- 规格: specs/001-screen-recorder.md §2.1（REQ-1..4）、§2.8（REQ-31）、§3.2/§3.3/§3.4、§4、§5（M1 行）
- 红阶段基线: docs/milestones/m1-red-report.md（§3 契约、§9 缺陷修订）
- 角色说明: 绿实现由先前的 M1-C 执行者会话落盘（会话因宿主问题取消、未留终报）；测试作者已完成独立核验（两处测试缺陷修复正确、同类 UB 无其它悬挂点、`cargo test --workspace` exit 0、enum_targets 4 绿 ×4 连跑、enum_audio 1 绿）。本报告由收尾会话完成：**全量实现审查 → 本机复跑全部 L1+L2 留证 → 本记录**。
- 环境: Windows 11 25H2，build **10.0.26200.9457**，SESSIONNAME=Console（交互式用户会话）；cargo/rustc **1.99.0 msvc**（rust-toolchain.toml 锁定，%USERPROFILE%\.cargo\bin）。

## 1. 范围

M1-C 收尾任务（只做必要最小修正）：

1. 全量盘点自 M1-A 骨架以来全部新增/修改文件，确认无未预期文件；
2. 逐文件代码审查（对照红报告 §3 契约与 REQ-1..4/31）；
3. 本机复跑 `cargo check --workspace`、`cargo test --workspace`、L2 enum_targets/enum_audio 并留证；
4. 补写本绿阶段报告。

约束遵守：**未修改任何测试文件**（enum_targets.rs / enum_audio.rs / common/mod.rs / channel_snapshot_rebind.rs / sr-itests Cargo.toml 全部冻结，逐字未动）；未改 specs/、docs/ 既有内容、tools/spikes；未 git 提交。

## 2. 变更文件盘点（自 M1-A 骨架以来；仓库尚无提交，全部为工作树新增）

### 2.1 生产实现（M1-C 绿实现落盘内容）

| 文件 | 状态 | 内容 |
|---|---|---|
| `crates/sr-core/src/lib.rs` | 修改 | 挂载 `pub mod error; pub mod events;`；types/state 仍为占位（后续里程碑） |
| `crates/sr-core/src/error.rs` | 新增 | 统一错误枚举（§3.4，见 §4.1） |
| `crates/sr-core/src/events.rs` | 新增 | Channel 订阅纯逻辑（REQ-31，见 §4.2） |
| `crates/sr-enum/Cargo.toml` | 修改 | 追加 `sr-core`（path）、`wasapi 0.26`、`windows 0.62`（features：Com/Variant/Shell_PropertiesSystem/EnhancedStorage 等，均为规格 §1 锁定栈） |
| `crates/sr-enum/src/windows.rs` | 充实 | `list_windows()`（REQ-1/3，见 §4.3） |
| `crates/sr-enum/src/monitors.rs` | 充实 | `list_monitors()`（REQ-2，见 §4.4） |
| `crates/sr-enum/src/audio_sessions.rs` | 充实 | `list_audio_sessions()`（REQ-4，见 §4.5） |
| `crates/sr-enum/src/identity.rs` | 充实 | `process_identity(pid)` 最小身份底座（PID → 创建时间 UNIX ms + 映像名 + 应用名兜底） |

### 2.2 测试（M1-B 红阶段交付，冻结；§9 两处缺陷修复已由测试作者审查确认并接管）

`crates/sr-core/tests/channel_snapshot_rebind.rs`（L1）、`tests/sr-itests/Cargo.toml`、`tests/sr-itests/src/lib.rs`、`tests/sr-itests/tests/enum_targets.rs`、`tests/sr-itests/tests/enum_audio.rs`、`tests/sr-itests/tests/common/mod.rs`。

### 2.3 未变动

其余骨架占位（sr-capture / sr-audio / sr-pipeline / sr-shot / sr-merge / tools/tone_player 仅含 `pub mod` + TODO）、src-tauri（M1-A 最小壳，本里程碑无 Channel 包装任务）、specs/、ui/、Cargo.toml（workspace 清单）、rust-toolchain.toml、.gitignore、README.md、docs/milestones/m1-bootstrap.md。

gitignoreed 的非契约产物：`target/`、`src-tauri/gen/`、`tools/ffmpeg/`、`tools/spikes/`（历史 spike 与锁定构建，均不属本次范围）。

### 2.4 未预期文件核查

未发现未预期文件。工作树顶层仅含骨架与上述交付物；`git status --porcelain` 全部为预期 untracked 条目（仓库 main 分支尚无任何提交）。

## 3. 复跑证据（本机实际执行，2026-10-16）

| # | 命令 | 结果 |
|---|---|---|
| 1 | `cargo check --workspace` | **exit 0，0 warning**（31.02s；日志 %TEMP%\sr_m1c_check.log） |
| 2 | `cargo test --workspace` | **exit 0**：L1 `channel_snapshot_rebind` → `1 passed; 0 failed`（0.36s）；L2 按门 `enum_targets` → `0 passed; 4 ignored`、`enum_audio` → `0 passed; 1 ignored`；其余 10 库单测/lib/bin/文档测试全部 `ok. 0 passed; 0 failed`——默认套件不受 L2 污染（红报告 §4 门控生效；日志 %TEMP%\sr_m1c_test_ws.log） |
| 3 | `cargo test -p sr-itests --test enum_targets -- --ignored --nocapture` | **exit 0，`4 passed; 0 failed`（1.08s，并行模式）**；复跑第二次 → `4 passed`（0.97s）。4 用例：`it_enum_windows_lists_visible_app_name` / `it_enum_monitors_physical` / `it_enum_refresh_reflects_open_close` / `it_enum_uwp_cloaked_filter`，各用例头部打印 `[env] OS: Microsoft Windows [Version 10.0.26200.9457]`（日志 %TEMP%\sr_m1c_targets_run1.log / run2.log） |
| 4 | `cargo test -p sr-itests --test enum_audio -- --ignored --nocapture` | **exit 0，`1 passed; 0 failed`（1.04s）**：`it_list_active_audio_sources_snapshot`（锁定 ffplay 实音源；日志 %TEMP%\sr_m1c_audio_run1.log） |

运行残留清理：`%TEMP%` 下陈留 3 个测试残留目录（`sr_m1_red_audio_14524` 43.9MB、`sr_m1_red_124200`/`sr_m1_red_82060` 微小 txt 目录）已全部删除（共释放 ~44MB）；运行结束无 ffplay 残留进程。

**L1+L2 全绿结论**：L1 1/1、L2 4+1/4+1，全部可复现（本次会话合计 enum_targets ×2 连绿、enum_audio ×1、workspace ×1、check ×1）。

## 4. 实现对照契约要点（审查结论）

### 4.1 sr-core::error（红报告 §3.4 / 规格 §3.4）

- 11 个规格变体与 E_* 错误码一一对应：TargetLost / LoopbackUnavailable / ConsentDenied / EncoderUnavailable / PipeStall / SidecarExit / DiskFull / SegmentCorrupt / RecoveryFailed / MergeIncompatible / IdentityAmbiguous，`code()` 返回规格错误码字符串；另设 `Internal(String)` 作实现期 OS/环境调用失败的兜底（无规格码，返回 "E_INTERNAL"，模块文档明确其不得作为跨模块契约码外溢）——与红报告 §3.4「实现可自选、后续任务钉死后再收紧」一致，**无过度设计**。
- `Debug + Clone + PartialEq + Eq + Display + std::error::Error`，满足测试 `.expect` 消费（`E: Debug`）。

### 4.2 sr-core::events（REQ-31，红报告 §3.2/§3.3）

- API 形状与契约一致：`ChannelHub<S,A,L>::new/update_state/subscribe/publish_alert/publish_level`；`Subscription { snapshot, alerts, levels }`；`Receiver` 具备 `recv_timeout`/`try_recv`（std mpsc，同步通道，无异步运行时依赖）；`LevelThrottle::new/should_emit` 显式时间参数可注入时钟。
- 订阅即快照：`subscribe()` 在状态锁内 clone 当前 S（测试断言重绑取得 revision=7 ✓）。
- 发送失败容忍（R6：publish 返回 `()` 钉死）：**事务式投递**——存在失效接收端（Receiver drop 置 live=false）时丢弃本条事件并回收失效订阅，发布方永不阻塞不外溢错误；存活订阅端以「订阅即快照」恢复。与测试「旧接收端全灭 → publish 不中断 → 新订阅继续收事件」逐条吻合（旧订阅失效后的在途一条被丢弃属实现选定的语义，测试未约束该条的送达，且重绑端有快照兜底）。
- 分通道与洪泛：alerts 与 levels 完全独立（各自 Vec + mpsc），200 条电平洪泛不触碰告警通道 ✓。
- 电平 ≤10Hz：`LevelThrottle`（首事件放行、间隔不足节流，0/50/100ms 边界确定性可测）+ 每订阅 `LevelGate` trailing 语义——节流区间内仅保留最新值，到期由 per-hub flush 线程（Condvar 等待，非忙轮询）送达并记账 last_emit，**保证任意区间至少送达 1 条**（满足「50 条瞬时 → ~250ms 窗口 ≤10 条且 ≥1」）。实时路径（should_emit 直发）与 trailing 路径共用同一节流记账，节奏一致。
- FFI/并发安全：无 unsafe；锁次序单一（state、subs 两把独立锁无嵌套）；flush 线程经 `wait_timeout` 释放锁等待，publish 侧 `notify_all` 唤醒，无死锁路径。

### 4.3 sr-enum::windows（REQ-1/3，红报告 §3.1/§3.3）

- `WindowEntry` 六字段与契约逐字段一致（hwnd:u64 / pid:u32 / process_created_at:u64 UNIX ms / title / app_name / process_name）；HWND 数值化 `hwnd.0 as usize as u64` 与测试假定一致。
- 可见性 `IsWindowVisible` + cloaked 过滤 `DwmGetWindowAttribute(DWMWA_CLOAKED)≠0` 剔除；DWM 查询失败（窗口恰在销毁）不误杀——与测试地面真值语义一致（REQ-1 cloaked 三段式 + 系统 cloaked 泄露扫描均绿）。
- 身份：`GetWindowThreadProcessId` 取属主 PID（非启动器）；`identity::process_identity` 解析创建时间（GetProcessTimes → FILETIME→UNIX ms）与映像名（QueryFullProcessImageNameW，PROCESS_QUERY_LIMITED_INFORMATION）；身份不可得 → 不列出（REQ-14 不静默编造）。
- UWP 身份解析（M1 最小）：打包应用读窗口 `System.AppUserModel.ID`（SHGetPropertyStoreForWindow + IPropertyStore，仅接受 VT_LPWSTR，读后 PropVariantClear）；未打包应用回退映像名去扩展名。满足「app_name 非空 + UWP 不归属 ApplicationFrameHost」的 M1 可测部分；REQ-14 完整有界规则（后代树/有界提升/宿主禁入）按模块文档留 M4+。
- 刷新（REQ-3）：每次调用 = 一次 EnumWindows 全量重枚举 + HWND 去重（保序）；单次毫秒级，满足 ≤100 窗口 ≤2s 预算（实测 1.08s 覆盖 4 用例全程）。
- FFI 卫生：enum 回调 `extern "system"` 无 panic 路径（Option/Result 驱动）；LPARAM 裸指针指向同步枚举全程存活的局部 Vec（标准 EnumWindows 惯用法，与测试助手同模式）。

### 4.4 sr-enum::monitors（REQ-2）

- `MonitorEntry` 六字段一致；`EnumDisplayMonitors` 收集 + 枚举序赋稠密 index 0..n-1（RouteConfig::Monitor(idx) 引用基础，测试假定成立）。
- 物理分辨率/位置取 `EnumDisplaySettingsW(ENUM_CURRENT_SETTINGS)` 的 DEVMODE dmPosition/dmPels（DPI 无关契约）；查询失败兜底 MONITORINFO rcMonitor（可能 DPI 虚拟化，仅防御性降级——测试真值同为 DEVMODE，若兜底被触发且不一致会如实失败，不构成契约违反）。
- 主显示器：`GetMonitorInfoW` 的 `MONITORINFOF_PRIMARY`，恰一台 ✓。

### 4.5 sr-enum::audio_sessions（REQ-4）

- `AudioSessionEntry` 四字段一致；wasapi 0.26 经默认渲染端点枚举会话，仅 `SessionState::Active`；会话竞态（get_session/get_state/get_process_id 失败）跳过不失败。
- 同 PID 归一为恰一条（同根去重弱形式 ✓）；返回 `Vec` 值快照——值语义天然满足「新增音源不自动并入先前快照」。
- COM 生命周期：叶子函数内 `initialize_mta()` 成功（含 S_FALSE 已初始化需配平的情形）→ `ComGuard` Drop 时 `deinitialize()` 配平；`RPC_E_CHANGED_MODE`（他处已初始化）则不配平直接复用——符合 COM 引用计数语义，无泄漏。
- 身份与 REQ-1 同源（identity.rs），退出会话随进程消失（回收延迟由测试轮询容忍）。

### 4.6 资源释放与错误处理总评

- 句柄：OpenProcess 句柄在 `process_identity` 内查询后无条件 CloseHandle（含查询失败路径）✓；PropVariantClear 配平 ✓；COM 初始化配平 ✓；测试侧 ffplay 由 KillOnDrop 收敛。
- 错误：枚举 API 全部返回 `Result<_, sr_core::Error>`，OS 调用失败经 `Error::Internal` 带上下文（中文前缀）不 panic；仅身份不可得按规格静默剔除该条目（REQ-14）。
- `cargo check --workspace` 0 warning，无未用代码告警；无死代码（identity 为三枚举共用底座）。

## 5. 偏差与未决（返回 orchestrator）

1. **测试运行残留（沿袭红报告 §9.4 #7，已知未决）**：`it_enum_windows_lists_visible_app_name` 每次运行遗留 `%TEMP%\sr_m1_red_<pid>`（微小 txt）；`it_list_active_audio_sources_snapshot` 的 `remove_dir_all` 先于 `player_a` 的 KillOnDrop 执行 → 每次运行最多残留 ~46MB（本次实测 43.9MB）WAV。测试文件冻结，本报告仅清理积压（已清 3 目录 ~44MB）；修复需测试侧将 player drop 前移（零断言语义变化的机制微调），待授权。
2. **ChannelHub flusher 线程生命周期（质量备注，非缺陷）**：`ChannelHub::new` 每 hub 启动 1 条 flush 线程，hub drop 后线程不退出（Arc 保持 inner 存活，60s 兜底轮询周期）。M1 桌面应用单 hub、随进程存活，可接受；若后续里程碑需要多 hub 或显式关停，需补 shutdown 通道。
3. **app_name 解析链冗余分支（质量备注，无行为影响）**：`windows.rs` 中 `window_aumid(hwnd).or(Some(identity.app_name))` 之后的「窗口标题 → "Unknown"」兜底分支逻辑上不可达（前两级恒有 Some）。属防御性写法；后续任务收紧契约时可简化。
4. **audio_sessions 的 STA 复用路径**：调用线程已按 STA 初始化 COM 时（RPC_E_CHANGED_MODE）按「他处已初始化」复用当前模式继续枚举，不配平。符合 COM 语义；Tauri 主线程若为 STA 枚举仍可工作，M3+ 接入 Tauri command 时留意调用线程模式。
5. **红报告既有未决项无新分歧**：#1 `process_created_at`=UNIX ms、#2 `MonitorEntry.index` 稠密 0..n-1、#3 `ChannelHub` 归属 sr_core::events 及命名——实现与测试假定完全一致，维持现状待规格方后续钉死；#4（REQ-4 路由级快照）与 #5（错误枚举收紧断言）留后续里程碑，实现侧已就绪（error.rs code() 锚点已备）。
6. **本报告性质**：绿实现非本会话编写（原执行者会话取消未留终报），本会话以「全量逐文件审查 + 本机复跑留证」对其验收；审查未发现需要返工的实现缺陷，未做任何代码修改。

## 6. 遗留文件清单

- 工作树（全部 untracked，未 git 提交）：§2.1/§2.2 所列生产实现与测试文件 + §2.3 骨架占位 + 本报告（`docs/milestones/m1-green-report.md`，本次唯一新增改动）。
- 本次会话临时证据日志（%TEMP%，gitignore 规则 `*.log` 覆盖不到仓库外路径，仅本机留存）：`sr_m1c_check.log`、`sr_m1c_test_ws.log`、`sr_m1c_targets_run1.log`、`sr_m1c_targets_run2.log`、`sr_m1c_audio_run1.log`。
- `%TEMP%\sr_m1_red_*` 测试残留：陈留已清理（见 §3）；后续运行仍会产生（未决 #1）。
