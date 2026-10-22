# M19b1 录制状态可查询 + 录制中关闭主窗一次提示（REQ-1/2）

- 日期：2026-10-23
- 基线：`748b5ca`（PRD/SDD 004 批准后）
- 规格：`docs/prd/004-recording-visibility.md`（Approved）、`specs/004-recording-visibility.md`（Approved）；本切片只实现 **REQ-1/2**，REQ-3/4（开始栏摘要/告知/ack）未进入本批。
- 状态：**源码与自动化验证完成，待编排者/acceptor；L3 真实托盘/关闭交互未执行。** SDD 004 仍为 `Approved`（两批完成并经 acceptor ACCEPTED 后才可标 `Implemented`）。
- 未提交、未打包；未改 UI、安装包、录制/编码/同步/恢复语义。

## 1. 实现行为

### REQ-1（AC-1）托盘/主窗状态

- `CommandsState::route_counts() -> RouteCounts { recording, stopping }`：
  - 按 route id 统计（纯录音多来源共享 handle 时仍按 id 各计 1）；
  - 只计 `recording` / `stopping`；`finalized` / `failed` 终态不计；
  - **未知非终态**（例如未来新增的 `finalizing`）保守归入 `recording`，保证活动会话不漏计；
  - 锁内只拷贝计数，返回前释放路由锁（测试用 `try_lock` 断言）。
- 纯函数 `tray_status(recording, stopping) -> TrayStatus`：
  - `0/0`：tooltip `Screen_rec`；start enabled；stop disabled；title `Screen_rec`；
  - `R>0,S=0`：tooltip `Screen_rec — 录制中 (R 路)`；start disabled；stop enabled；title `● 录制中 (R) — Screen_rec`；
  - `R=0,S>0`：tooltip `Screen_rec — 正在收尾 (S 路)`；start disabled；stop disabled；title `收尾中 (S) — Screen_rec`；
  - `R>0,S>0`：tooltip `Screen_rec — 录制中 (R 路) · 收尾中 (S 路)`；start disabled；stop enabled；title `● 录制中 (R) · 收尾中 (S) — Screen_rec`。
- setup 保存 tray/start/stop 句柄（`TrayIcon` 由轮询线程持有，避免 setup 返回后最后一个实例被丢弃），固定 **1s** 后台轮询；初始化立即读取一次，之后每 1s；仅在 `TrayStatus` 变化时更新 tooltip、start/stop enabled、主窗 title。
- 读取计数后释放状态锁，再调用 Tauri UI；不持路由锁等待主线程。未使用 `TrayIcon::set_title`（主窗标题走 `Window::set_title`），未新增图标资源。
- 托盘「开始/停止/退出」处理逻辑与 quit 语义未改（仅空闲时「停止」初始为 disabled，轮询立即按真实状态刷新）。

### REQ-2（AC-2）录制中关闭主窗一次提示

- 仅 `main` 窗口 `CloseRequested`：
  - idle（recording+stopping=0）→ `prevent_close` + 静默 hide；
  - active 且未提示、无 pending → 同步 `prevent_close`，`AtomicBool` CAS 原子认领，后台线程显示原生 MessageBox「录制仍在继续；可从托盘停止或重新打开窗口」；
  - pending 中再次关闭 → 只 `prevent_close`，不重复弹框；
  - 已 shown → `prevent_close` + 静默 hide。
- MessageBox 失败、提示线程启动失败或隐藏失败 → 重置 pending（隐藏失败同时 shown=false）、保留窗口，下次关闭可重试；仅 MessageBox 确认且主窗隐藏成功后置 shown=true。
- 进程级 `AtomicBool`：`CLOSE_PROMPT_PENDING` / `CLOSE_PROMPT_SHOWN`；不在窗口事件回调内同步弹框/等待线程/持状态锁。
- `tray_notice` 改为返回 `bool`（MessageBoxW 成功/确认），既有托盘提示调用忽略返回值。

## 2. 关键文件

| 文件 | 变更 |
|---|---|
| `src-tauri/src/commands.rs` | `RouteCounts` + `CommandsState::route_counts`；4 个计数测试 |
| `src-tauri/src/lib.rs` | `TrayStatus`/`tray_status`、`CloseMainAction`/`close_main_action`、1s 轮询、关闭提示原子/后台流程；8 个纯函数测试 |
| `docs/milestones/m19b1-report.md` | 本报告（实现/测试/未验证/L3 步骤） |

## 3. 红 → 绿证据

红（实现前，`cargo test -p screen-rec --lib tray_status`）：编译失败 24 项，含
- `error[E0425]: cannot find function close_main_action`
- `error[E0433]: cannot find type CloseMainAction`
- 缺失 `tray_status` / `TrayStatus`
- `route_counts` / `RouteCounts` 未定义（`E0599` 等）

绿（实现后）：
- `cargo test -p screen-rec --lib tray_status` → 4 passed。
- `cargo test -p screen-rec --lib close_main_action` → 4 passed。
- `cargo test -p screen-rec --lib unit_route_counts` → 4 passed。
- `cargo test --workspace` → 186 passed / 0 failed / 91 ignored（既有 L2/L3 ignored 不变）。
- `cargo fmt --all -- --check` → 通过。

## 4. 测试清单

| 测试 | 覆盖 |
|---|---|
| `tray_status_idle` | 0/0 tooltip/start/stop/title |
| `tray_status_recording` | R>0,S=0 |
| `tray_status_stopping_only` | R=0,S>0 |
| `tray_status_mixed` | R>0,S>0 |
| `close_main_action_idle` | idle → HideSilent |
| `close_main_action_active_once` | 首次 active → PromptOnce |
| `close_main_action_pending` | pending → Ignore |
| `close_main_action_after_prompt` | shown → HideSilent |
| `unit_route_counts_empty` | 空表 0/0 |
| `unit_route_counts_recording_and_stopping_by_route_id` | 按 id 分别计数 + 锁已释放 |
| `unit_route_counts_excludes_terminal_states` | finalized/failed 排除 |
| `unit_route_counts_unknown_non_terminal_is_conservatively_recording` | 未知非终态归 recording |

## 5. 未验证 / L3 人工步骤

未执行（不跑长媒体/GUI；本切片无 UI 变更）：

1. 启动应用，确认空闲时托盘 tooltip 为 `Screen_rec`、菜单「开始」可用/「停止」禁用、主窗标题 `Screen_rec`。
2. 开始一路/多路录制，悬停托盘确认 tooltip 路数与 `● 录制中` 标题；确认「开始」禁用、「停止」可用。
3. 停止录制，观察收尾阶段 tooltip `正在收尾 (S 路)`、停止禁用；收尾完成后恢复空闲文案/菜单。
4. 关闭主窗两次观察：首次录制中 → 弹出一次原生提示，确认后窗口隐藏但录制继续；再次关闭 → 不再弹框；从托盘「显示」可重新打开；托盘「停止」可正常停止。
5. 空闲关闭 → 静默隐藏；托盘「开始/停止/退出」既有语义不变。
6. 可选：混合状态（一路录制 + 一路收尾）同时显示两种计数。

## 6. 范围与后续

- 本批不含 REQ-3/4：开始栏摘要、首次告知、`recording_notice_ack`、UI runner；SDD 004 不标 Implemented。
- 未改 `ui/`、`tools/`、`specs/`、录制管线；未重建安装包/签名。
- 本报告为工程证据，不代表 L3 交互已签核。
