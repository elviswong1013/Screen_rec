# 006 - Screen_rec：About/许可证入口（M19d）

Status: Approved
Created: 2026-10-23
Updated: 2026-10-23
Approved: 2026-10-23（用户批准；外部 plan-reviewer/oracle 不可用，按本地自审方案 + 用户批准）
前置：M19a（命名脱敏）、M19b（状态/告知）已 Implemented；M19c（日志脱敏）代码/测试完成但独立验收阻塞。
关联：OI-12、OI-13；不是法律意见。

## 1. 背景与目标

应用目前没有 About/版本/许可证入口；安装包只带 FFmpeg 的 5 份文档，第三方 notices/SBOM 仍在 `docs/compliance` 候选状态。用户无法在应用内查看版本、FFmpeg 构建信息和已有许可证/源码说明。

M19d 只做**低法律风险**的 About 入口与只读文档查看器：展示应用版本、系统信息、已随包的 FFmpeg 文档；不接入未审阅的 EULA/隐私政策/第三方 notices/SBOM，不提供 clickwrap，不宣称专利/许可清权完成。

## 2. 非目标

- 不集成 `third-party-notices.md`、`third-party-licenses.txt`、SBOM、EULA/隐私草案（M19d2，需法律审阅）。
- 不添加 EULA 接受页、隐私同意、代码签名、CSP/IPC 收紧、专利/法务结论。
- 不新增依赖、不新增 Tauri ACL/fs 插件、不允许 UI 传任意路径。
- 不改录制/编码/同步/恢复/设置/命名/安装包资源内容（FFmpeg 5 文档已存在）。
- 不把“候选/草案”材料标为正式法律文本。

## 3. 用户故事

- US-1：我想在应用内看到 Screen_rec 版本和系统/FFmpeg 构建信息。
- US-2：我想查看随包 FFmpeg 的 LGPL/GPL 文本、源码说明、构建脚本和构建信息。
- US-3：我不想应用读取任意本地文件；只能查看开发者固定的许可证文档。

## 4. 验收标准（AC）

- **AC-1（入口/元数据）**：新增「关于」导航页；显示应用版本（与 bundle/Cargo 一致）、Windows 构建、FFmpeg 构建标识；不显示“正式签名/专利已清权/法律合规”等未证实状态。
- **AC-2（固定文档目录）**：列出 5 个已随包文档：FFmpeg LICENSE、GPLv3、source offer、build info、build script；每项显示标题/状态/大小。未知 ID、空 ID、路径分隔符、绝对路径、`..` 一律拒绝。
- **AC-3（只读查看）**：点击文档按需加载并展示纯文本；使用 `textContent`/转义，不用 `innerHTML`；缺失/过大/非 UTF-8 返回通用错误，不回显绝对路径；最大 4 MiB。
- **AC-4（打包一致性）**：安装包内 `licenses/` 5 文件与仓库源文件 SHA-256 一致；About 不读取 `docs/compliance` 整目录，不读取用户输出目录。
- **AC-5（范围/回归）**：不新增依赖/ACL；现有录制/截图/合并/设置/恢复行为不变；现有 UI runner 通过；新增 About runner；不宣称 EULA/隐私/第三方 notices 已完成。

## 5. 约束

- Windows 10 2004+/11 x64；无网络；无新依赖。
- 资源解析优先级：Tauri `resource_dir` → exe 同目录 `licenses/` → 开发回退 `CARGO_MANIFEST_DIR/resources/licenses`。
- 只允许固定文档 ID；UI 不传路径；canonicalize 后必须位于允许根目录。
- 保持 release GUI 无控制台；不新增日志文件。
- EULA/隐私/第三方 notices 仍是草案/候选，不在本切片展示为最终条款。

## 6. 分期

| 批次 | 内容 | 验收 |
|---|---|---|
| M19d1 | About 导航/版本/系统/FFmpeg 构建 + 5 个已随包文档查看器 | Rust 单测 + About UI runner + 打包哈希 |
| M19d2 | third-party notices/licenses/SBOM/EULA/隐私草案（法律审阅后） | 另立规格 |

## 7. 开放问题

- 是否在 About 显示“内部测试构建/未签名”？本切片建议显示，避免误认为正式发行。
- 是否显示项目自身许可证？当前无根 LICENSE，不能声称 MIT/Apache；本切片不显示。
- M19d2 是否分页/懒加载 1.68 MB licenses 文本？本切片不涉及。

## 8. 批准

待 plan review 与用户批准 AC-1..AC-5 后进入实现。
