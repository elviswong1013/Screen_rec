# M19c 默认日志/错误脱敏：实现报告

- 日期：2026-10-23
- 规格：`specs/005-log-redaction.md`（Approved；未标 Implemented）
- PRD：`docs/prd/005-log-redaction.md`（Approved）
- 状态：**源码/后端实现与自动化验证完成；独立 acceptor 两次返回 403，未取得验收结论。**
- 范围：默认 stderr 与用户可见错误的路径/设备 ID 脱敏；不改录制、编码、同步、恢复、文件名、manifest、设置格式或安装包。

## 1. 提交

| 提交 | 内容 |
|---|---|
| `8078486` | `sr-core::redact` 原语 + 单元测试（REQ-1） |
| `a1691b4` | `sr-pipeline` / `src-tauri` / `sr-merge` 边界接入 + 回归（REQ-2/3/5/6） |

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

## 4. 已知语义/风险

1. **`SR_DIAG_PATHS` 的统一性**：`sr-pipeline` 在错误构造时已脱敏，因此 opt-in 主要恢复该层 stderr；UI 绑定错误可能仍为脱敏文本。`src-tauri` 边界对上游原始文本有 opt-in 恢复路径，但对已脱敏文本无法还原。这是比规格表更严格的默认行为，但诊断 opt-in 不是全局一致；需 acceptor/用户确认是否接受，或追加“opt-in 恢复 UI 详情”改动。
2. `src-tauri` 增加了通用 `scrub_absolute_paths` 作为第二层；测试覆盖盘符/UNC/verbatim/Debug 转义/`.screenrec_tmp`，但仍可能有未覆盖的路径形态。
3. 设备处理：原始 WASAPI ID → `<device>`；友好名/GDI 稳定名保留；枚举 DTO 不变。
4. 默认 `include_window_title_in_filename=false` 下不输出标题；开启标题文件名时 basename 可能含用户选择的标题（PRD 开放问题）。
5. 未验证安装包内资源/真实 stderr 重定向；未改安装包。

## 5. 验收阻塞

- `acceptor` 第一次（acc-13）返回 403；换新会话（acc-14）再次 403。两次均未产生 ACCEPTED/REJECTED 结论。
- 因此 SDD 005 保持 `Approved`，不得标 `Implemented`；OI-09/G2-04 只能标“代码/测试完成，独立验收阻塞”。

## 6. 结论

实现与自动化证据已具备，默认路径/设备 ID 泄露面已收紧；但独立验收门禁未完成。下一步需 acceptor 恢复后复审，或由用户明确接受当前“自审 + 自动化证据”并承担未独立验收的风险。
