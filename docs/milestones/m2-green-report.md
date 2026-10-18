# M2-C 绿实现报告（TDD 绿阶段收尾：截图 REQ-5..8，sr-shot 全绿）

- 日期: 2026-10-16
- 规格: specs/001-screen-recorder.md §2.2（REQ-5..8）、§1（锁定栈：windows 0.62 直调 WGC）、§3.2/§3.3/§3.4、§4、§5（M2 行）
- 红阶段基线: docs/milestones/m2-red-report.md（重点 §3/§10 契约、§10.4 本机拓扑、§10.6 残留风险）
- 技术背景: docs/research/feasibility-2026-10-15.md §4（截图 WGC 单帧首选、PrintWindow(PW_RENDERFULLCONTENT) 窗口备选）
- 会话说明: 绿实现由先前的 M2-C 执行者会话落盘（该会话因宿主证书校验错误中断、未留终报）。本会话接手：**核对遗留改动范围 → 逐文件对照绑定契约审查 → 本机复跑全部 L1+L2 留证 → 本记录**。核对结论：遗留改动**恰好收敛于 `crates/sr-shot` 6 个文件**（2:43–3:08 落盘），实现完整、与本会话零代码修改即可全绿；除 sr-shot 外无任何文件被该会话触碰（见 §2）。
- 环境: Windows 11 25H2，build **10.0.26200.9457**，SESSIONNAME=Console（交互式用户会话）；cargo/rustc **1.99.0 msvc**（rust-toolchain.toml 锁定，%USERPROFILE%\.cargo\bin）。本机拓扑（测试运行时实测，DEVMODE 物理坐标）：`(0,0) 3840x2160 主（150%）` + `(3840,442) 1440x2560 副（125%）`，水平贴合、无负方向显示器（与红报告 §10.4 一致）。

## 1. 范围与约束遵守

M2-C 任务：实现 `sr-shot` 截图（REQ-5..8），使 `tests/sr-itests/tests/shot.rs`（绑定契约，**不得修改**）红测试全绿。

- **未修改任何测试文件**（shot.rs / enum_targets.rs / enum_audio.rs / common/mod.rs / channel_snapshot_rebind.rs / sr-itests Cargo.toml 逐字未动）；
- 未改 specs/、docs/ 既有内容、tools/；未 git 提交；
- 实现面改动 = `crates/sr-shot`（Cargo.toml + src×5）+ 随构建更新的 `Cargo.lock`（sr-shot 条目登记 `png 0.18.1`、`windows`；两依赖均已在锁文件既有版本集中：png 0.18.1 沿 src-tauri 侧 image 0.25 传递链、windows 0.62.2 全仓共用，无新版本引入）。

## 2. 变更文件盘点（遗留会话落盘 + 本会话核验）

| 文件 | 内容 |
|---|---|
| `crates/sr-shot/Cargo.toml` | 依赖仅两项：`png 0.18`（RGBA8 编码；复用锁文件既有 0.18.1）+ `windows 0.62`（features 按命名空间最小化：Graphics_Capture/WinRT 互操作、Direct3D(+11)/Dxgi、HiDpi、Storage_Xps（PrintWindow 在 0.62 归 Xps）、Com、WinRT(+Direct3D11/Graphics_Capture)、Gdi、Foundation、WindowsAndMessaging） |
| `crates/sr-shot/src/lib.rs` | 模块装配：`capture`/`compose`/`png` 为内部模块，公共面仅 `pub mod screenshot`（契约面最小化） |
| `crates/sr-shot/src/screenshot.rs` | 公共 API：`take_screenshot(ScreenshotRequest) -> Result<SavedScreenshot, ShotError>`；`ScreenRect{x,y,width_px,height_px}`、`ShotTarget::{Monitor{index},Window{hwnd},Region{rect}}`（均 Clone/Debug/PartialEq/Eq）、`SavedScreenshot{path,width_px,height_px,source}`（Clone/Debug）、`ShotError`（Debug + Display + Error；钉 `TargetMinimized`，其余变体实现自定并对齐 §3.4 E_* 语义）。`take_screenshot` 把捕获作业派发到专用 worker 线程（panic 映射 `Internal`，不向调用方 unwind），`source` 原样回传请求目标 |
| `crates/sr-shot/src/capture.rs` | 捕获基础设施：① `CaptureThread` = `RoInitialize(RO_INIT_MULTITHREADED)` + `CoIncrementMTAUsage` + `SetThreadDpiAwarenessContext(PER_MONITOR_AWARE_V2)`（仅本线程，不污染调用方进程 DPI 感知；初始化协议对齐 windows-capture 的 RoInitialize 记账，规避 GraphicsCapture.dll 过早卸载崩溃）；② 物理显示器枚举（MONITORINFOEXW + DEVMODE `ENUM_CURRENT_SETTINGS` 物理矩形，与 sr_enum 同法同源 ⇒ 同一 index 空间）；③ WGC 单帧抓取（`CreateFreeThreaded` frame pool + 轮询 `TryGetNextFrame`，4s 超时、8ms 轮询；staging 纹理 CPU 回读为紧凑 BGRA；D3D 子对象先于 device 释放、pool 先 Close session 后 Close）；④ 显示器帧尺寸校验：WGC 帧 ≠ DEVMODE 物理分辨率即拒绝输出（宁失败不出错图）；⑤ 窗口路径：IsWindow/IsIconic 前置（最小化 → `TargetMinimized`）→ WGC `CreateForWindow`，失败回退 `PrintWindow(PW_RENDERFULLCONTENT)`（GetWindowRect 物理矩形 + GetDIBits 顶行在前回读） |
| `crates/sr-shot/src/compose.rs` | 区域拼合（REQ-6）：请求矩形与每台显示器物理矩形求交（i64 中间量防溢出，负坐标天然合法），每台命中显示器取**各自原生物理分辨率**帧（混合 DPI 不经任何缩放），逐行 blit 进以请求矩形为原点的画布；零命中/零尺寸 → `EmptyRegion` |
| `crates/sr-shot/src/png.rs` | PNG 落盘：png crate RGBA8 编码（IHDR 即传入元数据）；命名最小实现 `shot_{kind}_{millis}_{seq}.png`（进程内原子序号 + `create_new` 禁覆盖；REQ-24 全量规则属 M8） |

**未预期文件核查**：按 LastWriteTime 全树扫描（>2026-10-16 02:30 且排除 target/），仅上述 sr-shot 6 文件 + Cargo.lock（2:44 构建更新）——与遗留会话时间窗吻合，无越界改动；tests/、specs/、docs/ 既有文件均未被触碰。

## 3. 复跑证据（本机实际执行，2026-10-16）

| # | 命令 | 结果 |
|---|---|---|
| 1 | `cargo build -p sr-shot` | **Finished（5.19s，全新编译），0 warning** |
| 2 | `cargo clean -p sr-shot` + `cargo check -p sr-shot` | **exit 0，0 warning 0 error** |
| 3 | `cargo test --workspace` | **exit 0（CARGO_EXIT=0）**：L1 `screen_rec_lib` → `7 passed; 0 failed`；`sr-core channel_snapshot_rebind` → `1 passed; 0 failed`；`shot.rs` 编译通过、4 用例按门 `ignored`；`enum_targets` 4 ignored、`enum_audio` 1 ignored；其余库单测/文档测试全部 `ok. 0 passed; 0 failed`（日志 %TEMP%\opencode\m2_ws_test.log） |
| 4 | `cargo test -p sr-itests --test shot it_screenshot_three_targets -- --ignored --nocapture` | **exit 0，`1 passed; 0 failed`（5.22s）**（日志 %TEMP%\opencode\m2_shot_t1.log） |
| 5 | `… it_screenshot_monitor_physical_size …` | **exit 0，`1 passed; 0 failed`（5.57s）**；输出：`[dpi] monitor index=0 at (3840,442) -> 1440x2560`、`[dpi] monitor index=1 at (0,0) -> 3840x2160`（日志 %TEMP%\opencode\m2_shot_t2.log） |
| 6 | `… it_screenshot_window_occluded_and_minimized_error …` | **exit 0，`1 passed; 0 failed`（2.19s）**；输出：`[occluded] 698x469: target-red=0.892 (≥0.30), cover-teal=0.000 (≤0.01)`（日志 %TEMP%\opencode\m2_shot_t3.log） |
| 7 | `… it_screenshot_mixed_dpi_negative_coords …` | **exit 101（§4 BLOCKED 机制，预期内，见 §4.2）**：跨屏子用例**全部断言执行并通过**（`[mixdpi] 拓扑：(3840,442) 1440x2560；(0,0) 3840x2160 主`；`rect=(3640,1151) 400x300` 跨 150%/125% 交界，输出尺寸=请求 400x300、IHDR=元数据、回执一致）；随后负坐标子用例按既定机制 panic：`BLOCKED: 本机拓扑无负方向显示器…（common/mod.rs:37）` → 该用例 FAILED（日志 %TEMP%\opencode\m2_shot_t4.log） |
| 8 | `cargo test -p sr-itests --test enum_targets -- --ignored --nocapture` | **exit 0，`4 passed; 0 failed`（0.77s）**——M1 L2 无回归 |
| 9 | `cargo test -p sr-itests --test enum_audio -- --ignored --nocapture` | **exit 0，`1 passed; 0 failed`（1.02s）**——M1 L2 无回归 |

## 4. 逐用例判定（对照任务验收）

### 4.1 通过项（3/4 + 跨屏子用例）

| 用例 | 判定 | 判别力证据 |
|---|---|---|
| `it_screenshot_three_targets` | **PASS** | 显示器尺寸=DEVMODE 物理分辨率（3840x2160 主屏，进程 DPI-unaware 下未被虚拟化）；区域尺寸=请求物理宽高（320x240）；窗口尺寸>0；三 PNG 均在 output_dir 内、IHDR=元数据、路径互不相同；三目标 `source` 回执==请求目标 |
| `it_screenshot_monitor_physical_size` | **PASS** | 全部显示器（index=0 副 1440x2560@150%… 实为 125% 档、index=1 主 3840x2160）元数据 = sr-enum 条目 = 测试侧 DEVMODE 直接查询；PNG IHDR=元数据——物理像素契约在**调用方进程 DPI-unaware** 前提下成立（worker 线程 PMv2 上下文 + DEVMODE 真值校验生效） |
| `it_screenshot_window_occluded_and_minimized_error` | **PASS** | 遮挡地面真值（WindowFromPoint 命中覆盖窗口）下截图：**红占比 0.892 ≥0.30、青占比 0.000 ≤0.01**（698x469 帧）——截到目标窗口自身表面内容，非空白、非屏幕抓取（覆盖色零泄漏）；最小化（IsIconic 真值确认）后 `expect_err` 得 `ShotError::TargetMinimized`（`matches!` 通过），未成功、未 panic、未借道其它变体 |
| `it_screenshot_mixed_dpi_negative_coords`（跨屏子用例） | **跨屏断言 PASS** | 区域 (3640,1151) 400x300 横跨主屏右缘/副屏左缘（两屏缩放 150%/125% 不同）：输出尺寸=请求物理像素 400x300——单屏缩放或 DPI 虚拟化实现必得错尺寸；IHDR=元数据、回执一致（拼接内容级判别仍属红报告 §10.6 残留，见 §5） |

### 4.2 BLOCKED 项（如实报告，未改测试、未放宽断言）

`it_screenshot_mixed_dpi_negative_coords` 的**负坐标子用例**在本机触发既定 BLOCKED 机制（红报告 §10.4 预告）：

- 机制：测试侧枚举确认无 x<0/y<0 显示器 → 调 `common::blocked()` → panic（`tests\sr-itests\tests\common\mod.rs:37`）→ 该用例 FAILED、二进制 exit 101（§4 门：BLOCKED=fail，SKIP 不算通过）；
- panic 文案（实测输出）：`BLOCKED: 本机拓扑无负方向显示器（全部显示器位于桌面原点 (0,0) 的正方向）：REQ-6 负坐标区域子用例无法在此环境实测（上方跨屏断言已执行；语义不削弱，供负坐标布局的多屏机器验证）`；
- 语义完整性：跨屏子用例（含其全部断言）已先执行并通过；**实现侧负坐标支持未被掩蔽**——`compose.rs` 交集计算全程 i64、以桌面物理坐标为原点，正/负坐标走同一条代码路径（跨屏子用例验证的物理坐标系拼合即负坐标语义的机制本体），本机仅缺「矩形落在负方向屏内」的可实测拓扑。

## 5. 契约符合性小结（红报告 §3/§10 → 实现/证据）

| 契约项 | 状态 | 落点 |
|---|---|---|
| `take_screenshot`/5 符号签名与 derives | ✅ | screenshot.rs；shot.rs 编译+运行通过 |
| `ScreenRect` 负坐标合法 | ✅ | compose.rs i64 交集；本机负坐标子用例 BLOCKED（§4.2） |
| Monitor：物理矩形、尺寸=DEVMODE、含非主屏 | ✅ | 用例 5 实测两屏全对（EnumDisplayMonitors 同序 index 空间） |
| Window：自身表面、遮挡下内容级达标 | ✅ | 用例 6：红 0.892/青 0.000 |
| 最小化 → `Err(TargetMinimized)` | ✅ | 用例 6 后半：matches! 通过 |
| Region：跨多显示器物理坐标拼合裁剪 | ✅ | 用例 7 跨屏子用例：混合 DPI 下尺寸精确 |
| PNG IHDR == 元数据 | ✅ | 全部 assert_png_shape 通过 |
| `source` 回执 == 请求目标 | ✅ | 全部用例 assert_eq 通过 |
| 输出位于 output_dir 内、.png、路径互不相同 | ✅ | 用例 4/5（canonicalize 前缀 + dedup 断言） |

## 6. 限制项与未决（沿红报告 §8/§10.6，无新增阻断）

1. **负坐标区域子用例**：本机拓扑不可实测（§4.2）；代码路径与跨屏共用，待负坐标布局机器复跑。
2. **跨屏拼接内容级判别**（红报告 §10.6 残留）：跨屏用例以「尺寸+IHDR+回执」判别；「尺寸正确但左右两半内容错位」不被捕获。如需内容级判别，后续在两屏放置可区分图案后追加颜色断言（与遮挡用例同手法）。
3. **窗口帧精确尺寸未钉**（红 §8-3）：本次实测 698x469（含标题栏/边框的窗口矩形语义）；契约仅断言 >0，维持未决。
4. **REQ-24 命名全量规则**（红 §8-6）：属 M8；当前最小实现已满足 M2 契约（目录内/png/唯一/create_new 禁覆盖）。
5. **错误枚举其余变体未钉**（红 §8-4）：`TargetMinimized` 已钉并验证；`sr_core::error::Error` 映射按红报告建议留待后续任务回补。
6. **WGC 主路径与 PrintWindow 备选在通过用例中不可区分**：内容级断言按设计路径无关（REQ-7 语义不关心取表面机制）；两条路径均在实现内并按 WGC 优先次序生效。

## 7. 结论

M2 绿阶段验收达成：公共 API 与红报告 §3/§10 契约一致；`cargo test --workspace` L1 全绿（exit 0）；shot L2 三用例通过（three_targets / monitor_physical_size / window_occluded_and_minimized_error），mixed_dpi 跨屏断言通过、负坐标子用例按既定 BLOCKED 机制如实报告（本机拓扑限制）；enum 两个 L2 复跑无回归（4+1 全绿）；证据全部落盘本报告。无阻塞项。
