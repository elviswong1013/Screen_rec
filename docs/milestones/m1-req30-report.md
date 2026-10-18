# M1-D 报告：REQ-30 外壳功能（设置持久化 / 单实例 / 托盘 / 关闭语义）

- 日期: 2026-10-16
- 规格: specs/001-screen-recorder.md §2.8（REQ-30）、§3.3（`get_settings`/`set_settings`）、§3.4（Settings 数据模型）、§3.1（进程模型）、§4（`manual_ux_tray_settings_single_instance`，L3 人工）
- 环境: Windows 11 25H2 build 10.0.26200.9457，SESSIONNAME=Console；cargo/rustc 1.99.0 msvc；tauri 2.12.4 + tauri-plugin-single-instance 2.5.4（官方插件，已批准栈内）
- 范围: 仅 `src-tauri/` 与 `ui/` 最小接线；未改 crates/*、tests/*、specs/、docs/ 既有内容、tools/；未 git 提交

## 1. 变更文件

| 文件 | 状态 | 内容 |
|---|---|---|
| `src-tauri/Cargo.toml` | 修改 | tauri 增 feature `tray-icon`+`image-ico`；新增 `tauri-plugin-single-instance = "2"`、`serde`(derive)、`serde_json` |
| `src-tauri/src/settings.rs` | 新增 | Settings 结构（§3.4 形状）+ 自管 JSON 原子读写 + 校验 + 7 个 L1 单测 |
| `src-tauri/src/lib.rs` | 重写 | REQ-30 接线：两命令、单实例插件、托盘（图标/菜单/事件）、关闭窗口隐藏、设置状态管理 |
| `src-tauri/tauri.conf.json` | 修改 | `"withGlobalTauri": true`（前端 `window.__TAURI__` 探针需要） |
| `ui/index.html` | 重写 | 最小接线探针页：载入即执行一次 `get_settings → 切 fps 30↔60 → set_settings → 再读` 回路 + 两个手动按钮（无样式美化，正式 UI 后续里程碑替换） |
| `Cargo.lock` | 随依赖更新 | single-instance/serde/serde_json 直依赖锁定的常规变更 |
| `docs/milestones/m1-req30-report.md` | 新增 | 本报告 |

无其它改动。crates/*、tests/* 冻结未动（`cargo test --workspace` 全绿不变，见 §3）。

## 2. 设计决策（记录在案）

| # | 决策 | 依据 |
|---|---|---|
| D1 | **存储选型：自管 JSON + 原子写**（写 `settings.json.tmp` → rename 覆盖），不用 tauri-plugin-store | 依赖面最小；原子写可控（与规格对 Manifest「临时文件+rename」同风格）；store 插件对单键 JSON 无增益 |
| D2 | 文件位置：`app_local_data_dir()` = `%LOCALAPPDATA%\com.screenrec.desktop\settings.json` | 设置含本机路径（output_dir），不宜随漫游（AppData\Roaming）同步。注：计划用的 `app_local_config_dir` 在 tauri 2.12.4 不存在（该 API 为更新版本引入），实测后改用语义等同的 local data 目录 |
| D3 | **Settings 结构按 §3.4 落在 src-tauri**（`settings::Settings`），JSON 字段名 = 规格字段名 | 本轮任务约束「不得改 crates/*」，而 §3.2 将 Settings 规划于 sr-core::types。属**归属偏差**：结构形状/序列化已定，后续里程碑迁移到 sr-core 时保持 JSON 兼容，src-tauri 命令层不动 |
| D4 | 托盘菜单本轮只做「显示/退出」；REQ-30 的「开始/停止」项留待录制管线（M3+）接入时追加 | 任务范围明确（无录制任务可开始/停止）；菜单事件处理器已按可扩展结构编写 |
| D5 | JSON 枚举形状：`default_audio_mode: "isolated"|"all_tracks"`；`encoder: "auto"` 或 `{"named":"h264_mf"}`（serde 外部标签枚举默认序列化） | §3.4 `Auto|String` 与 REQ-17 两模式的可持久化表达；形状在 settings.rs 模块文档中写明 |
| D6 | `set_settings` 轻量校验：fps/segment_secs ≠ 0、output_dir 非空；先落盘成功再更新内存 | 规格未定义取值域，不发明过严约束；失败时内存保持不变 |
| D7 | 关闭窗口=始终隐藏（骨架行为）；「录制中关闭不中断」分级逻辑留 M3+ | 当前无录制会话；CloseRequested 钩子处注释标明扩展点 |

## 3. 命令证据（本机实际执行，2026-10-16）

| 命令 | 结果 |
|---|---|
| `cargo check --workspace` | **exit 0，0 warning**（日志 %TEMP%\sr_m1d_check2.log） |
| `cargo test --workspace` | **exit 0**：新增 `screen-rec` 单测 **7 passed**（settings 默认值/序列化回路/部分文件补默认/校验拒绝/原子写回路/缺失文件写默认/损坏文件回退）；既有 L1 `channel_snapshot_rebind` `1 passed`；L2 按门 `4+1 ignored`；其余全部 `ok. 0 passed; 0 failed`——现有测试全绿未破坏（日志 %TEMP%\sr_m1d_test_ws.log） |
| `cargo build -p screen-rec` | exit 0（30.74s；single-instance 2.5.4 一并编译） |

## 4. 手动/自动化验证记录（L3 `manual_ux_tray_settings_single_instance` 的执行证据）

启动方式：直接运行 `target\debug\screen-rec.exe`（frontendDist 静态页内嵌，无 devUrl）。

### 4.1 首次启动 + 设置持久化 ✓

- 进程存活，主窗口 `Screen_rec` 可见（hwnd=20457442）。
- `%LOCALAPPDATA%\com.screenrec.desktop\settings.json` **自动创建**，内容为探针回路执行后的状态：

```json
{
  "output_dir": "C:\\Users\\elvis\\Videos\\Screen_rec",
  "fps": 60,
  "cursor": true,
  "default_audio_mode": "isolated",
  "mic_device": null,
  "encoder": "auto",
  "segment_secs": 5
}
```

- **fps=60 证明命令回路**：探针页载入即 `get_settings`（默认 30）→ 改 60 → `set_settings` → 落盘。即前端 invoke → 命令 → 校验 → 原子写全链路生效。
- 默认值逐项符合 §3.4：fps 30、cursor true、isolated、mic null、encoder auto、segment 5；output_dir 符合 REQ-24（`%USERPROFILE%\Videos\Screen_rec`）。
- **跨启动持久化**：第二次运行读到 60 → 探针切回 30 → 文件变为 fps=30（读回+写路径均跨进程验证）。

### 4.2 单实例 ✓

- 第二次启动 `screen-rec.exe`：**第二进程以 exit code 0 自行退出**（插件转发参数后退出），进程计数 2→1，首实例不受影响。
- **聚焦生效**：二次启动后 `GetForegroundWindow()` 返回的窗口标题 = `Screen_rec`，且窗口非最小化（回调内 unminimize+show+set_focus 生效）。

### 4.3 关闭窗口语义 ✓

- 向主窗口发送 `WM_CLOSE`：窗口隐藏（EnumWindows 实测 `Tauri Window` visible=**False**），**进程继续存活**（prevent_close 生效，未退出）。
- 备注：`Process.MainWindowHandle` 此时可能误报单实例插件助手窗口（`com.screenrec.desktop-siw`）为句柄，验证时以按窗口类名枚举（`Tauri Window`）为准。
- 偏差记录（§5 #2）：「录制中关闭不中断录制」需 M3+ 会话管理接入后在同一钩子分级；当前为始终隐藏的骨架行为。

### 4.4 托盘 ✓（含一项自动化限制说明）

- 图标注册：托盘助手窗口存在（`tray_icon_app` 类），UIA 溢出区中可见名为 `Screen_rec` 的图标按钮（tooltip 生效；图标为现有占位 icons/icon.ico）。
- 菜单结构：两项（显示/退出），弹出正常（实测菜单窗口 #32768，170×76px@物理分辨率）。
- **「显示」项 ✓**：窗口隐藏后激活菜单第一项 → 主窗口重新可见（visible=True）。
- **「退出」项 ✓**：激活菜单第二项 → **进程退出**（无活动任务时退出=正常退出；应用消失，`app.exit(0)` 路径生效）。重复两轮验证（含最后的收尾退出）均通过。
- **自动化限制（如实记录）**：在 Win11 溢出弹窗（XAML）上用合成鼠标输入点击图标，外壳未向应用转发该点击（合成输入下的宿主行为，非应用缺陷），故自动化验证通过**直接投递 Shell 回调消息**（`WM_USER_TRAYICON`=6002, lparam=`WM_RBUTTONUP`，与 explorer 转发真实右键后进入应用的消息完全相同）触发菜单——「菜单弹出 → 菜单事件 → 处理器 → 退出/显示」这一应用内完整路径已被端到端验证；「真实用户点击任务栏图标」一跳留 L3 人工签核（预计人工点击即可通过，因真实输入由 shell 正常转发）。

### 4.5 环境清理

- 验证结束无 `screen-rec` 残留进程；测试期间创建的设置文件保留（即产品数据，非垃圾）。

## 5. 偏差与未决（返回 orchestrator）

1. **Settings 归属偏差（D3）**：按 §3.2 应居 sr-core::types，本轮因「不得改 crates/*」约束落 src-tauri。迁移时 JSON 字段名与形状保持不变（D5 已记录），命令层无需变动。建议 sr-core types 任务（M3+）把 Settings 移入 sr-core 并让 src-tauri 引用。
2. **关闭语义骨架（D7）**：REQ-30 完整语义「关闭主窗口不退出**录制**」在 M3+ 录制接入后在 `on_window_event` 的 CloseRequested 钩子按会话状态分级（代码注释已标扩展点）；当前验证的是「隐藏+进程存活」骨架行为。
3. **托盘「开始/停止」菜单项未做（D4）**：无录制任务可开始/停止；随 M3+ 录制接入追加，菜单事件处理器结构已可扩展。
4. **托盘真实点击一跳待人工签核（见 4.4 限制说明）**：合成输入不被 Win11 溢出弹窗转发是自动化环境限制；应用内路径已完整验证。如需完全自动化的托盘 E2E，可考虑后续在 L3 harness 里用真实输入注入方案，或不作自动化目标。
5. **`app_local_config_dir` 不存在于 tauri 2.12.4（D2）**：使用 `app_local_data_dir`（同为 LocalAppData）；若未来升级 tauri 且需要 Roaming 语义再议。
6. **校验域从简（D6）**：fps/segment_secs 的合法取值域（如 fps 上限、编码器名合法性对照 S2 矩阵）待后续里程碑钉死后收紧。
7. 探针页（ui/index.html）的载入自动读写回路是 M1-D 验证手段，正式 UI 轮（designer/后续里程碑）应整体替换该页。

## 6. 验收对照

| 项 | 状态 |
|---|---|
| `get_settings`/`set_settings` 按 §3.3 契约可用（前端 invoke 实测） | ✅ |
| Settings 按 §3.4 形状与默认值（单测 + 实测文件逐字段核对） | ✅ |
| 持久化文件位置确定且原子写（单测 + 实测无 .tmp 残留） | ✅ |
| 单实例：二次启动聚焦已有窗口（exit 0 + 前台实测） | ✅ |
| 托盘：图标+菜单（显示/退出）可用（菜单弹出/两项行为实测） | ✅（真实任务栏点击一跳留 L3 签核） |
| 关闭窗口不退出进程（隐藏到托盘） | ✅（骨架行为，录制分级留 M3+） |
| `cargo check --workspace` 0 warning | ✅ |
| `cargo test --workspace` 全绿（含新增 7 单测，L1 1 passed，L2 门控不变） | ✅ |
| 未改 crates/*、tests/*、specs/、docs/spikes/、既有 harness；未 git 提交 | ✅ |
| 证据落盘（本报告） | ✅ |
