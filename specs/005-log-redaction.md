# 005 - Screen_rec：默认日志/错误脱敏（M19c）

Status: Approved
Created: 2026-10-23
Updated: 2026-10-23
PRD: `docs/prd/005-log-redaction.md`
Approved: 2026-10-23（用户批准；外部 plan-reviewer/oracle 因 ECONNREFUSED 不可用，按本地自审方案 + 用户批准）
前置：M19a（命名脱敏）、M19b（录制状态/告知）已 Implemented。
关联：OI-09、release-checklist G2-04。

## 1. 背景与目标

默认 stderr 与部分用户可见错误会把完整路径、Windows 用户名、会话目录、ffmpeg/ffprobe stderr 尾部、原始设备 ID 拼进字符串。release GUI 无控制台，但父进程/调试启动/重定向仍可能捕获。M19c 在**边界层**统一脱敏，保留 basename、会话 ID、错误码和诊断关键词；完整路径只能通过 `SR_DIAG_PATHS=1` 显式开启。

本规格只改日志/错误渲染与测试，不改录制、编码、同步、恢复、文件名、manifest、设置格式或安装包行为。

## 2. 需求（REQ）

### REQ-1：共享脱敏原语（`sr-core::redact`）

新增 `crates/sr-core/src/redact.rs`，不新增依赖：

- `pub fn paths_verbose() -> bool`：运行期包装；`pub fn paths_verbose_from(value: Option<&str>) -> bool` 为纯函数，仅 `1|true` 返回 true。测试用纯函数避免并行环境变量竞态，另有一条串行化环境变量测试。
- `pub fn redact_path(path: &Path) -> String`：
  - 含 `.screenrec_tmp` 组件时，保留从该组件起的后缀（如 `.screenrec_tmp\<session_id>\seg.mkv`）；
  - 其他路径返回 `<redacted>\<basename>`；
  - 无 basename/空路径返回 `<redacted>`；
  - 非 UTF-8 使用 lossy；不返回盘符、UNC 前缀或父目录。
- `pub fn redact_text_known_paths(text: &str, known: &[&Path]) -> String`：
  - 对每个已知路径同时匹配 `display()` 与正斜杠形式；
  - 最长优先、ASCII 大小写不敏感、替换为 `redact_path` 结果；
  - 不整段丢弃 stderr，不改变错误关键词/退出码。
- `pub fn redact_device_id(_id: &str) -> &'static str`：返回 `<device>`（默认错误/状态用；不用于 UI 枚举 DTO）。
- `pub fn redact_tool_error(err: &ToolResolveError) -> String`：在 `sr-core` 内提供脱敏渲染，但不改 `Display`；保留尝试顺序/错误类别，路径用 `redact_path`。

`sr-core` 不新增对外依赖；`sr-audio`/`sr-capture`/`sr-shot` 不新增 `sr-core` 依赖，统一在 `sr-pipeline`/`src-tauri` 边界脱敏。

### REQ-2：默认 stderr 脱敏

所有默认 `eprintln!` 中可控的路径/设备 ID 使用 REQ-1；至少覆盖：

| 区域 | 代表位置 | 处理 |
|---|---|---|
| 设置加载失败 | `src-tauri/src/settings.rs:406` | 路径 `redact_path`，保留 serde 原因 |
| 收尾段/成品校验 | `sr-pipeline/src/finalize.rs:91,99,118,134,178,182,514-577,777-780` | 路径 `redact_path`；ffprobe `stderr` 先按已知路径替换 |
| ffprobe 工具解析 | `finalize.rs:808,831` | `redact_tool_error`；segment 路径 `redact_path` |
| 编码器探测/spawn | `sr-pipeline/src/sidecar.rs:146,165,182,209,466` | `redact_tool_error` / 路径 `redact_path` |
| 恢复清理/manifest | `sr-pipeline/src/recovery.rs:1005-1008` 及既有 manifest 错误 | session/manifest 路径 `redact_path` |
| 路由回滚/清理 | `sr-pipeline/src/route.rs:724,2797-2800` | 先格式化错误，再对已知路径脱敏；不改变回滚语义 |
| 资源目录解析 | `src-tauri/src/lib.rs:304` | 路径/错误文本脱敏 |
| 托盘/窗口 API 错误 | `lib.rs:191-260` | 无已知用户数据；不强制改，但不得新增路径拼接 |

`finalize`/`recovery`/`route` 的默认 `eprintln!` 不得再直接 `{path:?}`/`{}` 输出完整路径。

### REQ-3：用户可见错误脱敏

默认错误 DTO/字符串在边界层使用 REQ-1：

- **边界原则**：内部 `RouteError`/`RecoveryError`/`FinalizeError` 保留原始路径，以维持恢复、校验和既有测试语义；只在 **stderr 输出** 与 **src-tauri 转 UI 字符串** 的边界脱敏。具体边界：`route_error_message`（`commands.rs:1561`）、`shot_error_string`（`:1565`）、`merge_error_string`（`:1580`）、`recovery_error_string`（`:1597`）、`notice_out`（`:707`）、`SessionOut`/NotFound 生成处；保留 `E_*` 前缀/错误码。
- `sr-merge` 的 ffmpeg/ffprobe 错误：内部保留原始路径；`merge_error_string` 边界用 `redact_path`，stderr 尾部按已知路径替换，保留可诊断原因。
- `sr-shot`/截图错误：内部保留；`shot_error_string` 边界输出目录/路径 `redact_path`，basename 保留。
- 设备 ID：默认错误/状态（如 `TargetInvalid`、`StopNoticeOut.endpoint`、`CaptureError` 暴露到 UI 的字符串）用 `<device>`；设备枚举 DTO（`render_endpoints`、音频设备列表）保持原始 ID。`capture_start_error`（`route.rs:968`）在转 `RouteError` 时处理设备错误，`notice_out` 在转 DTO 时处理 endpoint。
- 区域显示器 key/友好名进入错误时用 `<device>` 或稳定索引，不输出完整 `display_device` 路径；UI 选择面不变。
- 窗口标题：默认 `include_window_title_in_filename=false`；默认日志/错误不得出现标题。开启标题文件名时，basename 可能含用户选择的标题，按 PRD 开放问题处理，不额外输出 manifest 的 raw `identity.name`。

### REQ-4：诊断 opt-in

- 新增 `SR_DIAG_PATHS=1|true`：仅路径渲染恢复完整路径；默认关闭。
- `SR_DIAG_AV` 保持既有采集/路由统计与端点失效诊断；**不作为**路径开关；其 raw device ID 仍属显式诊断。
- 文档说明：release GUI 默认无控制台；父进程仍可能捕获 stderr；opt-in 仅供本地排查。
- 不在日志中打印环境变量值或用户目录。

### REQ-5：保留契约

以下不脱敏/不改语义：

- UI 结果路径：`RouteFinalized.path`、`ScreenshotOut.path`、`MergeOut.out`、`SessionOut.dir/sealed_segments/final_path`、恢复列表的会话路径。
- `manifest.json` / `settings.json` 持久化绝对路径；恢复、校验、清理继续使用。
- `--self-check` 的搜索根/工具路径输出；`ToolResolveError::Display` 保持原样。
- 录制/编码/同步/恢复/文件名/设置/安装包行为。
- `E_DISK_FULL` 等关键词分类必须在 stderr 脱敏**之前**执行；退出码/错误码不变。

### REQ-6：验证与文档

- 新增 `sr-core` 单元测试；调用点测试覆盖 finalize/route/recovery/commands/merge 的错误字符串。
- 更新因绝对路径断言而受影响的恢复测试：改为“basename 存在、父目录/用户名不存在”，不削弱“校验的是正确文件”的意图。
- `cargo fmt --all -- --check`、`cargo test --workspace`；L2 恢复测试按 `--ignored --nocapture --test-threads=1` 运行。
- 不跑长媒体；不改录制管线。
- 更新 `docs/compliance/open-items.md` OI-09、`release-checklist.md` G2-04、`privacy-policy-draft.md` 的 stderr 说明；新增 `docs/milestones/m19c-report.md`，逐项映射 REQ-1..6。
- 未完成前不把 OI-09/G2-04 标为完成。

## 3. 架构 / 接口 / 数据

```
sr-core::redact
  paths_verbose()
  redact_path(&Path) -> String
  redact_text_known_paths(text, known: &[&Path]) -> String
  redact_device_id(&str) -> &'static str
  redact_tool_error(&ToolResolveError) -> String

sr-pipeline / sr-merge / src-tauri
  error construction / stderr logging
    -> redact::* at boundary
```

- `sr-core` 已是 `sr-pipeline`、`sr-merge`、`src-tauri` 的依赖；不新增依赖，不改 `sr-audio`/`sr-capture`/`sr-shot` Cargo.toml。
- `sr-audio`/`sr-capture`/`sr-shot` 的错误在 `sr-pipeline`/`src-tauri` 转成 UI 字符串时脱敏。
- `redact_text_known_paths` 的 known 列表由调用方提供：输出目录、会话目录、最终路径、临时路径、concat/remux 路径、segment 路径；不依赖全局状态。
- 不引入日志文件/结构化日志；仅改现有 stderr 与错误 DTO。

## 4. 错误路径

| 场景 | 默认行为 | opt-in |
|---|---|---|
| settings.json 解析失败 | `<redacted>\settings.json` + 原因 | `SR_DIAG_PATHS=1` 显示完整路径 |
| ffprobe/ffmpeg stderr 含路径 | 已知路径替换为 `<redacted>\...`，保留关键词/exit code | 完整路径 |
| 恢复/清理失败 | 会话路径 `.screenrec_tmp\<id>`，不显示父目录 | 完整路径 |
| 设备不存在/端点失效 | `<device>` | `SR_DIAG_AV=1` 可保留 raw ID |
| 截图/合并输出失败 | `<redacted>\<basename>` | 完整路径 |
| UI 结果路径 | 不脱敏 | 不脱敏 |
| `--self-check` | 不脱敏 | 不脱敏 |

脱敏函数不得因路径不存在、非 UTF-8、空字符串或 UNC 路径 panic；失败时返回 `<redacted>`。

## 5. 测试计划（REQ → 测试映射）

| REQ | 测试 |
|---|---|
| REQ-1 | `crates/sr-core` 单元测试：盘符/UNC/正反斜杠/`.screenrec_tmp`/空/非 UTF-8；`redact_text_known_paths` 最长优先、大小写、部分匹配；`paths_verbose` 默认 false、`SR_DIAG_PATHS=1` true（测试串行设置环境变量）。 |
| REQ-2 | `finalize` 校验错误含 basename、无 `C:\Users\`；`route` 注入 stderr tail，断言路径被替换、`permission denied`/`E_DISK_FULL` 仍可分类；`recovery` 清理错误无父目录；`settings` 解析错误无用户名。 |
| REQ-3 | `commands` 的 `recovery_error_string`/`merge_error_string`/`shot_error_string` 保留 `E_*`/basename，去除父目录；设备错误为 `<device>`；默认标题文件名测试不出现标题。 |
| REQ-4 | `paths_verbose` 默认 false；`SR_DIAG_PATHS=1` 恢复完整路径；`SR_DIAG_AV` 不改变默认路径行为。 |
| REQ-5 | 现有 DTO/结果路径测试保持；`ToolResolveError::Display` 测试保持；manifest/settings 读写测试保持；`E_DISK_FULL` 分类测试保持。 |
| REQ-6 | `cargo fmt --all -- --check`、`cargo test --workspace`、L2 recovery ignored；报告映射；OI/G2 文档更新。 |

## 6. Out of scope

- 日志文件/滚动/保留期限/上传。
- manifest/settings 持久化格式或 UI 结果路径脱敏。
- `--self-check` 输出脱敏。
- About/许可证、CSP/IPC、签名、专利/法务。
- 修改 `sr-audio`/`sr-capture`/`sr-shot` 的依赖图。
- 保证第三方/系统/驱动日志不泄露。

## 7. 决策记录

- 选 `sr-core` 作为共享原语：零依赖、已被主要边界 crate 依赖；不把 `sr-core` 强加到音频/捕获/截图 crate。
- 保留 basename/会话 ID 而不是全量 `<redacted>`：维持可诊断性，符合 OI-09 建议。
- 内部错误保留原始路径，边界统一脱敏：避免破坏恢复/校验语义和既有测试，同时确保所有 UI/stderr 出口安全。
- `paths_verbose_from` 纯函数 + `paths_verbose()` 包装：测试不依赖进程级环境变量顺序。
- 不修改 `ToolResolveError::Display`：`--self-check` 与既有工具解析测试依赖完整路径；新增 `redact_tool_error` 仅用于运行时日志/错误。
- `SR_DIAG_PATHS` 与 `SR_DIAG_AV` 分离：路径泄露与采集诊断是不同风险，避免一个开关隐式打开另一个。
- 默认标题文件名不含标题；opt-in 标题的 basename 例外记录在 PRD 开放问题，若需严格处理应在实现前追加 REQ。

## 8. 批准与状态

- Status: Draft；待 plan review 与用户批准 AC-1..AC-6 / REQ-1..6。
- 未批准前不写生产代码；批准后按 TDD 红→绿实现，acceptor 验收。
