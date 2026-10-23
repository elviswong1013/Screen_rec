# M19d1 About/许可证入口：实现报告

- 日期：2026-10-23
- 规格：`specs/006-about-licenses.md`（Approved；未标 Implemented）
- PRD：`docs/prd/006-about-licenses.md`（Approved）
- 状态：**源码/UI/自动化验证完成；独立 acceptor 通道仍不可用。**
- 范围：About 导航、版本/系统/FFmpeg 构建、固定 5 个已随包 FFmpeg 文档查看；不接入 notices/SBOM/EULA/隐私草案。

## 1. 实现

- `src-tauri/src/legal.rs`：固定 allowlist（5 文档）、版本、FFmpeg commit/date 解析、资源根解析（resource_dir → exe 同目录 → 开发回退）、canonical root 校验、4 MiB 上限、UTF-8/缺失通用错误。
- `src-tauri/src/lib.rs`：注册 `get_app_info` / `read_legal_doc`。
- `ui/index.html` / `ui/app.js`：第 5 个「关于」视图；版本/Windows/FFmpeg 构建；5 个文档按钮；懒加载纯文本 `textContent`。
- `ui/api.js`：real/mock 两个命令；mock 固定 5 文档。
- `tools/ui/about-licenses.mjs`：About runner。
- `docs/design/ui-spec.md`：导航从四个视图更新为五个，补 About 视图说明。

## 2. 验证（已观察）

- `cargo test -p screen-rec`：94 passed / 0 failed（含 6 个 legal 测试）。
- `cargo test --workspace`：全部非 ignored 测试通过。
- `node tools/ui/about-licenses.mjs`：全部通过（版本、FFmpeg 构建、5 文档、正文加载、无效 ID 拒绝、textContent 静态断言）。
- `node tools/ui/recording-visibility.mjs`、`audio-sources-builder.mjs`、`region-record.mjs`、`settings-sample-rate.mjs`：全部通过。
- `cargo fmt --all -- --check`：通过。
- 未重建安装包、未做真实 Tauri GUI/安装目录读取验证。

## 3. 限制/未覆盖

- 未独立 acceptor；SDD 006 保持 Approved，不标 Implemented。
- 只展示已随包 FFmpeg 5 文档；`third-party-notices.md`、licenses/SBOM、EULA/隐私草案未接入（M19d2，需法律审阅）。
- 未显示项目自身 MIT/Apache（无根 LICENSE）、未签名状态、专利/许可结论。
- 未重建安装包；真实 `resource_dir`/exe 同目录读取仅由代码路径和开发回退覆盖。
- About runner 使用 mock 分支；真实命令由 Rust 单测与编译覆盖，未做 GUI 手动签核。

## 4. 结论

M19d1 源码/UI 与自动化验证完成，低法律风险范围内可用；正式完成仍需 acceptor（或用户明确接受自审）与安装包内真实文档读取验证。
