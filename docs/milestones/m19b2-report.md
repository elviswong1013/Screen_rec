# M19b2 开始栏摘要 / 首次告知 / ack（REQ-3/4）

- 日期：2026-10-23
- 基线：`e7df398`（M19b1 托盘/关闭提示提交后）
- 规格：`specs/004-recording-visibility.md`（Implemented）REQ-3/4
- 状态：**源码/UI/自动化通过，acceptor ACCEPTED；L3 2026-10-23 用户签核「目前一切正常」。** SDD 004 已标 `Implemented`。
- 实现提交 `d5c3d70`；两击绕过修复 `3a190a4`。L3 使用后续内部测试包，未签名；未改录制/同步/恢复语义。

## 1. 实现行为

### 后端（REQ-4 字段）

- `Settings.recording_notice_ack: bool`：默认 false；缺字段 false；字符串/数字/浮点/null/对象/数组等坏类型只降级本字段；复用 `deserialize_lenient_bool`；`Default`/往返/磁盘加载测试覆盖。

### 前端（REQ-3/4）

- 单一 `buildStartPlan()` 同时生成启动 DTO、摘要与 `needs_notice`，覆盖窗口/逐屏/区域/纯录音。
- 摘要含目标、音源标签、输出目录、格式；纯录音显示 WAV、不显示画布/帧率；区域显示最终选区尺寸。
- 敏感音源 = mic/system/render_endpoint；`needs_notice && !recording_notice_ack`：
  - 第一次点开始只展开摘要 + 告知，不调用启动 API；
  - 第二次“确认并开始”才启动；
  - 目标/音源/输出/格式变化、取消、启动失败/完成 → 复位；每次新启动尝试都需重新确认；
  - 仅 app 音源或 ack=true 一次启动；零音源仍拒绝；区域 overlay 确认不算开始确认；Ctrl+Enter 同门禁且防 repeat。
- ack 勾选/取消：写队列内先取最新 Settings、只改 ack、再整体保存；设置页保存也在同一队列内读取 base 并合并表单，保留 ack/M19a 字段；保存失败不伪装成功。
- **并发修复**：ack 保存与设置页保存共用 `enqueueSettingsWrite`，避免旧 Settings 后写覆盖新值（runner 复现并验证）。
- 旧 runner 适配：`audio-sources-builder.mjs` 显式把 mock ack 设为 true，保留原有启动 DTO 断言；新增 `recording-visibility.mjs` 覆盖两击/取消/ack/设置页。

## 2. 关键文件

| 文件 | 变更 |
|---|---|
| `src-tauri/src/settings.rs` | `recording_notice_ack` + 4 个测试 |
| `ui/app.js` | `buildStartPlan`、确认状态机、告知/ack、Settings 写队列、设置页保留 ack |
| `ui/api.js` | mock 默认 `recording_notice_ack=false` |
| `tools/ui/recording-visibility.mjs`（新） | 摘要/两击/取消/配置复位/Ctrl+Enter/零音源/区域/纯录音/ack/设置页 |
| `tools/ui/audio-sources-builder.mjs` | 旧 DTO 测试显式 ack=true，保留原断言 |
| `docs/milestones/m19b2-report.md` | 本报告 |

## 3. 红 → 绿

- 后端红：`cargo test -p screen-rec --lib unit_settings_recording_notice_ack` → 7 处 `no field recording_notice_ack`；绿 → 4 passed。
- 前端 runner 红→绿：
  - 初次运行 `recording-visibility.mjs`：2 项失败（音源变化未复位、区域场景等待 `#start-plan` 超时）；根因是音轨模式/麦克风 handler 只 `renderConfig()` 未刷新开始栏。
  - 修复后：全部行为检查通过；随后暴露设置页保存与 ack 并发覆盖，改为 Settings 写队列 + base 合并后通过。
  - runner 自身修正：CDP `Runtime.evaluate` 顶层 `await` 改为 async IIFE；增加“设置页选择 mp4 生效”断言。

## 4. 验证结果

- `cargo fmt --all -- --check`：通过。
- `cargo test --workspace`：190 passed / 0 failed / 91 ignored（后端字段提交时；UI 修改不改变 Rust 测试）。
- `node tools/ui/recording-visibility.mjs`：全部通过（摘要、仅 app 一次启动、mic/system/render_endpoint 两击、取消、配置复位、Ctrl+Enter、零音源拒绝、区域、纯录音、ack 往返、设置页保留、保存失败不伪装）。
- `node tools/ui/audio-sources-builder.mjs`：全部通过。
- `node tools/ui/region-record.mjs`：全部通过（mock-only；真实 GUI 由 E2E）。
- `node tools/ui/settings-sample-rate.mjs`：全部通过。
- 未跑长媒体；L3 已签核；安装包为内部测试、未签名。

## 5. L3 人工项（2026-10-23 已签核）

以下 1–5 已由用户现场执行并反馈「目前一切正常」；未单独覆盖多路/混合状态与长媒体。

1. 真实主窗开始：含麦克风/系统/端点 → 首次展开告知、二次确认；仅应用一次启动。
2. 勾选/取消“不再显示”，重启应用后 ack 保持；设置页保存其他字段后 ack 不丢。
3. 区域 overlay 确认后仍需开始确认；纯录音 WAV 摘要。
4. 真实 Tauri `get_settings/set_settings` 往返（由用户重启/设置页操作覆盖）。
5. L3 真实托盘/关闭提示（b1）已签核；两批 L3 均有现场证据。

## 6. 范围

- 本切片不含托盘状态（b1 已完成）、浮窗/红点、托盘开始确认、退出确认、日志/About/CSP、安装包/签名。
- SDD 004 已因 b1+b2 acceptor ACCEPTED + L3 签核标 `Implemented`；正式发行仍未完成。

## 7. 验收修正（acceptor 回归）

- 首次 acceptor 复现：手动点击「摘要」会直接进入确认态，随后第一次点「开始」即调用启动 API，违反 REQ-3「第一次点开始只展开、不启动」。
- 修复：`startConfirm` 拆为 `idle` / `summary`（手动摘要）/ `confirmed`（第一次开始）；手动摘要只展示内容，第一次开始只转 `confirmed` 并返回，第二次且签名匹配才启动；配置变化/收起/取消仍复位。
- 新增 `recording-visibility.mjs` 2b 回归：手动摘要不启动、第一次开始不启动且按钮变「确认并开始」、第二次开始恰好一次；旧实现反证失败，新实现全部通过。
- 复审：acceptor **ACCEPTED（source/UI only）**；L3 真实托盘/关闭/告知及真实 IPC 持久化已于 2026-10-23 用户签核「目前一切正常」，SDD 004 已标 `Implemented`（见 `m19b-l3-signoff.md`）。
