# M12-D 区域框选端到端整改报告（第三轮 REJECTED：P1 真实交互 + P2 失败提示 + fmt）

- 日期：2026-10-17
- 范围：只处理第三轮验收指出的三项——① 区域框选真实交互未验证（唯一实质 P1）；② 失败提示丢失（P2，`region.rs:224–227` 成功失败都关窗）；③ `cargo fmt --all -- --check` 失败。
- 约束遵守：未改任何既有测试断言/忽略标记；未改 `specs/`、`docs/spikes/`；未 `git` 提交/暂存；**未重建安装包**、未跑 `sync/soak` 长测（留给编排者最终一轮）；未新增外部依赖（`Cargo.toml` 无变化）。
- 证据口径：本文「实测」均为本机真实命令/真实 GUI 操作输出；「推断」已显式标注。全部截图与自动汇总落盘于 `docs/milestones/m12-d-evidence/`。

## 1. 结论

| 项 | 状态 | 关键证据 |
|---|---|---|
| P1 区域框选真实交互 | **完成（真实 GUI + 真实输入，端到端 PASS）** | 覆盖层绘制遮罩（桌面均值 116.59 → 50.35，ratio=0.432）；拖拽 → PNG 落盘 900×600；成品与覆盖前底层**逐像素完全一致**（meanDiff=0、maxDiff=0、0/540000 像素差异） |
| P1 根因 | **已定位并修复：sync 命令重入 WebView2 死锁（非环境问题）** | 修复前覆盖层只有 `WRY_WEBVIEW` + `Chrome_WidgetWin_0(0×0,hidden)`，`build()` 永不返回；改为 `async` 后 `Chrome_WidgetWin_1`/`RootView`/D3D 子窗口全部可见并可绘制（§2） |
| P2 失败提示 | **完成** | 失败路径覆盖层保持可见 + 顶部错误提示条渲染（红色提示像素 2843），输出目录未创建、无文件（§5.c） |
| `cargo fmt --all -- --check` | **完成（exit 0）** | §6.2；仅格式变化 |

## 2. P1 根因：sync 命令在 WebView2 IPC 回调里同步创建第二个 WebView2 → 重入死锁

### 2.1 修复前实测（本机，release 与 debug 均复现）

点击「截图 → 框选区域截图」（真实鼠标）后：

- `EnumWindows`：`class=Tauri Window title=region-overlay rect=0,0,5280,3002 WS_EX_TOPMOST`，**窗口确实被创建**；
- 子窗口仅两级：`WRY_WEBVIEW (visible, 5280×3002)` → `Chrome_WidgetWin_0 (vis=False, 0×0)`；
- UIA 子树仅 `Window → Pane(WRY_WEBVIEW)`，**没有** `Chrome_WidgetWin_1` / `RootView` / `Document`；
- 桌面捕获看不到 55% 遮罩（覆盖层完全透明），拖拽不产生 PNG；
- 前端 `busy=true` 卡死（`invoke` 既不 resolve 也不 reject，因此连错误 toast 都没有）。

临时诊断（已在最终代码中移除）在 `open_region_overlay` 内写文件，输出**只有** `creating overlay window` 一行，没有 `build ok` / `build err`：

```
creating overlay window
```

即 `WebviewWindowBuilder::build()` **永不返回**。

### 2.2 机制（代码事实 + 推断）

- Tauri 2 的 **sync** 命令直接在 WebView2 IPC 回调线程（主线程）执行；
- `WebviewWindowBuilder::build()` 在 Windows 上同步创建窗口 **并同步创建 WebView2 控制器**（wry `create_controller` → `wait_with_pump` 等在**同一线程**泵消息）；
- 在 WebView2 自己的事件回调里再同步创建第二个 WebView2 控制器属于重入：控制器的完成回调无法在当前已阻塞的调用栈上派发，`wait_with_pump` 永久等待 → 窗口已创建但 webview 永不初始化，命令永不返回。
- 这与「窗口在、无内容、不绘制、invoke 不返回」的全部现象一致（§2.1）。

### 2.3 排除项（实测实验，均已还原）

| 实验 | 结果 | 结论 |
|---|---|---|
| `.transparent(false)`（不透明覆盖层） | 仍然无 WebView2 子树、`build()` 不返回 | 与透明窗口/合成无关 |
| `.inner_size(800, 600)`（小尺寸） | 仍然无 WebView2 子树、`build()` 不返回 | 与窗口尺寸/虚拟桌面大小无关 |
| 改为 `async fn open_region_overlay` | `build()` 立即返回；子窗口 `Chrome_WidgetWin_0/Chrome_WidgetWin_1/Chrome_RenderWidgetHostHWND/Intermediate D3D Window` 全部 `visible=True` | **根因=调用上下文重入**，修复有效 |

结论：`docs/milestones/m12-a-report.md` §2.4 把「覆盖层不绘制」归因为“本会话环境”**不成立**——这是应用缺陷（旧代码在 debug/release 均复现，修复后均正常）。

### 2.4 修复（`src-tauri/src/region.rs:105–151`）

```rust
// 必须是 async 命令：sync 命令在 WebView2 IPC 回调（主线程）里执行，此时再同步创建
// 第二个 WebView2 控制器会重入死锁——窗口已出现但 webview 永不初始化（无
// Chrome_WidgetWin_1/Document、不绘制遮罩），invoke 也永不返回（前端 busy 卡死）。
pub async fn open_region_overlay(app: AppHandle) -> Result<(), String> { ... }
```

其余窗口参数（无边框/置顶/透明/跳过任务栏/物理像素 set_position+set_size）**未改**；命令注册（`lib.rs:184`）未改。覆盖层尺寸实测始终等于**当次显示器并集**（本轮会话显示器拓扑变更为 3840×2160 + 1440×2560@(3840,442) → 并集 5280×3002，`GetWindowRect` 与并集完全一致）。

## 3. P2：失败时覆盖层保持可见并显示错误（`region.rs:247–260`）

| 结果 | 行为 |
|---|---|
| 成功 | 关闭覆盖层（原行为保持） |
| 失败 | `window.show()` + `set_focus()` → 覆盖层恢复可见；前端 `ui/region.js` 既有 `catch → showError()` 现在**真实可见**（红色提示条），用户可重拖重试或 Esc 取消；不残留隐藏窗口 |

`ui/region.js` 仅同步注释（说明成功/失败语义；无行为改动）。Esc 取消路径未改（仍直接 `close_region_overlay`，不发截图请求）。

## 4. 端到端脚本（可重复）

| 文件 | 作用 |
|---|---|
| `tools/e2e/RegionE2E.psm1` | 基座：per-monitor-v2 DPI 感知、窗口枚举/子窗口、桌面 Bitmap 截图、像素统计、真实鼠标/键盘注入、按颜色定位「框选区域截图」按钮 |
| `tools/e2e/New-SolidTarget.ps1` | 底层目标窗口：已知纯色（#00FF00）+ 四角/中心色块 + MaxTop，写回实际物理矩形 |
| `tools/e2e/region-e2e.ps1` | 三个场景（a/b/c）的编排、断言与证据落盘（`-Scenario all|draw|esc|failure`） |

运行方式（本轮实际执行，release 构建）：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/e2e/region-e2e.ps1 `
  -AppExe target\release\screen-rec.exe `
  -EvidenceDir docs\milestones\m12-d-evidence
```

真实入口（先查明并记录，非热键/托盘）：主窗口 → 左侧「截图」导航 → 页面底部「区域」面板 →「框选区域截图」按钮（`ui/index.html:92`，`ui/app.js:799` → `api.openRegionOverlay()` → `invoke('open_region_overlay')`）。脚本用真实鼠标点击这条路径（含主内容区滚动），不调用任何 webview 内部 API。

输入注入说明：脚本**优先 SendInput**；本会话实测 `SendInput` 返回 1 但光标完全不动（`GetCursorPos` 不变、相对/绝对移动均无效），因此自检后回退到 Win32 `mouse_event`/`keybd_event`（同为 OS 级真实输入，非脚本注入页面事件）。回退是自动的，正常桌面会话会使用 SendInput。

## 5. 场景结果（release，2026-10-17 21:03，全部 PASS）

自动汇总：`docs/milestones/m12-d-evidence/summary.md`。

### a) 框选 → PNG 落盘（底层已知图案）

底层目标窗口物理矩形 `600,300 900×600`；拖拽同样坐标（物理像素）。

| 断言 | 结果 | 实测 |
|---|---|---|
| a0 覆盖层 WebView2 初始化 | PASS | children=`WRY_WEBVIEW[True], Chrome_WidgetWin_0[True], Chrome_WidgetWin_1[True], Chrome_RenderWidgetHostHWND[True], Intermediate D3D Window[True]`；overlay=5280×3002（= 显示器并集） |
| a1 遮罩真实绘制 | PASS | 桌面亮度均值 **116.587 → 50.347（ratio=0.432）**，与 `rgba(5,8,14,0.55)` 理论压暗一致 |
| a2 拖拽生成 PNG | PASS | `C:\Users\elvis\Videos\Screen_rec\shot_region_1792242209017_0000.png`，拖拽→落盘 1880ms，覆盖层随后关闭 |
| a3 物理尺寸 = 拖拽矩形 | PASS | PNG IHDR **900×600**（期望 900×600；拖拽矩形按 3 的倍数选取，150% 下 CSS 整数无取整误差） |
| a4 无遮罩/边框污染 | PASS | 与覆盖前底层截图逐像素比对：**meanDiff=0、maxDiff=0、changedPixels=0/540000**（任何遮罩/边框像素都会使该值显著 >0） |

证据：`m12-d-evidence/baseline.png`（覆盖前底层）、`overlay-mask.png`（遮罩可见）、`region-shot.png`（成品 PNG 副本）。

### b) Esc 取消

| 断言 | 结果 | 实测 |
|---|---|---|
| b1 Esc 关闭、无新文件、无残留 | PASS | `gone=True`、`newPng=0`、`appAlive=True`、`residualOverlay=0` |

证据：`m12-d-evidence/after-esc.png`。

### c) 失败路径（输出目录不可写）

设置 `output_dir = %TEMP%\screen-rec-e2e\blocker.file\sub`（`blocker.file` 是普通文件），重启应用后框选：

| 断言 | 结果 | 实测 |
|---|---|---|
| c1 覆盖层保持可见 + 显示错误 + 无静默失败 | PASS | `overlayVisible=True`、错误提示红色像素 **2843**、`outputDirCreated=False`；覆盖层顶部提示条文本实测为 `截图失败：E_OUTPUT_UNAVAILABLE: 创建截图输出目录失败（…\blocker.file\sub）：當檔案已存在時，無法建立該檔案。(os error 5)` |

证据：`m12-d-evidence/failure-output-unavailable.png`。测试后已按备份恢复用户 `settings.json`。

## 6. 验证命令（实测）

### 6.1 构建

```
cargo build --release -p screen-rec  → Finished `release` profile in 23.85s（target/release/screen-rec.exe 2026-10-17 21:01:42）
```

### 6.2 格式化

```
cargo fmt --all
cargo fmt --all -- --check  → exit 0（本轮修复前为失败）
```

`cargo fmt --all` 重排了 57 个 `.rs` 文件：其中 `src-tauri/src/region.rs` 含本轮实质改动，其余 56 个（`crates/**` 与 `src-tauri/src/{commands,settings}.rs`、`tests/sr-itests/**`）**仅格式变化、无语义改动**。

### 6.3 测试（短测，未跑 ignored 集）

```
cargo test --workspace --no-fail-fast
→ 全部 test result: ok；0 failed；23 passed（screen_rec_lib，含 5 个 region 测试）
  其余 crate 全绿；42 ignored（L2/L3 门，按约束未运行）
```

## 7. 未决 / 风险（回交编排者）

1. **截图记录不追加**：区域截图成品不进入主窗口「本次会话截图记录」（M11-B 报告「未决」#2 的既有缺口；本轮未扩展事件契约）。本轮生成 1 个 `shot_region_*.png` 留在输出目录（作为证据链，历史列表不显示）。
2. **脚本的 UI 定位假设**：导航项按 CSS `(45,139)` + `clientWidth/1024` 缩放定位；主窗口布局若大改需同步该常量。目标区域默认 `600,300 900×600`（可用 `-DragRect` 覆盖）。
3. **覆盖层提示条位置**：提示条按窗口水平居中；若多显示器并集存在大“空洞”，中心可能落在无显示区域（本机两屏实测居中可见，未复现）。属既有视觉问题，未在范围内改。
4. **显示器拓扑在会话中变化**：实测先后出现 4224×2402 与 5280×3002 两种并集；覆盖层尺寸始终等于创建时的并集（已核对 `GetWindowRect`）。脚本按当次并集测量，不写死分辨率。
5. **未运行项**：L2/L3 ignored（42 项）未跑；未重建安装包、未做安装态验证；未跑任何 soak/sync 长测（按任务约束留给最终一轮）。

## 8. 合规声明

- 未修改任何既有测试的断言或忽略标记；未修改 `specs/`、`docs/spikes/`；未执行任何 `git` 操作。
- 无新增外部依赖；`src-tauri/Cargo.toml` 本轮无变化（仅格式化 Rust 源码）。
- 生产代码无调试输出/临时标记：已检索 `TEMP-DIAG`、`log_diag`、`append_diag`、`SetWindowPos` 均为空；临时诊断文件已删除。
- 测试产生的副作用已清理：`settings.json` 已恢复；输出目录仅保留本轮最后一张成品 PNG（见 §7.1）。

## 9. 证据索引

| 路径 | 内容 |
|---|---|
| `docs/milestones/m12-d-evidence/summary.md` | 脚本自动生成的逐项断言结果（本轮 8/8 PASS） |
| `docs/milestones/m12-d-evidence/baseline.png` | 覆盖层出现前的底层目标截图（900×600） |
| `docs/milestones/m12-d-evidence/overlay-mask.png` | 覆盖层绘制中的桌面（遮罩可见，5280×3002） |
| `docs/milestones/m12-d-evidence/region-shot.png` | 拖拽产出的 PNG 副本（900×600，与 baseline 逐像素一致） |
| `docs/milestones/m12-d-evidence/after-esc.png` | Esc 之后（无覆盖层、无残留） |
| `docs/milestones/m12-d-evidence/failure-output-unavailable.png` | 失败路径：覆盖层保持可见 + 顶部红色错误提示条 |
| `tools/e2e/RegionE2E.psm1`、`tools/e2e/New-SolidTarget.ps1`、`tools/e2e/region-e2e.ps1` | 可重复端到端脚本（PowerShell + 真实输入） |
