# M12-A 验收整改报告（第二轮退回：P1-A / P1-B）

- 日期：2026-10-17
- 范围：仅整改第二轮验收退回的 2 项 P1（区域截图捕获覆盖层；`safe_commit` 原子竞态）。
- 约束遵守：只跑短测（L1/定向 + `cargo test --workspace` 非 ignored 集）；**未执行任何 L2 长测**（`#[ignore]` 42 项全部保持 ignored，留待统一串行复跑）；未改既有测试断言；未 `git` 提交/暂存；未改 `specs/`、`docs/spikes/`。
- 证据口径：本文「实测」均为本机实际命令输出（日志见 §7）；「推断」为代码推理，已标识；GUI 项按任务允许标为 **L3 人工**，并附本次自动尝试的真实结果与阻塞原因。

## 1. 结论

| 项 | 状态 | 关键证据 |
|---|---|---|
| P1-A 区域截图捕获覆盖层 | **代码完成 + L1 短测通过；GUI 像素举证 L3 人工（本机会话阻塞，原因见 §2.4）** | 新命令 `take_region_screenshot`（`src-tauri/src/region.rs:195–228`）：先 `hide()`（207）→ `DwmFlush`+≥150ms（153/183–187）→ 截图（216）→ 关闭（226）；前端改为调用该命令（`ui/region.js:34,95`），Esc 路径不变 |
| P1-B `safe_commit` 原子竞态 | **完成 + 确定性红→绿** | `MoveFileExW` 不带 `MOVEFILE_REPLACE_EXISTING`（`crates/sr-pipeline/src/naming.rs:139–164`）；新回归测试先红（旧 `std::fs::rename` 下 FAILED）后绿（3 passed） |

## 2. P1-A 区域截图会捕获框选覆盖层

### 2.1 整改前证据（任务给定，代码已核对）

| 位置 | 问题 |
|---|---|
| `ui/region.js:87–89`（整改前） | 先 `await takeScreenshot(phys)`，成功后才 `await close()`；截图瞬间覆盖层仍可见 |
| `src-tauri/src/region.rs:109–116`（整改前） | 覆盖层 `always_on_top(true)` + `visible(true)`，创建后按物理像素覆盖整个虚拟桌面 |
| `src-tauri/src/commands.rs:827–840` | `take_screenshot` 直接调用 `screenshot::take_screenshot`，不感知 overlay |
| `crates/sr-shot/src/capture.rs:193–195` | `capture_monitor` 从 WGC 抓实时桌面帧（覆盖层像素会被合成进去） |

### 2.2 修法（仅在范围内）

| 文件:行 | 改动 |
|---|---|
| `src-tauri/src/region.rs:150–228` | 新增 `take_region_screenshot(rect)`：`region_shot_target` 校验/映射（167–177）→ 取 `region-overlay` 窗口 → 阻塞线程内 `hide()`（207）→ `wait_for_composition()`（183–187：`DwmFlush` → sleep `REGION_COMPOSE_SETTLE_MS=150`（153）→ `DwmFlush`）→ 创建输出目录 → `sr-shot` 区域截图（216，物理像素）→ 命令返回后 `window.close()`（226）；**成功/失败都关闭覆盖层**，不残留隐藏窗口 |
| `src-tauri/src/region.rs:155–164` | 回执 `RegionScreenshotOut { path, width_px, height_px }`（路径 + 物理像素元数据） |
| `src-tauri/src/lib.rs:187` | 注册 `region::take_region_screenshot` |
| `ui/region.js:31–36` | `backend.takeRegionScreenshot(rect) → invoke('take_region_screenshot', { rect })` |
| `ui/region.js:43–51` | 浏览器 mock 同步改为 `take_region_screenshot` 形状（并模拟截图后关窗） |
| `ui/region.js:93–99` | 确认路径只调用新命令；不再「先截图后关窗」，成功后不再重复 `close` |
| `ui/region.js:12,77–79,127–130` | **Esc 取消路径保持不变**：仍直接 `close_region_overlay()`，不发截图请求 |
| `src-tauri/src/commands.rs:991,996,1020,1167` | `invalid`/`blocking`/`resolve_output_dir`/`shot_error_string` 改为 `pub(crate)` 供 region 命令复用（无行为变化） |
| `src-tauri/Cargo.toml:26–30` | `windows` 增加 `Win32_Graphics_Dwm` feature（`DwmFlush`；非新增外部包） |

顺序语义（代码事实）：`hide` 提交后阻塞线程立即进入 `DwmFlush` + 150ms 等待；Tauri 在事件循环线程处理 `hide`，等待结束前的第二次 `DwmFlush` 会等待下一帧合成提交，之后才调用 `screenshot::take_screenshot`（`region.rs:207 → 209 → 216`）。因此截图时覆盖层已从合成表面移除。

### 2.3 L1 短测（实测）

```
cargo test -p screen-rec --lib
→ 23 passed; 0 failed; 0 ignored（0.16s）
  含新增：
  - region::tests::region_shot_target_maps_physical_rect_and_rejects_empty  ok（region.rs:248）
  - region::tests::compose_settle_floor_is_at_least_150ms                   ok（region.rs:268）
  - region::tests::wait_for_composition_actually_waits_for_the_floor        ok（region.rs:277，行为级 ≥150ms）
```

```
node --check ui/region.js
→ exit 0（语法通过）
cargo check -p screen-rec
→ exit 0（本命令无 warning；workspace 运行时仅 sr-itests/probe.rs 既有 unused import 警告）
```

日志：`docs/milestones/m12-verify-src-tauri-lib.log`、`m12-verify-workspace.log`。

### 2.4 真实 GUI 举证

#### 本次自动尝试（实际执行，结果为阻塞）

1. `cargo build -p screen-rec` 成功；启动 `target/debug/screen-rec.exe`（真实 GUI，UIA 可见完整界面）。
2. 通过 UI Automation 真实点击「截图」导航与 `btn-region-shot`（`ui/app.js:799` 的入口），后端确实创建了覆盖层窗口：
   - `EnumWindows`：`class=Tauri Window rect=0,0,5280,3002 title=region-overlay`，`WS_EX_TOPMOST`；
   - 命中测试：框选坐标处返回 `WRY_WEBVIEW`（覆盖层在最上层）。
3. **阻塞现象（本会话环境）**：覆盖层 WebView2 表面未绘制——UIA 子树仅有 `Window → Pane(WRY_WEBVIEW)`（无 Document）；`Chrome_WidgetWin_0` 子窗口 `vis=False rect=0,0,0,0`；`gdigrab` 与 `ddagrab` 两种桌面捕获均看不到 55% 遮罩（纯色窗口像素仍为 `(0,255,0)`，其它区域亮度未变化）。因此覆盖层收不到页面事件：`SendInput` 拖拽后未生成 PNG、输出目录也未创建。
4. 结论：本机无法完成 P1-A 的自动 GUI 像素举证。本轮未改覆盖层创建与页面渲染（推断：该现象不由本轮改动引入；M11-B 报告「未决」#3 亦将透明窗口列为假设项）。**按要求标注 L3 人工**，需在覆盖层可正常绘制的目标机上执行下述步骤。

（已复核：真实桌面捕获 + 像素统计链路可用——对已知纯色区域 `crop=1200:700:2100:200` 提取后统计 `840000` 像素、`non_green=0`，说明 §2.4.2 的命令本身可执行。）

#### 2.4.1 L3 人工步骤

1. 准备已知纯色窗口：例如无边框窗口，背景 `#00FF00`（RGB 0,255,0），置于屏幕固定物理位置（记录其 x/y/w/h），确保框选区域完全落在纯色内部且无其它窗口遮挡。
2. 启动 `target/debug/screen-rec.exe`（或安装版），主窗口 →「截图」→「框选区域截图」。
3. 在纯色区域内拖选一个已知物理矩形（如 1200×700），松开鼠标；覆盖层应关闭，输出目录（设置中的 `output_dir`）出现 `shot_region_*.png`。
4. 记录返回/提示的物理尺寸，执行 2.4.2 的尺寸与像素判据。
5. Esc 路径：再次打开覆盖层 → 按 `Esc` → 覆盖层关闭；用 `EnumWindows`（或 Spy++）确认无 `region-overlay` 残留窗口，且输出目录无新增 PNG。

#### 2.4.2 像素判据（锁定 ffmpeg 实测可用）

```powershell
$ff = "tools\ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin\ffmpeg.exe"
# ① 尺寸必须 = 框选物理尺寸（与回执 width_px/height_px 一致）
& $ff -v error -i shot.png -vf "crop=1200:700:2100:200,format=rgb24" -f rawvideo -y crop.rgb
$b = [IO.File]::ReadAllBytes("crop.rgb")
$bad = 0
for ($i = 0; $i -lt $b.Length; $i += 3) {
  if ([math]::Abs($b[$i]) -gt 2 -or [math]::Abs($b[$i+1]-255) -gt 2 -or [math]::Abs($b[$i+2]) -gt 2) { $bad++ }
}
"pixels=$($b.Length/3) non_green=$bad"
```

- **通过判据**：`non_green = 0`（容差 ±2）；即成品中不存在遮罩合成色（≈`(2,115,6)`/`(3,123,7)`）与选区边框色（`#6b90ff` = `(107,144,255)`）。
- 若框选区域不是全纯色，改为：区域内每个像素与背景色的差 ≤2，且统计中**不存在**上述遮罩/边框颜色（任一出现即判失败）。
- 尺寸判据：PNG IHDR 宽高 = 拖选物理尺寸（前端提示值 = 回执 `width_px/height_px`）。

## 3. P1-B `safe_commit` 原子竞态

### 3.1 整改前证据（已核对）

`crates/sr-pipeline/src/naming.rs:120–124`（整改前）：`exists()` 预检 → `verify()` → `std::fs::rename()`。`rename` 在 Windows 上带替换语义（`MOVEFILE_REPLACE_EXISTING`），预检与提交之间被外部创建同名文件时会被静默覆盖。

### 3.2 修法

| 文件:行 | 改动 |
|---|---|
| `crates/sr-pipeline/src/naming.rs:115–137` | `safe_commit` 保留 `exists()` 快速拒绝（保证既有契约「已存在时 verify 不被调用」）；verify 通过后改走 `rename_no_replace`；失败按 `ErrorKind::AlreadyExists → FinalExists{final_path}`，其余 `Io` |
| `crates/sr-pipeline/src/naming.rs:139–164` | `rename_no_replace`：`MoveFileExW(..., MOVE_FILE_FLAGS(0))`（**不带** `MOVEFILE_REPLACE_EXISTING`），同卷原子「禁止替换」；Win32 错误码还原为 `io::Error` 以复用 std 的 `ErrorKind` 映射（183/80 → `AlreadyExists`，跨卷 17 → `Io`） |
| `crates/sr-pipeline/src/naming.rs:103–113` | `CommitError::FinalExists` 文档补充竞态来源（目标字节不变） |
| `crates/sr-pipeline/Cargo.toml:24` | `windows` 增加 `Win32_Storage_FileSystem`（`MoveFileExW`；非新增外部包） |
| `crates/sr-pipeline/tests/safe_commit.rs:160–199` | 新增确定性回归测试 `unit_safe_commit_rejects_target_created_after_verify`：**在 verify 闭包内创建目标文件**（确定性复现 verify→提交之间的外部创建窗口）→ 断言提交失败、目标字节保持 `EXTERNAL-RACE`、临时文件保留、移除冲突后可重试 |

既有测试断言未改动（`unit_session_tmp_dir_contract` / `unit_safe_commit_no_overwrite_and_verify` 原样）。

### 3.3 红→绿证据（实测）

**红（旧实现验证）**：临时把提交调用换回 `std::fs::rename`（仅验证用，已按字节哈希还原）：

```
cargo test -p sr-pipeline --test safe_commit unit_safe_commit_rejects_target_created_after_verify -- --nocapture
→ FAILED；panicked at crates\sr-pipeline\tests\safe_commit.rs:177:
  提交阶段目标已存在时必须失败（原子禁止替换）: ()
→ test result: FAILED. 0 passed; 1 failed
```

日志：`docs/milestones/m12-red-probe-old-rename.log`。说明：旧实现下 `rename` 成功替换了外部目标，测试在 `expect_err` 处失败——回归测试确实捕获该竞态，而非空转。

**绿（最终实现）**：

```
cargo test -p sr-pipeline --test safe_commit -- --nocapture
→ 3 passed; 0 failed（含跨卷 Io 分支实际执行：日志无「单卷环境」提示）
```

还原校验：`naming.rs` SHA-256 = `C5C50D3A1B012750C2457B1CC0E210B674FC6A08F4D0049898397E368D45ED92`（临时验证前后一致，无临时标记残留）。

说明（跨 lane 透明）：红探针期间 `naming.rs` 有约 2 分钟处于旧实现状态；并行 M12-B lane 的 workspace 运行曾因此观察到一次瞬时失败（见 `m12-b-report.md` §4）。探针已按字节哈希还原，本文 §3.4 的最终 workspace 日志为还原后重跑结果（52 passed / 0 failed）。

日志：`docs/milestones/m12-verify-safe-commit.log`。

### 3.4 工作区短测（实测）

```
cargo test --workspace --no-fail-fast
→ CARGO_EXIT=0；50 个 test-result 段（含 doc-tests）：52 passed；0 failed；42 ignored（全部为 L2/L3 门，按约束未运行）
```

关键结果：`screen_rec_lib` 23 passed、`sr-pipeline::safe_commit` 3 passed、`sr_pipeline` 其余 7 passed、`sr_audio` 10 passed、`sr_core` 8 passed、`sr_capture` 1 passed。日志：`docs/milestones/m12-verify-workspace.log`。

## 4. 变更文件清单

| 文件 | 性质 |
|---|---|
| `src-tauri/src/region.rs` | 生产：新命令、合成等待、回执、3 个 L1 测试 |
| `src-tauri/src/lib.rs` | 生产：命令注册 |
| `src-tauri/src/commands.rs` | 生产：4 个辅助函数 `pub(crate)`（无行为变化） |
| `src-tauri/Cargo.toml` | 依赖 feature：`Win32_Graphics_Dwm` |
| `ui/region.js` | 生产：确认路径改调 `take_region_screenshot`；Esc 不变 |
| `crates/sr-pipeline/src/naming.rs` | 生产：原子禁止替换提交 |
| `crates/sr-pipeline/Cargo.toml` | 依赖 feature：`Win32_Storage_FileSystem` |
| `crates/sr-pipeline/tests/safe_commit.rs` | 测试：新增 P1-B 回归（既有断言未改） |
| `docs/milestones/m12-a-report.md` | 本文 |
| `docs/milestones/m12-verify-*.log` / `m12-red-probe-old-rename.log` | 实测日志 |

## 5. 未决 / 风险（回交编排者）

1. **P1-A GUI 像素举证未在本机完成（L3 人工）**：本会话覆盖层 WebView2 表面未绘制（§2.4），无法自动拖选。需在覆盖层正常渲染的目标机执行 §2.4.1/§2.4.2；若目标机同样复现「无遮罩/无 Document」，应作为独立缺陷（覆盖层渲染，非本轮 P1-A 改动）另行处理。
2. **失败可见性**：`take_region_screenshot` 在失败时也关闭覆盖层（满足「不得残留隐藏窗口」），因此覆盖层内的错误提示不可见；主窗口没有区域截图完成/失败回传通道（M11-B 报告「未决」#2 既有缺口）。本轮未扩展事件契约。
3. **合成等待上限**：固定 ≥150ms（任务判据）。若主事件循环长时间阻塞，`hide` 可能尚未生效；当前命令在独立阻塞线程执行，主循环空闲，正常路径无此风险。未引入轮询重试（避免范围扩张）。
4. **`safe_commit` 跨卷**：仍要求临时文件与成品同卷（REQ-24）；跨卷返回 `Io` 且临时文件保留（实测分支执行）。
5. **未运行项**：L2/L3 ignored 用例（42 项）未运行，按任务约束留待统一串行复跑；未做 release 打包/安装态验证。

## 6. 合规声明

- 未修改任何既有测试的断言或忽略标记；仅新增测试。
- 未修改 `specs/`、`docs/spikes/`；未执行任何 `git` 操作（仓库当前无提交，全部文件为工作区状态）。
- 未引入新外部依赖；两处 `Cargo.toml` 仅启用既有 `windows 0.62` 的 feature。
- 生产代码无调试输出、无注释掉的死代码、无临时标记（已全局检索 `TEMP-RED-PROBE` 无结果）。

## 7. 证据索引（本机实际日志）

| 日志 | 内容 |
|---|---|
| `docs/milestones/m12-red-probe-old-rename.log` | 旧 `std::fs::rename` 下新回归测试 FAILED（红证据，exit 101） |
| `docs/milestones/m12-verify-safe-commit.log` | 最终实现 `safe_commit` 3 passed |
| `docs/milestones/m12-verify-src-tauri-lib.log` | `screen_rec_lib` 23 passed（含 3 个 region 测试） |
| `docs/milestones/m12-verify-workspace.log` | 工作区短测 52 passed / 0 failed / 42 ignored |
| GUI 尝试原始截图 | `%TEMP%\m12gui\desktop-overlay.png`、`desktop-dd.png`（会话临时目录，未入库） |
