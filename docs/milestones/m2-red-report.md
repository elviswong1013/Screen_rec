# M2 红测试报告（TDD 红阶段：截图 REQ-5..8 的 M2 任务范围）

- 日期: 2026-10-16
- 规格: specs/001-screen-recorder.md §2.2（REQ-5..8）、§3.2/§3.3（take_screenshot 命令与接口）、§3.4（数据模型）、§4（测试映射）、§5（M2 行）
- 角色: Test Writer（红阶段）。本任务只写测试，未写任何生产实现（crates/sr-shot 仍是 M1-A 空占位骨架）。
- 环境实测: Windows 11 25H2，build **10.0.26200.9457**，SESSIONNAME=Console（交互式用户会话）；rustc/cargo 1.99.0 msvc（rust-toolchain.toml 锁定）。
- 任务给定测试名（与 §4 名称的偏差见 §8 未决 1/2）: `it_screenshot_three_targets` / `it_screenshot_monitor_physical_size` / `it_screenshot_window_occluded`（`manual_region_select` 为 L3，未写）。

## 1. 交付物

| 文件 | 性质 | 内容 |
|---|---|---|
| `tests/sr-itests/tests/shot.rs` | 新增 | L2 ×3：三目标 / 显示器物理尺寸（DPI）/ 被遮挡窗口内容 |
| `tests/sr-itests/Cargo.toml` | 追加 1 项 | `sr-shot = { path = "../../crates/sr-shot" }`（被测 crate，沿用 M1-B 对 sr-core/sr-enum 的做法） |
| `docs/milestones/m2-red-report.md` | 新增 | 本报告（契约 + 红证据） |

未改动：specs/、生产 crate 的 src/（sr-shot 仍为 M1-A 占位）、既有测试文件（enum_targets.rs / enum_audio.rs / common/mod.rs / channel_snapshot_rebind.rs 逐字未动，见 §6.3 验证）、docs 既有内容。未 git 提交。
无新增外部依赖（§5）。仓库外 scratch 工程见 §6.2（不进仓库）。

## 2. REQ → 测试 → 文件映射

| REQ | 测试名 | 层 | 文件 | 断言要点 |
|---|---|---|---|---|
| REQ-5 | `it_screenshot_three_targets` | L2 | tests/sr-itests/tests/shot.rs | 三目标（显示器/自建窗口/程序化区域矩形，无交互）各出一张 PNG：文件存在、扩展名 png、位于 output_dir 内、三路径互不相同；显示器尺寸=DEVMODE 物理分辨率；区域尺寸=请求矩形物理像素宽高；窗口尺寸>0；每张 PNG 实际编码尺寸（IHDR）=元数据；`source` 回执=请求目标 |
| REQ-6（DPI/物理尺寸部分） | `it_screenshot_monitor_physical_size` | L2 | tests/sr-itests/tests/shot.rs | 遍历全部显示器：元数据宽高 = sr-enum 条目 = 测试侧 EnumDisplaySettingsW(ENUM_CURRENT_SETTINGS) DEVMODE 直接查询（按 x/y/w/h 匹配同一台）；PNG IHDR=元数据。测试进程保持默认 DPI-unaware（无清单）：实现若返回被 DPI 虚拟化的尺寸即与 DEVMODE 真值不符而失败。跨屏拼合/负坐标不在本用例（§8 未决 1） |
| REQ-7（被遮挡部分） | `it_screenshot_window_occluded` | L2 | tests/sr-itests/tests/shot.rs | 自建红底目标窗口（先完整绘制）+ 青底覆盖窗口完全盖住其客户区并 SetWindowPos(HWND_TOP)；遮挡地面真值=目标客户区中心 WindowFromPoint 命中覆盖窗口（无法构造→BLOCKED fail）；截图后内容级判定：锁定 ffmpeg 解码 PNG→rgb24，红占比 ≥0.30（空白帧≈0）、青占比 ≤0.01（屏幕 Blt 式误实现≈0.9 青）——不止看尺寸/文件存在。最小化→明确错误未覆盖（§8 未决 2） |
| REQ-8 | —（L3 `manual_region_select_multi_monitor`） | L3 | — | 本任务明确不写（区域目标以程序化矩形覆盖于 REQ-5 用例） |

## 3. API 契约清单（测试即契约；模块名沿用 M1-A 骨架 `sr_shot::screenshot`）

```rust
/// crates/sr-shot —— 待实现符号（绿阶段目标）
pub mod screenshot {
    /// 物理像素矩形，桌面物理坐标系（REQ-6：负坐标合法；原点语义=桌面原点）。
    #[derive(Clone, Debug, PartialEq)]
    pub struct ScreenRect {
        pub x: i32,
        pub y: i32,
        pub width_px: u32,   // 物理像素（DPI 无关）
        pub height_px: u32,
    }

    /// 截图目标（REQ-5）。Monitor.index 与 sr_enum::monitors::MonitorEntry.index
    /// 为同一索引空间（RouteConfig::Monitor(idx) 兼容，§3.4）。
    #[derive(Clone, Debug, PartialEq)]
    pub enum ShotTarget {
        Monitor { index: u32 },
        Window { hwnd: u64 },          // HWND 数值化（usize→u64，与 WindowEntry.hwnd 同型）
        Region { rect: ScreenRect },   // 物理桌面坐标；跨显示器由实现拼合后裁剪（REQ-6）
    }

    pub struct ScreenshotRequest {
        pub target: ShotTarget,
        pub output_dir: PathBuf,       // 测试侧先创建好再传入（假定已存在）
    }

    pub struct SavedScreenshot {
        pub path: PathBuf,      // 输出目录内的 PNG 文件（REQ-5；命名规则属 REQ-24，M8 另测）
        pub width_px: u32,      // 物理像素（REQ-6）
        pub height_px: u32,
        pub source: ShotTarget, // 实际捕获来源回执（测试断言 == 请求目标）
    }

    /// 按值消费请求。错误策略：E 仅要求 Debug（测试经 .expect 消费）——与 M1 §3.4
    /// 相同，规格 §3.4 的 E_* 错误枚举由后续任务钉死（REQ-7 最小化错误将需要，
    /// 见 §8 未决 4）。
    pub fn take_screenshot(req: ScreenshotRequest) -> Result<SavedScreenshot, E>;
}
```

### 3.1 契约要点（非类型层面的行为契约）

- **单位**：`width_px/height_px` 与 `ScreenRect` 全部为**物理像素**；`x/y` 为桌面物理坐标（支持负值，多显示器负坐标场景本任务未排测但类型已支持）。
- **索引空间**：`ShotTarget::Monitor{index}` 必须复用 `sr_enum::monitors::MonitorEntry.index`（0..n-1 稠密，EnumDisplayMonitors 枚举序）——测试以 sr-enum（M1 已对 DEVMODE 验证）+ 测试侧 DEVMODE 双重交叉验证。
- **尺寸=真值**：显示器结果宽高必须等于 DEVMODE 当前设置的物理分辨率；区域结果宽高必须等于请求矩形宽高；PNG 文件实际编码尺寸（IHDR）必须等于元数据（元数据不得是空头支票）。
- **输出位置/格式**：结果必须是 output_dir 之内、扩展名 .png 的已存在文件；三目标调用产出互不相同路径。
- **窗口遮挡语义（REQ-7）**：被遮挡（非最小化、非 cloaked）的可见顶层窗口截图必须捕获**该窗口自身表面内容**（WGC 单帧语义），不得退化为屏幕抓取（覆盖窗口内容不得出现）也不得是空白帧。
- **错误策略**：本任务不钉错误枚举（仅 `E: Debug`，`.expect` 触发）；REQ-7 的「最小化窗口返回明确错误」与 REQ-28 降级路径需要错误码契约时再收紧。

## 4. 运行命令与门控

L2 门控沿用 M1 的 `#[ignore]` 门（理由见 m1-red-report.md §4；默认 `cargo test` 编译但不执行 L2，`--ignored` 显式执行时前置缺失走 BLOCKED=panic=fail，SKIP 不算通过）。

```text
# L2（显式执行；须在交互式用户会话）
cargo test -p sr-itests --test shot -- --ignored --nocapture
# 单用例示例
cargo test -p sr-itests --test shot it_screenshot_three_targets -- --ignored --nocapture

# 全仓（红阶段：因 shot 编译失败 exit 101 属预期）
cargo test --workspace
```

BLOCKED 触发点：非交互会话（Session 0）；锁定 ffmpeg 缺失（遮挡用例内容解码无法进行）；遮挡前置无法构造（目标客户区中心被第三方窗口占据）。

## 5. 新增依赖说明

- `tests/sr-itests/Cargo.toml` 仅追加被测 crate 的 path 依赖 `sr-shot`。**无任何新增外部 crate**（不新增 dev-dep）：
  - 内容级判定复用任务锁定的 `tools/ffmpeg/ffmpeg-master-latest-win64-lgpl/bin/ffmpeg.exe`（子进程 `-f rawvideo -pix_fmt rgb24 pipe:1` 解码 PNG，与 enum_audio.rs 用同一二进制）；
  - PNG 尺寸校验自解析 IHDR（8 字节签名 + 首块 width/height 大端 u32，无解码器）；
  - Win32 交叉验证沿用 sr-itests 已有 `windows 0.62` 依赖与同一 feature 集（未改 features）。

## 6. 红灯证据（实际执行，2026-10-16）

### 6.1 `cargo test --workspace` → exit 101（唯一编译错误，仅预期未解析符号）

```text
error[E0432]: unresolved imports `sr_shot::screenshot::take_screenshot`, `sr_shot::screenshot::SavedScreenshot`, `sr_shot::screenshot::ScreenRect`, `sr_shot::screenshot::ShotTarget`, `sr_shot::screenshot::ScreenshotRequest`
  --> tests\sr-itests\tests\shot.rs:39:27
   |
39 | use sr_shot::screenshot::{take_screenshot, SavedScreenshot, ScreenRect, ShotTarget, ScreenshotRequest};
   |                           ^^^^^^^^^^^^^^^  ^^^^^^^^^^^^^^^  ^^^^^^^^^^  ^^^^^^^^^^  ^^^^^^^^^^^^^^^^^ no `ScreenshotRequest` in `screenshot`
   |                           |                |                |           no `ShotTarget` in `screenshot`
   |                           |                |                no `ScreenRect` in `screenshot`
   |                           |                no `SavedScreenshot` in `screenshot`
   |                           no `take_screenshot` in `screenshot`

error: could not compile `sr-itests` (test "shot") due to 1 previous error
```

- 显式命令同果：`cargo test -p sr-itests --test shot -- --ignored --nocapture` → exit 101，同一 E0432（5 个未解析名字，全部来自本次新定义的 API）。
- 既有测试不受影响：`cargo test -p sr-itests --test enum_targets --no-run` / `--test enum_audio --no-run` → 均 Finished、exit 0；`cargo test -p sr-core --test channel_snapshot_rebind` → `1 passed; 0 failed`（L1 复绿证据）。

**红因确认**：全部错误均为「新测试引用的预期公共符号 unresolved（实现缺失）」，共 1 个 error、5 个名字，无任何其它编译错误/警告。

### 6.2 Win32/Helper 代码的仓库外运行时自检（M1-B 做法；scratch 不进仓库）

位置：`%TEMP%\opencode\sr-m2-red-scratch`（独立 cargo 工程，同版本 windows 0.62.2，`--offline`）。覆盖将写入 shot.rs 的全部非被测 helper 路径，实跑输出（exit=0，两次运行结果一致）：

```text
[env] OS: Microsoft Windows [Version 10.0.26200.9457]
[env] SESSIONNAME=Ok("Console")
target visible: true
WindowFromPoint(center=(300,231)) = 0x2652850 (target=0x821e02, cover=0x2652850)
PrintWindow(FULLCONTENT) 464x281: target-red=0.871, cover-teal=0.000
PNG IHDR: 320x240
decoded red fraction: 1.000
ALL HELPER CHECKS DONE
```

结论：类画刷纯色窗口 + 消息泵 + UpdateWindow 可靠渲染；覆盖窗口置顶后 WindowFromPoint 地面真值成立；**被遮挡目标窗口的 DWM 表面确实保有目标内容**（红 0.871、覆盖色 0.000，PrintWindow(PW_RENDERFULLCONTENT) 只读验证）——REQ-7「WGC 单帧截到内容」的前提在真实环境得到独立验证；PNG IHDR 解析与 ffmpeg rgb24 解码正确（320x240 纯色图红占比 1.000）。

### 6.3 stub-编译自检（落实 m1-red-report §9.4 备注的方法论改进）

M1 缺陷 2 证明 compile-red 对「import 可解析之后」的编译问题不设防。本次在 scratch 内以**契约 stub**（`pub mod screenshot` 提供 5 个符号的形状，函数体恒 `Err`，无任何生产逻辑）对仓库 `tests/sr-itests/tests/shot.rs` 原文件做全量 typecheck/borrowcheck：

```text
cargo test --offline --no-run   # scratch/typecheck 包（sr-shot 指向 stub，sr-enum 指向真实仓库 crate）
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.70s   # exit=0，0 error 0 warning
    Executable tests\shot.rs (target\debug\deps\shot-*.exe)
```

即：除预期的 5 个未解析符号外，测试文件的类型/借用/断言全部成立（含 helper 与 Win32 调用的真实签名核对：SetWindowPos/SWP_* 与 WindowFromPoint 在 WindowsAndMessaging、BeginPaint/EndPaint/ClientToScreen/UpdateWindow 在 Graphics::Gdi 等 windows 0.62 实际布局，均经 registry 源核对+scratch 编译验证）。

## 7. 关键设计取舍

| # | 取舍 | 依据 |
|---|---|---|
| R1 | 窗口载体=测试自建 Win32 窗口（红底/青底类画刷纯色背景），不自建消息线程、泵内联 | 与 M1 ProbeWindow 同思路：PID=测试进程、内容可控、销毁确定；纯色背景经 WM_ERASEBKGND 系统绘制，内容级判定无需应用自绘 |
| R2 | 目标窗口先完整绘制（泵 600ms + UpdateWindow + 泵 200ms）再创建覆盖窗口 | 被完全遮挡的窗口首次绘制可能收到空 rcPaint；先绘制保证 DWM 表面已含目标内容（scratch 验证 0.871） |
| R3 | 覆盖窗口=GetWindowRect 外扩 20px + SetWindowPos(HWND_TOP)，遮挡真值=客户区中心 WindowFromPoint 命中覆盖窗口 | 全程同一坐标空间（本进程虚拟化桌面坐标），任意 DPI 缩放下覆盖关系保持；WindowFromPoint 提供独立于被测实现的地面真值，无法构造前置即 BLOCKED |
| R4 | 内容级判定用锁定 ffmpeg 解码 PNG→rgb24+颜色占比，红 ≥0.30 / 青 ≤0.01 | 零新增 Rust 依赖（复用 enum_audio 同一锁定二进制）；阈值经 scratch PrintWindow 实测校准（0.871）并留出标题栏/圆角/边框余量，同时与两种失败模式（空白帧≈0 红、屏幕抓取≈0.9 青）拉开一个数量级 |
| R5 | 显示器真值=测试侧 DEVMODE 直接查询 + sr-enum 条目双链 | 任务要求「与直接 API 查询一致」；sr-enum（M1 绿）承担 index 空间定义并把 sr_shot 钉到 RouteConfig 兼容的索引上 |
| R6 | 区域目标取主显示器内 1/4 处 320x240 程序化矩形（按 DEVMODE 物理坐标计算并 clamp） | 不依赖交互（REQ-8 属 L3）；尺寸断言=请求矩形宽高，钉死物理像素契约 |
| R7 | `source` 回执=ShotTarget（derive PartialEq） | 「结果元数据（路径/宽/高/来源）足以完成断言」的最小形状；回执=请求目标，防止实现把目标张冠李戴 |
| R8 | 测试进程保持默认 DPI-unaware | 任何对 GDI/坐标的 DPI 虚拟化误用都会使结果偏离 DEVMODE 真值——这是 REQ-6 的判别力来源（同 M1 R3） |

## 8. 未决问题（返回 orchestrator）

1. **测试名与 §4 命名的范围偏差（spec 缺口）**：任务给定 `it_screenshot_monitor_physical_size`，而 §4 为 REQ-6 登记的是 `it_screenshot_mixed_dpi_negative_coords`。本测试只覆盖「单显示器维度=物理像素」；**区域跨显示器按物理坐标拼合后裁剪、负坐标**两个 REQ-6 明文行为未被任何 M2 用例覆盖。建议：或补 L2 用例（本机具备双显示器：3840x2160@ (0,0) + 1440x2560@(3840,442)，可直接构造跨屏/负坐标区域），或明确降级该行为到后续里程碑。
2. **REQ-7「最小化窗口返回明确错误」未测**：任务将用例限定为 `it_screenshot_window_occluded`（§4 原名含 `…_and_minimized_error`）。补测时需先钉错误契约（见 4），建议随绿阶段实现一并收紧（如断言 `code()=="E_TARGET_LOST"` 或专项变体）。
3. **窗口截图帧尺寸契约未钉死**：本测试仅断言 >0（WGC 窗口帧含标题栏/边框，具体等于 GetWindowRect 还是 DWM 扩展边框属实现语义）。若产品需要确定尺寸（如固定画布预览），需后续钉死并补断言。
4. **错误枚举仍未钉死**（沿 M1 未决 5）：`take_screenshot` 错误仅约束 Debug。REQ-7 最小化错误与 REQ-28 降级均需错误码；建议 M2 绿阶段实现 `sr_core::error::Error` 映射（骨架已存在）后回补断言。
5. **内容阈值依赖环境标定**：红 ≥0.30/青 ≤0.01 以本机（Win11 25H2，DWM 主题默认）scratch 证据校准；若绿阶段在暗色标题栏/高对比主题下红占比逼近阈值，应以实测证据上调/调整阈值而非放宽断言语义。
6. **PNG 命名/防覆盖（REQ-24）未在 M2 断言**（属 M8 的 `unit_naming_sanitize_no_overwrite`）；本测试仅要求「位于输出目录内、扩展名 png、三目标路径互不相同」这一最小形状。

## 10. 补充（2026-10-16 第二轮：§4 对照缺口补齐，仍为红阶段）

关闭 §8 未决 1/2（测试名与规格 §4 对齐 + 两处行为覆盖缺口），零生产改动、既有断言只增不改（`it_screenshot_window_occluded` 仅改名并**追加**最小化子用例，遮挡部分断言逐字保留）。

### 10.1 交付物变更

| 文件 | 变更 |
|---|---|
| `tests/sr-itests/tests/shot.rs` | ① `it_screenshot_window_occluded` → **`it_screenshot_window_occluded_and_minimized_error`**（§4 名），尾部追加最小化子用例；② 新增 **`it_screenshot_mixed_dpi_negative_coords`**（§4 名，REQ-6）；头部契约注释更新 |
| `docs/milestones/m2-red-report.md` | 追加本节 §10 |

### 10.2 契约增量（在 §3 基础上）

```rust
pub mod screenshot {
    /// 错误类型由「仅约束 Debug」收紧为具名枚举（M1「错误未钉」约定的最小收紧：
    /// 只钉 REQ-7 最小化语义所需的一个变体，其余变体/形状实现自由）。
    #[derive(Debug)]
    pub enum ShotError {
        TargetMinimized,   // 最小化窗口（IsIconic=true）→ take_screenshot 的明确错误；
                           // 测试以 matches!(.., ShotError::TargetMinimized { .. }) 断言
                           // （花括号 + .. 模式：单元变体与带字段变体均匹配）
        // …（实现自定；红测试不引用其它变体）
    }

    pub fn take_screenshot(req: ScreenshotRequest) -> Result<SavedScreenshot, ShotError>;
    // SavedScreenshot 追加 derive(Debug)（最小化子用例 expect_err 需要 Ok 侧 Debug）
}
```

### 10.3 新用例断言要点

| 用例 | 断言 |
|---|---|
| `it_screenshot_window_occluded_and_minimized_error` | 遮挡部分=原 `it_screenshot_window_occluded` 全部断言（红 ≥0.30/青 ≤0.01 内容级判定等，逐字未改）；追加：ShowWindow(SW_MINIMIZE) 后轮询 IsIconic 地面真值（5s）→ `expect_err`（不得成功）→ `matches!(err, ShotError::TargetMinimized { .. })`（明确错误，不得借道其它变体） |
| `it_screenshot_mixed_dpi_negative_coords` | 拓扑由 DEVMODE 动态拼接（零硬编码坐标）。子用例 1（跨屏）：找水平方向间隙最小的一对显示器，区域以交界/间隙中点为中心横跨两屏（前置自检断言矩形确实同时覆盖 A/B），输出尺寸=请求物理像素宽高 + IHDR=元数据 + source 回执；<2 台显示器 → BLOCKED。子用例 2（负坐标）：矩形置于负方向原点显示器内（x/y 为负且在屏内），成功 + 尺寸=请求 + IHDR + 回执；无负方向显示器 → BLOCKED（跨屏断言已先执行，语义不削弱） |

### 10.4 本机拓扑与限制（scratch 实测，per-monitor-aware V2 + DPI 探针）

```text
[env] Microsoft Windows [Version 10.0.26200.9457]；SESSIONNAME=Console
monitor (0,0)..(3840,2160)     3840x2160 primary=true  dpi=144x144   → 150% 缩放
monitor (3840,442)..(5280,3002) 1440x2560 primary=false dpi=120x120  → 125% 缩放
（两屏水平贴合：主屏右缘 3840 == 副屏左缘 3840；DEVMODE 拓扑与 M1/§6 一致）
```

- **跨屏/混合 DPI 子用例：本机可实测**（两台显示器缩放不同：150% vs 125%，贴合）。
- **负坐标子用例：本机 BLOCKED**——全部显示器位于桌面原点正方向（无负 x/y），无法构造有真实内容的负坐标矩形。测试已写出（供负坐标布局的多屏机器），运行到此打印 BLOCKED 并 fail（§4：SKIP 不算通过；上方跨屏断言已执行完毕）。
- 「不同缩放」非测试前置断言（DEVMODE 不暴露缩放、测试不得改系统显示设置）：本机恰好混合 DPI，物理尺寸断言本身即 DPI 判别（任一屏的缩放逻辑被误用于全域即得错尺寸）。

### 10.5 红证据（第二轮，实际执行 2026-10-16）

```text
cargo test --workspace → exit 101，仍仅 1 个编译错误（预期未解析符号 5→6，+1=ShotError）：
error[E0432]: unresolved imports `sr_shot::screenshot::take_screenshot`, `sr_shot::screenshot::SavedScreenshot`,
  `sr_shot::screenshot::ScreenRect`, `sr_shot::screenshot::ShotError`, `sr_shot::screenshot::ShotTarget`,
  `sr_shot::screenshot::ScreenshotRequest`   (tests\sr-itests\tests\shot.rs:44:5)
error: could not compile `sr-itests` (test "shot") due to 1 previous error
```

- 显式门 `cargo test -p sr-itests --test shot -- --ignored --nocapture` → exit 101，同一 E0432（6 名字）。
- 既有目标不受影响：enum_targets / enum_audio `--no-run` 均 Finished exit 0；sr-core L1 `1 passed; 0 failed`。
- **helper 运行时自检（scratch 复跑，含新增路径）**：`target minimized (IsIconic): true`（ShowWindow(SW_MINIMIZE)+IsIconic 地面真值成立）；原彩窗/遮挡/DWM 内容（红 0.871 青 0.000）/IHDR/ffmpeg 解码（1.000）全部复验通过，exit=0。
- **stub-编译自检（§6.3 方法论）**：契约 stub 增补 `ShotError { TargetMinimized }` 与 `SavedScreenshot` derive(Debug) 后，对更新后的 shot.rs 原文件全量 typecheck：Finished、exit=0、0 error 0 warning（同时证实 `ShotError::TargetMinimized { .. }` 花括号模式对单元变体可匹配）。

### 10.6 未决项更新

- §8-1（REQ-6 跨屏/负坐标缺口）、§8-2（REQ-7 最小化错误缺口）：**已由本节补齐**（负坐标子用例在本机 BLOCKED 属环境限制，非语义削弱，见 10.4）。
- §8-3（窗口帧精确尺寸未钉）、§8-5（内容阈值标定）、§8-6（REQ-24 命名）：维持原状。
- §8-4（错误枚举）：**部分收紧**——现为具名 `ShotError` 且钉 `TargetMinimized` 一个变体；其余变体（显示器索引越界、WGC 初始化失败、输出目录不可写等）仍未钉，建议绿阶段实现 `sr_core::error::Error` 映射（规格 §3.4：窗口丢失类 E_TARGET_LOST 语义）后按需回补。
- 新增残留风险备注：跨屏区域用例以「尺寸+IHDR+回执」判别（尺寸错=实现按单屏缩放/DPI 虚拟化即失败），**不判别内容拼接正确性**（尺寸正确但左右两半内容错位的实现不会被本用例捕获）；如需内容级判别，可后续在两屏分别放置可区分测试图案后追加颜色断言（与遮挡用例同手法）。
