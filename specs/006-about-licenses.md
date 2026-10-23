# 006 - Screen_rec：About/许可证入口（M19d）

Status: Approved
Created: 2026-10-23
Updated: 2026-10-23
PRD: `docs/prd/006-about-licenses.md`
Approved: 2026-10-23（用户批准；外部 plan-reviewer/oracle 不可用，按本地自审方案 + 用户批准）
前置：M19a/M19b 已 Implemented；M19c 日志脱敏代码/测试完成，独立验收阻塞（不阻塞本切片）。
关联：OI-12、OI-13；不是法律意见。

## 1. 背景与目标

应用没有 About/版本/许可证入口；安装包已随附 5 份 FFmpeg 文档，但 UI 不可见。M19d 只做 About 入口、版本/系统/FFmpeg 构建信息和**已随包 FFmpeg 文档**的只读查看器；候选/草案法律材料留到 M19d2。

本切片不新增依赖、不新增 Tauri ACL/fs 插件、不允许 UI 传任意路径。

## 2. 需求（REQ）

### REQ-1：About 入口与元数据

- 新增第 5 个导航项「关于」及 `#view-about`；沿用现有 `.nav-item` 路由（`ui/index.html`、`ui/app.js`）。
- 显示：
  - 应用版本：运行时 `app.package_info().version`（bundle authoritative）；Rust 测试与 `tauri.conf.json`/Cargo 版本一致。
  - Windows 构建：复用前端 `state.caps.os_build`（不新增后端探测）。
  - FFmpeg 构建：读取随包 `ffmpeg-build-info.txt` 的 `FFmpeg commit:` 与 `Build date:` 行；缺失/解析失败显示 `unknown`。不展示 `Base image: ...:latest` 为已验证 digest，不声称专利/许可清权。
- 不显示项目自身 MIT/Apache（当前无根 LICENSE），不显示“正式签名/法律合规”等未证实状态。

### REQ-2：固定文档目录

- 后端固定 5 个文档 ID：
  - `ffmpeg_license` → `FFmpeg-LICENSE.txt`
  - `gplv3` → `GPLv3.txt`
  - `ffmpeg_source_offer` → `ffmpeg-source-offer.txt`
  - `ffmpeg_build_info` → `ffmpeg-build-info.txt`
  - `ffmpeg_build_script` → `ffmpeg-build-script.sh`
- `get_app_info` 返回版本、FFmpeg 构建标识和 `docs: [{id,title,status,bytes}]`；status 固定 `shipped`。
- 不列出 `docs/compliance` 整目录；不接入 notices/licenses/SBOM/EULA/隐私草案。

### REQ-3：安全只读命令

- 新模块 `src-tauri/src/legal.rs`；命令：
  - `get_app_info() -> AppInfoOut`
  - `read_legal_doc(id: String) -> LegalDocOut { id, text, bytes }`
- `read_legal_doc` 只接受固定 ID；拒绝空/未知/路径分隔符/绝对路径/`..`。
- 资源根解析顺序：`sr_core::tools::registered_resource_dir()` → exe 同目录 → `CARGO_MANIFEST_DIR/resources/licenses` 开发回退；只读。
- 文件名来自 allowlist，不来自 UI；canonicalize 后必须 `starts_with(root)`；最大 4 MiB；UTF-8 失败/缺失/超限返回通用错误，错误文本用 `sr_core::redact` 脱敏，不回显绝对路径。
- 不新增 `tauri-plugin-fs`/ACL；只注册两个自定义命令。

### REQ-4：UI 查看器

- `ui/index.html` 新增 About 视图；`ui/app.js` 新增 `renderAbout()`，加载 `get_app_info`，点击文档时懒加载。
- 文档标题/状态/大小用现有 `esc()`；正文用 `textContent`，不用 `innerHTML`。
- 加载失败显示通用提示；不把异常路径写入 DOM。
- `ui/api.js` 增加 real/mock 两个命令；mock 返回固定 5 文档与示例文本，便于 runner。
- `docs/design/ui-spec.md` 的“四个主视图”更新为五个。

### REQ-5：打包与可追溯

- `src-tauri/tauri.conf.json` 已映射 5 个 FFmpeg 文档；本切片不新增资源文件，不改变映射（除非修复路径）。
- 安装包内 `licenses/` 5 文件与仓库源 SHA-256 一致；About 读取安装目录/exe 同目录，不读用户输出目录。
- 记录安装包大小变化（预期仅前端代码几 KB；无新增资源）。
- 不重建正式发行包/签名；若重建，标注内部测试。

## 3. 架构 / 接口 / 数据

```
src-tauri/src/legal.rs
  const DOCS: [LegalDoc; 5]  // id, relative file, title
  AppInfoOut { version, ffmpeg_build, docs }
  LegalDocOut { id, text, bytes }
  get_app_info(app) -> AppInfoOut
  read_legal_doc(app, id) -> Result<LegalDocOut, String>

ui/index.html  #view-about / nav
ui/app.js     renderAbout(), loadLegalDoc()
ui/api.js     getAppInfo(), readLegalDoc(id)
```

- 版本使用 `app.package_info().version.to_string()`；测试读取 `tauri.conf.json` 与 Cargo 版本比对。
- FFmpeg 构建解析只读文本，不执行 sidecar、不联网。
- 资源解析函数复用 `sr_core::tools::{registered_resource_dir, current_search_paths}` 的目录优先级；开发回退用于 `cargo test`/dev。
- 错误字符串使用 `sr_core::redact::redact_path`/`redact_text_paths`（M19c helper）避免泄漏；M19c 未验收不影响该只读用途。

## 4. 错误路径

| 场景 | 行为 |
|---|---|
| 未知/空/含分隔符 ID | 通用 `invalid legal document`，不读取文件 |
| 文档缺失 | 通用“文档不可用”，不含路径 |
| 文件 >4 MiB | 拒绝，提示过大 |
| 非 UTF-8 | 拒绝，提示不可读 |
| 资源根缺失 | 尝试 exe 同目录/开发回退；全失败返回通用错误 |
| 版本/FFmpeg 解析失败 | 显示 `unknown`，不阻塞 About |
| 安装包文档哈希不符 | 打包检查失败，不进入正式发布 |

## 5. 测试计划（REQ → 测试）

| REQ | 测试 |
|---|---|
| REQ-1 | `legal::tests`：版本与 tauri.conf/Cargo 一致；FFmpeg commit/date 解析；缺失显示 unknown；UI runner 显示版本/OS。 |
| REQ-2 | 固定 5 ID/标题/文件；未知/空/`..`/`/`/`\`/绝对路径拒绝；不做目录扫描。 |
| REQ-3 | canonical root escape 拒绝；缺失/超 4 MiB/非 UTF-8 通用错误；错误串无绝对路径；只读语义；现有 ACL 不变。 |
| REQ-4 | `tools/ui/about-licenses.mjs`：导航、元数据、5 文档、懒加载、文本转义、错误提示；real/mock API 形状；`textContent` 静态断言。 |
| REQ-5 | 安装包 `licenses/` 5 文件哈希与仓库一致；`cargo test --workspace`；现有 4 个 UI runner；安装包大小记录。 |

## 6. Out of scope

- `third-party-notices.md`、`third-party-licenses.txt`、SBOM、ffmpeg-components（M19d2）。
- EULA/隐私政策/recording guidelines 的正式展示或 clickwrap。
- 代码签名、CSP/IPC、专利/法务清权、安装包正式发行。
- 任意文件查看器、目录浏览、写文件。
- 修改 FFmpeg 资源内容或 source offer。

## 7. 决策记录

- 选第 5 个导航页而非设置面板：现有路由器通用，长文本可滚动；设置面板 560px 且每次重建会丢状态。需更新 `ui-spec.md`。
- 选固定 allowlist + 后端命令：不新增 fs 插件/ACL，UI 不能传路径；符合最小权限。
- 只接入已随包 FFmpeg 文档：候选/草案法律材料未经审阅，不能显示为最终条款。
- 不显示项目许可证/签名/专利状态：当前无根 LICENSE、未签名、未清权。

## 8. 批准与状态

- Status: Approved（2026-10-23 用户批准；外部 plan-reviewer/oracle 不可用，按本地自审方案 + 用户批准）。
- 实现：`src-tauri/src/legal.rs` + About 视图/API/runner；`cargo test -p screen-rec` 94 passed，workspace 全绿，About + 既有 4 个 UI runner 通过。
- **独立 acceptor 通道仍不可用；本规格未标 `Implemented`。**
- 未重建安装包、未做真实 Tauri 安装目录读取/GUI 手动签核。
