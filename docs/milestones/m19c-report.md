# M19c 默认日志/错误脱敏：实现报告

- 日期：2026-10-23
- 规格：`specs/005-log-redaction.md`（Approved；未标 Implemented）
- PRD：`docs/prd/005-log-redaction.md`（Approved）
- 状态：**源码/后端实现与自动化验证完成；acceptor 首轮 REJECTED（4 项 P1/P2），修复已提交 `d88efbe`，等待复审。**
- 范围：默认 stderr 与用户可见错误的路径/设备 ID 脱敏；不改录制、编码、同步、恢复、文件名、manifest、设置格式或安装包。

## 1. 提交

| 提交 | 内容 |
|---|---|
| `8078486` | `sr-core::redact` 原语 + 单元测试（REQ-1） |
| `a1691b4` | `sr-pipeline` / `src-tauri` / `sr-merge` 边界接入 + 回归（REQ-2/3/5/6） |
| `43e3e10` | 通用路径扫描 + opt-in 渲染 helper |
| `d88efbe` | 验收修复：opt-in 恢复、空格/引号/Debug 路径、settings reason |

## 2. 实现要点

- `sr-core::redact`：`redact_path`、`redact_text_known_paths`、`redact_device_id`、`redact_tool_error`、`paths_verbose_from`/`paths_verbose`；无新增依赖。
- 默认路径只保留 `<redacted>\<basename>`；`.screenrec_tmp` 路径保留会话后缀；UNC/盘符/父目录不输出。
- `SR_DIAG_PATHS=1|true` 打开路径诊断；`SR_DIAG_AV` 仍只控制采集/路由诊断，不作为路径开关。
- `sr-pipeline`：finalize/sidecar/recovery/route 的 stderr 与错误详情脱敏；ffmpeg/ffprobe stderr 尾部按已知路径替换；`E_DISK_FULL` 在脱敏前分类。
- `sr-merge`：合并/探测错误的路径与 stderr 尾部脱敏；不改变 merge 语义与 `MergeOut.out`。
- `src-tauri`：settings 解析失败路径、route/shot/merge/recovery 错误字符串、resource_dir 失败、StopNotice 原始 WASAPI ID 边界脱敏。
- 保留：UI 结果路径、manifest/settings 持久化、`--self-check`、`ToolResolveError::Display`、设备枚举 DTO。

## 3. 验证（已观察）

- `cargo fmt --all -- --check`：通过。
- `cargo test --workspace`：全部非 ignored 测试通过；91 ignored（L2/L3 门禁）。
- `cargo test -p sr-itests --test recovery -- --ignored --nocapture --test-threads=1`：**5 passed / 0 failed**。
- `cargo test -p sr-itests --test probe --no-run`：编译通过。
- 未跑长媒体、未重建安装包、未做 GUI L3。

## 4. 验收修正（acc-15 REJECTED → d88efbe）

1. **opt-in 恢复**：`sr-pipeline` 的错误构造改用 `render_path` / `render_text_known_paths` / `render_tool_error`；`SR_DIAG_PATHS=1` 现在能恢复该层错误详情中的完整路径。stderr 仍由同一开关控制。
2. **通用扫描**：新增 `redact_text_paths`，覆盖盘符/UNC/正斜杠、引号、空格、撇号、括号、Debug 双反斜杠；`src-tauri` 的本地扫描器改为委托该函数。`C:\Users\Alice Smith\Private\file.mkv` 现在输出 `<redacted>\file.mkv`。
3. **Debug 提交错误**：finalize/route 的 `CommitError::FinalExists` 等 Debug 路径改走 `render_text_paths`；known-path 匹配也增加 Debug 变体。
4. **settings reason**：解析失败日志既脱敏文件名，也脱敏 serde reason 中回显的路径/设备值；保留错误类别。
5. **隐私文档**：`privacy-policy-draft.md` 更新默认脱敏、`SR_DIAG_PATHS`、托盘状态和标题文件名默认值；仍是草案，不是最终法律文本。

## 5. 验收状态

- 初次 acceptor 两次 403；随后 acc-15 给出 **REJECTED**（4 项 P1/P2）。
- 修复提交 `d88efbe`；`cargo test --workspace` 全部非 ignored 通过，L2 recovery **5/5**，格式检查通过。
- **等待 acc-15 复审；SDD 005 仍为 `Approved`，不得标 `Implemented`。**

## 6. 残余风险

1. 开启 `include_window_title_in_filename=true` 时，basename 可能含用户选择的标题；默认 false 严格不含标题。
2. 未重建安装包、未做真实 stderr 重定向/长媒体验证。
3. 通用扫描器是文本启发式；已知路径优先，未知路径的边界仍可能有误判。
4. 已保留 UI 结果路径、manifest/settings、`--self-check`、`ToolResolveError::Display`；未改录制/同步/恢复语义。
