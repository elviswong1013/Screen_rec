# 003 - Screen_rec 隐私命名与 metadata 脱敏（SDD / M19a 变更请求）

Status: Implemented
Created: 2026-10-23
Updated: 2026-10-23
Approved: 2026-10-23（plan review APPROVED + 用户批准 AC-1..4 / REQ-1..5）
Accepted: 2026-10-23（acceptor ACCEPTED：d42f9c5 实现 + 16389e5 规格/报告；源码/UI 验收，不含安装包）
PRD: `docs/prd/003-privacy-naming-redaction.md`（Approved 2026-10-23）
实现基线：提交 `d42f9c5`（实现先于本批准记录；本文件是正式变更请求，不回溯批准历史）

## 1. 范围

本规格只覆盖 M19a：窗口录制默认文件名/manifest 脱敏、Settings 开关、恢复命名与旧 v1 manifest 兼容。托盘指示、开始前摘要/告知、旧会话 UI 标题、日志脱敏、About/许可证、CSP/IPC、浮窗、退出确认、安装包/签名/法律清权均不在本规格。

## 2. 需求（REQ）

### REQ-1（AC-1）默认窗口命名

- `sr_pipeline::naming::output_target_name(window_title, seq, include_window_title)`：
  - `false` → `Window {seq}`，不含 `window_title`；
  - `true` → 原样 `window_title`，后续仍由 `sanitize_target_name` 处理。
- `start_route_impl` 的 Window 分支使用该函数；`Monitor`/`Region` 名称保持既有行为。
- 同一 `target_name` 同时用于预留成品名与新建 manifest 的 `identity.name`；**成品名使用 `sanitize(target_name)`，manifest 保存未 sanitize 的 `target_name`**（false 时两者都不含标题；true 时 manifest 保存原样标题，文件名保存 sanitize 后标题）。
- 既有 public route API 默认 `true`，保持旧测试/调用行为；命令层通过新 batch API 传入设置值（`commands.rs` 启动路径同样传递）。

### REQ-2（AC-1/AC-2）Settings 与 UI

- `Settings.include_window_title_in_filename: bool`，默认 `false`。
- 缺字段 → false；坏类型（字符串/数字/null/对象/数组）只降级本字段，不重置其他设置。
- JSON 字段名固定 `include_window_title_in_filename`；磁盘加载、`set_settings`、`Default` 同口径。
- 设置页复选框 `#set-window-title`，默认不勾选；保存/重载往返；提示文案说明标题可能含文档名/聊天对象。
- `ui/api.js` mock 默认 false；mock 的 `setSettings` 合并仅用于前端测试，不构成后端保留未知字段的契约（后端以完整 Settings 替换）。

### REQ-3（AC-3）manifest 与恢复命名

- 新录制 manifest `identity.name` 使用 REQ-1 的 `target_name`（未 sanitize；false 为 `Window {seq}`，true 为原样标题）；不新增必填字段、不改 v1 字段集。
- `sr_pipeline::recovery::recovery_output_stem(identity_name, route_id, include_window_title, timestamp)`：
  - false → `{timestamp}_recovered-{sanitize(route_id)}`；
  - true → `{timestamp}_{sanitize(identity_name)}`（旧行为）。
- 上述恢复命名只用于**需要新生成的非 WAV 成品**；已有有效 `final_path` 原样复用（即使名称含标题），旧 manifest 的 `identity.name` 不追溯脱敏。
- `finalize_session_with_options_and_container` 旧签名固定 `true`；新入口
  `finalize_session_with_options_container_and_privacy(..., include_window_title)` 由命令层传入快照值。

### REQ-4（AC-3/AC-4）旧 v1 兼容

- v1 manifest 解析字段（`identity.pid/created_at/name/path`、`segments.path/t_start/t_end/closed`）保持必填，不删除、不升级版本。
- 含窗口标题/绝对路径的旧 manifest 仍可扫描、收尾、幂等复用；旧会话最终成品名按 REQ-3 的当前开关处理，不修改旧文件。
- 旧会话恢复列表的标题显示脱敏不在本规格（后续切片）。

### REQ-5（AC-4）验证与追溯

- 测试映射（实际测试名）：
  - REQ-1：`privacy_naming.rs` 的 `output_target_name_false_omits_window_title`、`output_target_name_true_keeps_window_title`、`output_target_name_false_seq_differs`。
  - REQ-2：`settings.rs` 的 `unit_settings_include_window_title_default_false_and_round_trip`、`unit_settings_include_window_title_missing_field_false_keeps_others`、`unit_settings_include_window_title_bad_types_degrade_field_only`、`unit_settings_include_window_title_disk_load_tolerates_bad_value`；`tools/ui/settings-sample-rate.mjs` 的 50 项检查。
  - REQ-3：`recovery_output_stem_default_does_not_include_identity_title`、`recovery_output_stem_true_keeps_sanitized_identity_title`；`commands.rs` 的 `unit_commands_recovery_settings_snapshot_once`；`commands.rs` 启动路径（`start_routes`）设置传递源码追溯。
  - REQ-4：`recovery_old_v1_manifest_with_title_identity_still_parses`；既有 recovery 幂等/收尾测试保持通过；L2 recovery 5/5（注意：L2 调用旧 API，走 `true`，不等同于 false 命令路径端到端实测）。
  - REQ-5：`cargo fmt --all -- --check`、`cargo test --workspace`、设置 runner；`docs/compliance/open-items.md` OI-08 与 `docs/compliance/release-checklist.md` G2-03 的状态同步作为文档证据。
- 不要求长同步/完整 GUI/安装包；这些必须在报告中注明未验证，不得写成通过。
- 每个实现文件/测试需在 M19a 报告中映射到 REQ-1..5。

## 3. 数据/接口

- Settings JSON 新字段默认 false；旧配置缺字段兼容。
- 无新依赖、无 schema v2、无数据库/网络变化。
- 路由/恢复使用同一隐私开关；设置快照在启动/恢复执行时读取。

## 4. Out of scope

- 托盘持续指示、开始前摘要/告知、关闭主窗提示、退出确认。
- 旧会话 UI 标题脱敏、日志/错误串脱敏、About/许可证、CSP/IPC、浮窗。
- 安装包重建、签名、法律/专利/第三方通知清权。

## 5. 追溯矩阵

| REQ | 生产变更 | 测试/证据 |
|---|---|---|
| REQ-1 | `naming.rs`、`route.rs` Window 分支、`manifest identity.name`、final_path；`commands.rs:883-891` 启动设置传递 | `privacy_naming.rs` 3 项；M19a 报告 |
| REQ-2 | `settings.rs`、`ui/app.js`、`ui/api.js` | 4 个 settings 单测；`settings-sample-rate.mjs` 50 项 |
| REQ-3 | `recovery.rs` 新 stem/新入口/旧入口、`commands.rs` 快照 | `recovery_output_stem` 2 项；`unit_commands_recovery_settings_snapshot_once` |
| REQ-4 | v1 解析保持、旧 API true、旧文件不改名、已有 final_path 复用 | `recovery_old_v1_manifest_with_title_identity_still_parses`；recovery L2 5/5（旧 API true，非 false 命令路径） |
| REQ-5 | 无额外生产变更 | workspace 测试、格式检查、UI runner、恢复 L2；`open-items.md` OI-08 / `release-checklist.md` G2-03 |

## 6. 批准与状态

- plan review 于 2026-10-23 返回 APPROVED；用户同日明确批准 AC-1..AC-4 / REQ-1..5；本规格状态 `Approved`。
- 实现提交 `d42f9c5` 先于批准记录存在，已如实记录；不得倒填历史。
- acceptor 于 2026-10-23 对 `d42f9c5` + `16389e5` 复审 **ACCEPTED**（源码/UI；安装包、真实 Tauri、长媒体未验证）；本规格状态 `Implemented`。
- 批准日期使用实际日期，未回溯填写。
