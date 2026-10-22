# M19a 隐私脱敏最小切片（文件名 / manifest / 设置）

- 日期：2026-10-23
- 基线：`027b73e`（合规文档提交后）
- 范围：只做**默认文件名/manifest 脱敏 + Settings 开关**；托盘状态、开始前摘要、日志脱敏、About/许可证、CSP、浮窗、退出确认均未进入本切片。
- 状态：**源码实现与 L1/UI/恢复 L2 回归通过；未重建安装包；未提交验收结论。**

## 1. 行为

- `Settings.include_window_title_in_filename` 默认 `false`；坏类型只降级本字段，不影响其他设置。
- 窗口录制默认目标名 `Window {seq}`，不含窗口标题；成品文件名与 manifest `identity.name` 使用同一通用名。
- 设置 `true` 时回退旧行为：窗口标题经 `sanitize_target_name` 进入成品名与 manifest。
- 崩溃恢复默认成品名 `{timestamp}_recovered-{route_id}`，不把旧 manifest 的窗口标题带入新成品名；`true` 时保留旧行为。
- 旧 v1 manifest 解析字段未改，含标题的旧会话仍可扫描/收尾/幂等复用；旧成品文件不改名。
- 设置页新增「成品文件名包含窗口标题」复选框，默认关闭；mock 设置默认 false 且保存合并保留其他字段。

## 2. 关键文件

| 文件 | 变更 |
|---|---|
| `crates/sr-pipeline/src/naming.rs` | `output_target_name` |
| `crates/sr-pipeline/src/route.rs` | 隐私参数贯通；旧 API 默认 true |
| `crates/sr-pipeline/src/recovery.rs` | `recovery_output_stem`；新收尾入口；旧入口兼容 |
| `src-tauri/src/settings.rs` | 新字段 + 容错布尔 + 测试 |
| `src-tauri/src/commands.rs` | 启动/恢复快照传递隐私字段 |
| `ui/app.js` / `ui/api.js` | 设置开关 / mock 默认与合并 |
| `crates/sr-pipeline/tests/privacy_naming.rs` | 纯函数红→绿测试 |
| `tools/ui/settings-sample-rate.mjs` | 设置开关往返与兼容断言 |

## 3. 验证

- `cargo fmt --all -- --check`：通过。
- `cargo test --workspace`：通过（全工作区测试；含新增命名/设置/恢复测试）。
- `node tools/ui/settings-sample-rate.mjs`：50 项检查全部 PASS（含 M19a 9 项）。
- `cargo test -p sr-itests --test recovery -- --ignored --nocapture --test-threads=1`：5/5 PASS（旧 API true 兼容、崩溃恢复、磁盘满、损坏成品、44.1k/96k）。
- 未跑长同步/完整 GUI；未重建安装包。

## 4. 仍待后续

- 旧会话恢复列表仍可能显示旧 manifest 的窗口标题（本切片只保证新成品名/新 manifest 与恢复成品名）。
- 托盘持续指示、开始前摘要/告知、日志脱敏、About/许可证入口、CSP/IPC、浮窗、退出确认。
- 安装包重建后需验证新设置字段真实 Tauri 往返；本次只有 mock runner 与 Rust 单测。
- 本报告是工程证据，不是法律意见。
