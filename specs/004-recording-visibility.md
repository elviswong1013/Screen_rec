# 004 - Screen_rec 录制状态可查询与开始前告知（SDD / M19b）

Status: Approved
Created: 2026-10-23
Updated: 2026-10-23
Approved: 2026-10-23（plan review APPROVED + 用户批准 AC-1..4 / REQ-1..5）
PRD: `docs/prd/004-recording-visibility.md`（Approved 2026-10-23）
实现基线：待实现（M19b1 先行；M19b2 随后）

## 1. 范围

M19b 只做：托盘/主窗状态查询、录制中关闭主窗一次提示、主窗开始栏摘要与首次告知、`recording_notice_ack` 设置。置顶浮窗、红点图标、托盘开始确认、退出确认、旧会话 UI 标题、日志脱敏、About/许可证、CSP/IPC、安装包/签名均不在本规格。

本规格分两批验收：M19b1（REQ-1/2）、M19b2（REQ-3/4）；两批完成前不标 Implemented。

## 2. 需求（REQ）

### REQ-1（AC-1）托盘/主窗状态

- 后端固定 1s 轮询（初始化立即刷新；变化才应用），快照按 route id 统计：
  - `recording`：state=recording 的活动路由数；
  - `stopping`：已请求停止/收尾中的路由数（state=stopping 或等价收尾阶段）；
  - 终态 finalized/failed 不计入；纯录音多个来源共享 handle，但按 route id 计数。
- 纯函数 `tray_status(recording, stopping) -> TrayStatus`：
  - 0/0：tooltip `Screen_rec`；start enabled；stop disabled；title `Screen_rec`；
  - R>0, S=0：`Screen_rec — 录制中 (R 路)`；start disabled；stop enabled；title `● 录制中 (R) — Screen_rec`；
  - R=0, S>0：`Screen_rec — 正在收尾 (S 路)`；start disabled；stop disabled；title `收尾中 (S) — Screen_rec`；
  - R>0, S>0：`Screen_rec — 录制中 (R 路) · 收尾中 (S 路)`；start disabled；stop enabled；title `● 录制中 (R) · 收尾中 (S) — Screen_rec`。
- 更新 tooltip/menu/window title 前释放状态锁；不持锁跨 Tauri UI 调用；不新增图标资源；不得使用 Windows 不支持的 `TrayIcon::set_title`。

### REQ-2（AC-2）录制中关闭主窗提示

- 仅 `main` 窗口 `CloseRequested`：
  - idle → 现有静默 `prevent_close + hide`；
  - active 且本进程未提示过且无提示进行中 → 同步 `prevent_close`，原子认领首次提示，后台显示原生 MessageBox（正文「录制仍在继续；可从托盘停止或重新打开窗口」）；确认后通过 Tauri 窗口接口 hide；
  - 提示进行中再次关闭 → 只 `prevent_close`，不重复弹框；
  - 已提示过 → 静默 hide。
- 不得在窗口事件回调内同步弹框/等待线程/持状态锁；弹框失败记录并保留窗口，允许下次重试；成功后置 shown=true。
- 纯函数 `close_main_action(active, shown, pending) -> HideSilent | PromptOnce | Ignore` 覆盖上述分支；进程级原子标志。

### REQ-3（AC-3）主窗开始栏摘要/告知

- 单一 `buildStartPlan()` 生成启动 DTO、摘要和 `needs_notice`：
  - 覆盖窗口、逐屏、区域、纯录音；摘要目标/音源/输出目录/格式；
  - 音源包含 mic/system/render_endpoint → needs_notice=true；仅 app 音源 → false；
  - 零音源仍走既有拒绝，不新增纯画面路径；
  - 纯录音显示 WAV，不显示画布/帧率；区域显示最终选区尺寸。
- 确认状态机（会话内存）：`Idle -> Expanded(plan) -> Starting`；仅当 needs_notice && !recording_notice_ack：
  - 第一次点“开始”只展开并显示告知，不调用启动 API；
  - 第二次点“确认并开始”才启动；
  - 目标/音源/输出/格式变化、取消、启动失败/完成均复位到 Idle；每次新启动尝试都需重新确认。
- 仅 app 音源或 ack=true：一次点击启动；摘要仍可展开。
- Ctrl+Enter 走同一门禁；按键防重复；区域 overlay 的确认只交付选区，不视为开始确认。
- 告知条 ≤2 行、事实陈述；提供“不再显示此提示”复选框；勾选本身不启动。

### REQ-4（AC-2/AC-4）Settings 与 ack 持久化

- `Settings.recording_notice_ack: bool` 默认 false；缺字段 false；坏类型只降级本字段；磁盘加载/`set_settings` 同口径。
- 开始栏保存 ack 时先读当前 Settings，只改该字段后整体保存，保留其他字段；设置页保存其他字段时必须保留当前 ack。
- 保存失败不得显示成功；ack=true 后仍可从展开摘要取消勾选；未勾选时每次新启动尝试都需确认。
- 不新增依赖；mock 默认 false，mock 合并仅前端测试语义，不代表后端未知字段保留契约。

### REQ-5（AC-4）验证与追溯

- 具名纯函数测试：`tray_status_idle`、`tray_status_recording`、`tray_status_stopping_only`、`tray_status_mixed`、`close_main_action_idle/active_once/pending/after_prompt`。
- Settings 测试：默认/缺字段/坏类型/磁盘加载。
- UI runner（新增 `tools/ui/recording-visibility.mjs` 或扩展既有 runner）：
  - 摘要内容；仅 app 一次启动；mic/system/render_endpoint 两击；取消不启动；配置变化复位；Ctrl+Enter；零音源仍拒绝；ack 勾选/取消/重载；设置页保存保留 ack。
  - 更新 `audio-sources-builder.mjs`、`region-record.mjs` 中直接启动 helper/用例，但保留零音源/终态竞态等原断言，不能统一改成“多点一次”。
- 命令：`cargo fmt --all -- --check`、`cargo test --workspace`、设置 runner、告知 runner、`audio-sources-builder.mjs`、`region-record.mjs`。
- L3 人工必须签核：真实 tooltip/menu/title、录制中关闭后继续录制、再次关闭不重复提示、空闲关闭、托盘停止；未执行不得算通过。
- 不要求完整 GUI/长媒体/安装包；报告中逐项映射 REQ-1..5，未跑项明确标注。

## 3. 接口/数据

- 托盘：轮询线程/定时器读取状态快照；保存 tray/menu 句柄或按 id 获取；纯函数独立测试。
- 关闭：`CloseRequested` 分支同步 prevent_close + 原子认领；后台 MessageBox/hide；仅 main。
- Settings：新增 bool 字段；开始栏/设置页双向保留。
- 前端：`buildStartPlan` 单一来源；确认状态机内存态；ack 落盘。

## 4. Out of scope

- 置顶悬浮窗、红点/任务栏图标、托盘开始确认、退出确认。
- 旧会话 UI 标题、日志/错误串脱敏、About/许可证、CSP/IPC。
- 安装包重建、签名、法律/专利/第三方通知清权。
- 零音源纯画面录制。

## 5. 测试计划映射

| REQ | 测试/证据 |
|---|---|
| REQ-1 | tray_status 四态纯函数；主窗标题映射；1s 轮询源码追溯；L3 真实托盘 |
| REQ-2 | close_main_action 四态纯函数；L3 录制中关闭/重复关闭/空闲关闭 |
| REQ-3 | recording-visibility runner：摘要/两击/取消/配置变化/Ctrl+Enter/零音源拒绝/区域/纯录音 |
| REQ-4 | settings 单测；runner ack 勾选/取消/重载；设置页保留 ack |
| REQ-5 | workspace 测试、格式检查、四个 UI runner、L3 签核记录 |

## 6. 批准与状态

- plan review 于 2026-10-23 APPROVED；用户同日批准 AC-1..AC-4 / REQ-1..5；本规格状态 `Approved`。
- 实现按 TDD 红→绿，M19b1（REQ-1/2）与 M19b2（REQ-3/4）分批验收；两批均完成并经 acceptor ACCEPTED 后才可 `Implemented`。
- 进度（2026-10-23）：M19b1 REQ-1/2 与 M19b2 REQ-3/4 源码/纯函数/Rust/四 UI runner 完成；真实 Tauri、托盘/关闭/告知 L3 待签；两批均未标 Implemented。
