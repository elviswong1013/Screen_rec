# 005 - Screen_rec：默认日志/错误脱敏（M19c）

Status: Approved
Created: 2026-10-23
Updated: 2026-10-23
Approved: 2026-10-23（用户批准；外部 plan-reviewer/oracle 因 ECONNREFUSED 不可用，按本地自审方案 + 用户批准）
前置：M19a（命名脱敏）与 M19b（录制状态/告知）已 Implemented。
关联：OI-09、release-checklist G2-04；不是法律意见。

## 1. 背景与目标

M19a 已让默认成品名/manifest 不含窗口标题，M19b 已补录制状态与开始前告知。但应用默认 stderr 与部分用户可见错误仍直接拼接完整路径、用户名、会话目录、ffmpeg/ffprobe stderr 尾部，以及原始设备 ID。release 构建是 GUI 子系统，stderr 通常不可见；但父进程、调试启动、崩溃收集或用户手动重定向仍可能捕获，属于 OI-09 / G2-04 的隐私缺口。

M19c 的目标是：**默认情况下，应用产生的 stderr 与错误提示不暴露完整绝对路径、Windows 用户名、窗口标题或原始设备 ID；保留 basename、会话 ID、错误码和可诊断关键词。** 需要完整路径时，只能通过显式诊断开关开启。

本切片只做日志/错误文本的默认脱敏，不改变录制、编码、同步、恢复、文件名、manifest 或安装包行为。

## 2. 非目标

- 不修改 manifest/settings 中持久化的绝对路径；恢复与清理仍依赖它们。
- 不脱敏 UI 的“成品在哪里”结果路径（`RouteFinalized.path`、`ScreenshotOut.path`、`MergeOut.out`、恢复会话目录等），这是产品契约。
- 不脱敏 `--self-check` 的完整搜索根/工具路径；它是用户显式执行的诊断命令。
- 不新增日志文件、遥测、上传或崩溃收集。
- 不修改 `ToolResolveError::Display` 的既有诊断输出；只在运行时日志/错误边界使用脱敏渲染。
- 不做 About/许可证入口、CSP/IPC、代码签名、专利/法务清权（另批）。
- 不承诺“所有第三方/系统/驱动日志”都被脱敏；只覆盖本应用生成并可控的字符串。

## 3. 用户故事

- US-1：我把 stderr 贴给支持人员时，不希望暴露 `C:\Users\<我的用户名>\...` 或完整输出目录。
- US-2：错误提示仍应告诉我失败的文件 basename、会话 ID、错误码和原因，否则无法排查。
- US-3：我需要复现复杂问题时，可以显式开启完整路径诊断，而不是默认泄露。
- US-4：我使用默认设置时，日志/错误不应出现窗口标题；开启“文件名包含窗口标题”是我自己的选择。

## 4. 验收标准（AC）

- **AC-1（默认路径）**：默认配置下，应用生成的 stderr 与用户可见错误不包含盘符绝对路径、UNC 共享路径、`C:\Users\<name>`、输出目录的父级或 Windows 用户名。文件路径至少保留 `<redacted>\<basename>`；`.screenrec_tmp` 会话路径保留 `.screenrec_tmp\<session_id>` 及必要 basename。
- **AC-2（错误可诊断）**：脱敏后仍保留错误码（`E_*`）、exit code、失败原因关键词、basename/会话 ID；`E_DISK_FULL` 等既有分类不得因脱敏改变。ffmpeg/ffprobe stderr 尾部只替换已知绝对路径，不整段丢弃。
- **AC-3（设备与标题）**：默认错误/状态字符串不输出原始 WASAPI 设备 ID，统一 `<device>`；设备枚举 DTO 仍返回原始 ID 供选择。默认 `include_window_title_in_filename=false` 下，日志/错误不出现窗口标题；开启标题文件名后，basename 可能含用户选择的标题，文档明确说明，不额外输出 manifest 的 raw `identity.name`。
- **AC-4（显式诊断）**：新增 `SR_DIAG_PATHS=1|true`，仅该开关开启时恢复完整路径渲染；默认关闭。`SR_DIAG_AV` 仍只控制既有采集/路由诊断，不作为路径开关。
- **AC-5（不破坏契约）**：UI 结果路径、manifest/settings 持久化、`--self-check` 输出、恢复/清理语义、录制/编码/同步行为不变；现有 DTO 与测试不得因脱敏被削弱。
- **AC-6（验证与追溯）**：新增纯函数与调用点测试；更新受影响的绝对路径断言；`cargo test --workspace` 通过；L2 恢复测试按既有 `--ignored --nocapture --test-threads=1` 口径通过；报告逐项映射 REQ-1..6。

## 5. 约束

- 不新增依赖；`sr-core` 已是 `sr-pipeline`/`sr-merge`/`src-tauri` 的依赖，脱敏工具放 `sr-core`，不把 `sr-core` 强加到 `sr-audio`/`sr-capture`/`sr-shot`。
- 只在边界层脱敏：`sr-pipeline`、`sr-merge`、`src-tauri`；这些层已有 `sr-core` 依赖或可直接调用。
- 路径处理需覆盖 Windows 盘符、UNC、正/反斜杠、非 UTF-8 lossy；替换已知路径时最长优先、ASCII 大小写不敏感、避免部分残留。
- 任何默认开关都必须 opt-in；不得通过环境变量隐式开启完整路径。
- 保持 release GUI 无控制台的行为；不新增日志文件。
- 文档需明确：脱敏降低泄露面，不保证所有父进程/系统/第三方日志；不是法律合规结论。

## 6. 分期

单批 M19c，按两条实现 lane 串行/并行：
1. `sr-core` 脱敏原语 + 单元测试。
2. 调用点接入（sr-pipeline / sr-merge / src-tauri）+ 调用点测试 + 文档。

## 7. 开放问题

- 开启 `include_window_title_in_filename=true` 时，basename 含标题是否还需要进一步哈希？本切片按“用户显式选择”处理，默认配置严格不含标题；如要求 opt-in 也完全不含标题，需在实现前追加 REQ。
- 设备友好名是否也要替换为 `<device>`？本切片默认错误只替换原始 ID；友好名可能仍出现在 UI 选择面，不进入默认错误字符串。
- 是否需要在未来增加结构化日志/日志文件？本切片不新增。

## 8. 批准

待 plan review 与用户批准 AC-1..AC-6 后进入实现；未批准前不写生产代码。
