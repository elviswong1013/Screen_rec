# M19a 隐私脱敏最小切片（文件名 / manifest / 设置）

- 日期：2026-10-23
- 基线：`027b73e`（合规文档提交后）
- 范围：只做**默认文件名/manifest 脱敏 + Settings 开关**；托盘状态、开始前摘要、日志脱敏、About/许可证、CSP、浮窗、退出确认均未进入本切片。
- 状态：**源码/UI 验收 ACCEPTED（2026-10-23）；PRD 003 Approved、SDD 003 Implemented；未重建安装包。**
- 规格：`docs/prd/003-privacy-naming-redaction.md`（Approved 2026-10-23）、`specs/003-privacy-naming-redaction.md`（Implemented 2026-10-23）。实现提交 `d42f9c5` 先于批准记录；未回溯批准历史；acceptor 证据含 `16389e5`。

## 1. 行为

- `Settings.include_window_title_in_filename` 默认 `false`；坏类型只降级本字段，不影响其他设置。
- 窗口录制默认目标名 `Window {seq}`，不含窗口标题；成品文件名使用 `sanitize(target_name)`，manifest `identity.name` 保存**未 sanitize 的 `target_name`**（false 为通用名）。
- 设置 `true` 时回退旧行为：成品文件名使用 sanitize 后的窗口标题；manifest 保存原样标题（不做 sanitize）。
- 崩溃恢复默认成品名 `{timestamp}_recovered-{route_id}`，只用于**需要新生成的非 WAV 成品**；已有有效 `final_path` 原样复用（即使名称含标题），旧 manifest 的 `identity.name` 不追溯脱敏；`true` 时保留旧恢复命名。
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

## 4. REQ 追溯（spec 003，Implemented）

| REQ | 变更/行为 | 测试/证据 |
|---|---|---|
| REQ-1（AC-1） | `naming::output_target_name`；窗口默认 `Window {seq}`；true 回退标题；成品名 sanitize / manifest 原样 target_name；`commands.rs` 启动设置传递 | `output_target_name_false_omits_window_title`、`..._true_keeps_window_title`、`..._false_seq_differs`；`route.rs` Window 分支源码 |
| REQ-2（AC-1/2） | Settings 字段默认 false/容错；设置页复选框；mock 默认 false | settings 4 个 `unit_settings_include_window_title_*` 测试；`settings-sample-rate.mjs` 50 项 |
| REQ-3（AC-3） | manifest `identity.name` 原样 target_name；恢复 stem `recovered-{route_id}`；新恢复入口 | `recovery_output_stem_default...`、`recovery_output_stem_true...`；`unit_commands_recovery_settings_snapshot_once` |
| REQ-4（AC-3/4） | v1 manifest 字段不变；旧 API true；旧文件不改名；已有 final_path 复用 | `recovery_old_v1_manifest_with_title_identity_still_parses`；recovery L2 5/5（旧 API true，非 false 命令路径） |
| REQ-5（AC-4） | 仅测试/报告，无额外生产变更 | `cargo test --workspace`；UI runner；恢复 L2；`open-items.md` OI-08、`release-checklist.md` G2-03 状态 |

## 5. 仍待后续

- 旧会话恢复列表仍可能显示旧 manifest 的窗口标题（本切片只保证新成品名/新 manifest 与恢复成品名）。
- 托盘持续指示、开始前摘要/告知、日志脱敏、About/许可证入口、CSP/IPC、浮窗、退出确认。
- 安装包重建后需验证新设置字段真实 Tauri 往返；本次只有 mock runner 与 Rust 单测。
- 本报告是工程证据，不是法律意见。
