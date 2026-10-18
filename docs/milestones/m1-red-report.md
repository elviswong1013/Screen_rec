# M1-B 红测试报告（TDD 红阶段：目标枚举 REQ-1..4 + Channel 语义 REQ-31）

- 日期: 2026-10-16
- 规格: specs/001-screen-recorder.md §2.1（REQ-1..4）、§2.8（REQ-31）、§3.2/§3.3/§3.4、§4、§5（M1 行）
- 角色: Test Writer（红阶段）。本任务只写测试，未写任何生产实现。
- 环境实测: Windows 11 25H2，build **10.0.26200.9457**，SESSIONNAME=Console（交互式用户会话）；rustc/cargo 1.99.0 msvc（rust-toolchain.toml 锁定）。

## 1. 交付物（测试文件，全部为新增）

| 文件 | 内容 |
|---|---|
| `tests/sr-itests/tests/enum_targets.rs` | L2 ×4：窗口枚举 / 显示器枚举 / 刷新 / cloaked 过滤 |
| `tests/sr-itests/tests/enum_audio.rs` | L2 ×1：活动音频会话与快照 |
| `tests/sr-itests/tests/common/mod.rs` | 测试共享助手（BLOCKED 机制、前置检查、Win32 交叉验证地面真值、ProbeWindow） |
| `tests/sr-itests/Cargo.toml` | 追加依赖：sr-core / sr-enum（path）、windows 0.62（仅测试用，见 §5） |
| `crates/sr-core/tests/channel_snapshot_rebind.rs` | L1 ×1：Channel 订阅语义（纯逻辑，不依赖 Tauri 运行时，未用 tauri::test） |

未改动：specs/、生产 crate 的 src/（sr-enum/sr-core 仍是 M1-A 空占位）、tools/、docs/ 既有内容（本报告除外）。未 git 提交。

## 2. REQ → 测试 → 文件映射（测试名与 §4 一字不差）

| REQ | 测试名 | 层 | 文件 | 断言要点 |
|---|---|---|---|---|
| REQ-1 | `it_enum_windows_lists_visible_app_name` | L2 | tests/sr-itests/tests/enum_targets.rs | 自建可见窗口 + spawn notepad 双载体：标题（=GetWindowTextW 真值）/应用名（非空）/进程名（=属主进程映像名）/PID（=窗口属主）/身份字段（HWND+PID+进程创建时间，与 GetProcessTimes 真值偏差 ≤2s）；条目按 HWND 去重；cloaked 过滤（每个存活条目 DWMWA_CLOAKED=0） |
| REQ-2 | `it_enum_monitors_physical` | L2 | tests/sr-itests/tests/enum_targets.rs | 数量=EnumDisplayMonitors；物理分辨率+位置=EnumDisplaySettingsW(ENUM_CURRENT_SETTINGS) 的 DEVMODE（钉死 DPI 无关的物理像素契约）；主显示器标识=MONITORINFOF_PRIMARY 恰一个且位置一致 |
| REQ-3 | `it_enum_refresh_reflects_open_close` | L2 | tests/sr-itests/tests/enum_targets.rs | 自建窗口：打开→一次刷新可见；DestroyWindow→一次刷新消失；单次调用 ≤2s（条目数 ≤100 时断言，>100 打 note 不判定——规格预算以 ≤100 窗口为前提） |
| REQ-3（cloak 关联 REQ-1） | `it_enum_uwp_cloaked_filter` | L2 | tests/sr-itests/tests/enum_targets.rs | DWMWA_CLOAK 自造 cloaked 前置条件（与 UWP 幽灵同机制）→ 不被列出；未 cloak 时先被列出（防空洞通过）；解除后重新列出（对称性，排假阳性）；系统自然 cloaked 窗口一并断言不泄露；无法构造前置 → BLOCKED（fail） |
| REQ-4 | `it_list_active_audio_sources_snapshot` | L2 | tests/sr-itests/tests/enum_audio.rs | 锁定 ffplay 播放临时生成 440/880Hz WAV → 快照含 PID/进程名(含 ffplay)/应用名(非空)/身份创建时间（≤2s 偏差）；同 PID 恰一条（同根去重弱形式）；「全选=快照」M1 可测部分：启动后新增音源 B 不自动并入先前快照、新取快照含 A+B；B 退出后会话消失（容忍回收延迟，轮询 ≤20s） |
| REQ-31 | `unit_channel_snapshot_rebind` | L1 | crates/sr-core/tests/channel_snapshot_rebind.rs | subscribe 返回**当前**状态快照（非初始）；UI 重绑后旧接收端全灭时 publish 不中断（类型钉死返回 `()`）且新订阅继续收事件；告警与电平分通道（200 条电平洪泛后告警 1s 内送达）；电平 ≤10Hz：LevelThrottle 确定性单测（0/50/100ms 边界）+ 通道级行为（瞬时 50 条在 ~250ms 窗口送达 ≤10 条且 ≥1） |

## 3. API 契约清单（测试即契约）

### 3.1 `crates/sr-enum`（模块名来自 M1-A 骨架，已存在；以下为待实现符号）

```rust
/// pub mod windows（REQ-1/3）
pub struct WindowEntry {
    pub hwnd: u64,               // 测试假定：HWND 数值化（usize→u64）
    pub pid: u32,
    pub process_created_at: u64, // 测试假定：进程创建时间，UNIX epoch 毫秒（身份字段，防 PID 复用）
    pub title: String,           // 窗口标题（GetWindowTextW 语义）
    pub app_name: String,        // 应用显示名（非空；UWP 经统一身份解析归属真实进程）
    pub process_name: String,    // 属主进程映像名，如 "Notepad.exe"
}
pub fn list_windows() -> Result<Vec<WindowEntry>, E>; // E 见 3.4

/// pub mod monitors（REQ-2）
pub struct MonitorEntry {
    pub index: u32,        // 测试假定：0..n-1 稠密唯一（RouteConfig::Monitor(idx) 的引用基础）
    pub is_primary: bool,
    pub x: i32,            // 物理桌面坐标（可为负；DEVMODE dmPosition 语义）
    pub y: i32,
    pub width_px: u32,     // 物理像素（DPI 无关）
    pub height_px: u32,
}
pub fn list_monitors() -> Result<Vec<MonitorEntry>, E>;

/// pub mod audio_sessions（REQ-4）
pub struct AudioSessionEntry {
    pub pid: u32,
    pub process_created_at: u64, // 测试假定：UNIX ms，与 REQ-1 身份同源
    pub process_name: String,    // 如 "ffplay.exe"
    pub app_name: String,        // 非空
}
pub fn list_audio_sessions() -> Result<Vec<AudioSessionEntry>, E>;
```

### 3.2 `crates/sr-core`（**测试假定**：新增 `pub mod events`，REQ-31 纯逻辑落点）

```rust
/// src-tauri 只做绑定 Tauri Channel 的薄包装；快照/容忍/分通道/节流均为纯逻辑。
pub mod events {
    /// S=状态快照类型，A=告警·状态事件，L=电平事件（实现侧泛型；测试用本地替身类型驱动）
    pub struct ChannelHub<S, A, L> { /* 内部结构由实现定 */ }

    impl<S: Clone + Send + 'static, A: Clone + Send + 'static, L: Clone + Send + 'static>
        ChannelHub<S, A, L>
    {
        pub fn new(initial_state: S) -> Self;
        pub fn update_state(&self, state: S);                    // 测试假定：推进当前状态
        pub fn subscribe(&self) -> Subscription<S, A, L>;        // 订阅入口：返回当前快照 + 两条独立接收端
        pub fn publish_alert(&self, alert: A);                   // 测试假定：返回 ()（发送失败内部容忍）
        pub fn publish_level(&self, level: L);                   // 测试假定：返回 ()；电平流内部节流 ≤10Hz
    }

    pub struct Subscription<S, A, L> {
        pub snapshot: S,          // 当前状态快照（订阅时刻）
        pub alerts: Receiver<A>,  // 告警/状态通道（独立于电平，不被洪泛饿死）
        pub levels: Receiver<L>,  // 电平通道（≤10Hz）
    }
    // 测试假定：Receiver 为同步通道，具备 recv_timeout(Duration)/try_recv（std mpsc 或
    // crossbeam-channel 均满足；勿用需异步运行时的通道）。

    /// 电平节流 ≤10Hz 的确定性逻辑（显式时间参数，可注入时钟）。
    pub struct LevelThrottle { /* … */ }
    impl LevelThrottle {
        pub fn new(min_interval: Duration) -> Self;  // 10Hz ⇒ Duration::from_millis(100)
        pub fn should_emit(&mut self, now: Instant) -> bool; // 首事件立即放行；间隔不足则节流
    }
}
```

### 3.3 契约要点（非类型层面的行为契约）

- `list_windows` 结果**不含 cloaked 窗口**（DWMWA_CLOAKED≠0），含可见顶层窗口；窗口消失后在下一次调用即不再列出（REQ-3）。
- `list_windows` 刷新预算：条目 ≤100 时单次调用 ≤2s。
- `list_audio_sessions` 返回**值快照**（非活动视图）：同一 Vec 不会因后续新增音源而变化（「全选=启动时快照」的数据模型钩子；路由级冻结写入会话清单属 M3+ 测试范围）。
- `ChannelHub::publish_*` 对已失效订阅（接收端 drop）不得 panic/外溢错误；不影响其余订阅者。
- 电平洪泛（200 条瞬时）不得阻塞/饿死告警通道（REQ-27 关联）。
- 电平流 ≤10Hz：瞬时 50 条在 ~250ms 窗口送达 ≤10 条且 ≥1 条。

### 3.4 错误类型（有意未钉死）

测试仅经 `.expect(..)` 消费错误（要求 `E: Debug`），**未**引用具体错误枚举。理由：规格 §3.4 的 E_* 错误枚举属于 sr-core `error` 模块的后续任务契约，红测试不代为定型；实现可自选（建议 `sr_core::error::Error`，见规格 §3.4），未来任务钉死后再收紧。

## 4. 运行命令与门控

**L2 门控选择：`#[ignore]` 门**（二选一中的选择及理由：环境变量门要么污染默认 `cargo test`（未设变量即 fail）要么造成静默 skip，均违反「不污染默认测试」与「SKIP 不算通过」的组合要求；`#[ignore]` 使默认 `cargo test` 编译但不执行 L2，`--ignored` 显式执行时前置条件缺失再走 BLOCKED（fail））。每个 L2 用例标注了 ignore reason（含运行命令）。

```text
# L1（随默认 cargo test 执行）
cargo test -p sr-core --test channel_snapshot_rebind -- --nocapture

# L2（显式执行；须在交互式用户会话）
cargo test -p sr-itests --test enum_targets -- --ignored --nocapture
cargo test -p sr-itests --test enum_audio    -- --ignored --nocapture
# 单用例示例
cargo test -p sr-itests --test enum_targets it_enum_monitors_physical -- --ignored --nocapture

# 全仓
cargo test --workspace
```

**BLOCKED 机制（§4）**：每个 L2 用例开头执行 `require_interactive_session()`（SESSIONNAME=Console/RDP-*，Session 0 不合格）与 `record_os_build()`（`cmd /c ver`，实测输出 `Microsoft Windows [Version 10.0.26200.9457]`）；ffplay/ffmpeg 缺失、notepad 窗口不可得、DWMWA_CLOAK 无法置位、音频端点不可用（ffplay 早退）等前置失败均 `panic!("BLOCKED: …")` → 测试**失败**（非 skip）。

## 5. 新增测试侧依赖说明

`tests/sr-itests/Cargo.toml` 追加：

- `sr-core`、`sr-enum`（path 依赖）——被测产品 crate。
- `windows = "0.62"`（features：Foundation/Graphics_Dwm/Graphics_Gdi/System_LibraryLoader/System_Threading/UI_WindowsAndMessaging）——**仅测试用**的 Win32 交叉验证（与规格 §1 锁定的枚举栈同版本；Cargo.lock 中 0.62.2 已因 Tauri 存在，无新增下载版本）。仅进入 sr-itests（测试容器）的依赖，不进入产品 crate。

`crates/sr-core` 无新增依赖（REQ-31 测试为纯 std 逻辑）。

## 6. 红灯证据（实际执行，2026-10-16）

### 6.1 `cargo test --workspace` → exit 101

```text
error[E0432]: unresolved import `sr_core::events`
  --> crates\sr-core\tests\channel_snapshot_rebind.rs:29:14
   |
29 | use sr_core::events::{ChannelHub, LevelThrottle};
   |              ^^^^^^ could not find `events` in `sr_core`

error[E0432]: unresolved import `sr_enum::monitors::list_monitors`
  --> tests\sr-itests\tests\enum_targets.rs:25:5
   |
25 | use sr_enum::monitors::list_monitors;
   |     |                        no `list_monitors` in `monitors`

error[E0432]: unresolved imports `sr_enum::audio_sessions::list_audio_sessions`, `sr_enum::audio_sessions::AudioSessionEntry`
  --> tests\sr-itests\tests\enum_audio.rs:28:31
   |                               ^^^^^^  ^^^^^^^^^^^^^^^^ no `AudioSessionEntry` in `audio_sessions`
   |                               no `list_audio_sessions` in `audio_sessions`

error[E0432]: unresolved import `sr_enum::windows::list_windows`
  --> tests\sr-itests\tests\enum_targets.rs:26:5
   |     |                     no `list_windows` in `windows`

error[E0425]: cannot find type `WindowEntry` in module `sr_enum::windows`
  --> tests\sr-itests\tests\enum_targets.rs:329:54
   |                                                      ^^^^^^ not found in `sr_enum::windows`

error: could not compile `sr-core` (test "channel_snapshot_rebind") due to 1 previous error
error: could not compile `sr-itests` (test "enum_audio") due to 1 previous error
error: could not compile `sr-itests` (test "enum_targets") due to 3 previous errors
```

### 6.2 显式命令（与 6.1 同类错误，均 exit 101）

- `cargo test -p sr-itests --test enum_targets -- --ignored --nocapture` → 3 个错误（`list_monitors`、`list_windows`、`WindowEntry`）。
- `cargo test -p sr-itests --test enum_audio -- --ignored --nocapture` → 1 个错误（`list_audio_sessions`、`AudioSessionEntry`）。
- `cargo test -p sr-core --test channel_snapshot_rebind -- --nocapture` → 1 个错误（`sr_core::events`）。

**红因确认**：全部 5 个错误均为「新测试引用的预期公共符号 unresolved（实现缺失）」；测试自身代码（含 Win32 助手）0 错误 0 警告，并在独立 scratch 工程（仓库外，同版本 windows 0.62.2）以同一助手代码完成运行时自检：

```text
[env] OS: Microsoft Windows [Version 10.0.26200.9457]
self process created (unix ms) = 1792083078351
probe hwnd=0x662844
probe window lifecycle OK (create/visible/cloak/uncloak/destroy)
monitor: x=3840 y=442 1440x2560 primary=false
monitor: x=0 y=0 3840x2160 primary=true
notepad launcher pid=119272 (window owner may differ)
notepad window: hwnd=0x2f62104 pid=115884 title="sr_helper_note_26672.txt - Notepad"
notepad owner created (unix ms) = 1792083078507
notepad window gone after WM_CLOSE: true
ALL HELPER CHECKS DONE
ffplay alive after 3s: True   # 锁定 ffplay + 临时生成 440Hz WAV 冒烟（音频端点可用）
```

即：失败原因=实现缺失，而非测试笔误/环境不可用。

## 7. 关键设计取舍（供实现方与 orchestrator 知悉）

| # | 取舍 | 依据 |
|---|---|---|
| R1 | REQ-1/3 的「自 spawn 窗口」以**测试自建 Win32 窗口**（ProbeWindow）为主载体，notepad 为辅（REQ-1 用例内必选、字段跨应用验证；其窗口经标题标记定位后以 WM_CLOSE 清理） | 本机实测 Win11 25H2 记事本合并标签页：两次 spawn 仅 1 个窗口、启动器 PID ≠ 窗口属主 PID（119272≠115884），纯 notepad 无法提供确定性的「新窗口出现/窗口消失」；自建窗口 PID=测试进程、标题可控、销毁确定，字段断言（PID/进程名/创建时间）反而更强 |
| R2 | REQ-1 cloaked 过滤用 DWMWA_CLOAK **自造** cloaked 窗口作前置条件，另扫描系统自然 cloaked 窗口一并断言 | 使 §4 的 cloaked 前置确定性成立（不依赖系统恰有挂起 UWP）；无法自造时才 BLOCKED。「未 cloak 先可见 → cloak 不可见 → 解除复可见」三段式排除了空列表假阳性 |
| R3 | REQ-2 交叉验证用 DEVMODE（EnumDisplaySettingsW）而非 GetMonitorInfo rcMonitor | 测试进程默认 DPI-unaware 时 rcMonitor 被 DPI 虚拟化；以 DEVMODE 为真值钉死「物理分辨率/物理坐标」契约（若实现依赖进程 DPI 感知则测试失败，符合 REQ-2/REQ-6 意图） |
| R4 | REQ-4 的「全选=快照」仅测 M1 可测部分（返回值为快照、新增音源不自动并入旧快照、退出后会话消失） | 路由级「启动时快照写入会话清单」依赖 RouteConfig/SessionManifest（M3+），届时由对应里程碑测试收紧；本测试不预先发明会话级 API |
| R5 | REQ-31 纯逻辑置于 sr_core::events（ChannelHub/LevelThrottle），不用 tauri::test | 任务优先级：不依赖 Tauri 运行时；src-tauri 后续只做薄包装。ChannelHub 三泛型由测试替身类型驱动，不预设载荷枚举（route_state/route_level 载荷定型属后续里程碑） |
| R6 | `publish_alert/publish_level` 返回类型以 `let (): () = …` 钉死为 `()` | 「发送失败不得中断录制」的 API 级表达：失败只允许内部吞掉；实现若想返回 Result 需先与规格方确认 |
| R7 | L2 门控取 `#[ignore]`（理由见 §4）；红阶段 `cargo test --workspace` 因编译失败而 fail 属预期 | 绿阶段后默认 `cargo test` 将编译 L2 但不执行（ignored），不污染 CI |

## 8. 未决问题（返回 orchestrator）

1. **`process_created_at` 单位**：测试假定为 UNIX epoch 毫秒（u64）。若实现方倾向 FILETIME/100ns 或纳秒，需改测试两处比较（`enum_targets.rs`、`enum_audio.rs`）——属契约分歧，应在此钉死。
2. **`MonitorEntry.index` 稠密性（0..n-1）** 为测试假定；规格仅有 `RouteConfig::Monitor(idx)` 隐含索引引用。若实现打算用稳定设备 ID 而非连续索引，REQ-2/RouteConfig 契约需同步澄清。
3. **`ChannelHub` 归属与命名**（sr_core::events、update_state/publish_* 命名、Subscription 字段形状）为测试假定；规格 §3.3 只定义行为与 Tauri command 侧。实现若另设归属（如 sr-core::channel），需同步改测试 import，属可协商项非阻塞。
4. **REQ-4 路由级快照语义**（写入会话清单、UI 手动追加）超出 M1 可测范围，本红测试仅覆盖数据模型钩子；M3+ 需补 `it_list_active_audio_sources_snapshot` 的路由级后继测试或并入 REQ-17/19 用例。
5. **错误枚举未钉死**（§3.4）：当前所有被测函数的错误类型仅约束 `Debug`。建议 M1 实现任务先落 `sr_core::error::Error`（E_* 变体）再回头收紧断言。
6. 规格 §4 将 `it_enum_uwp_cloaked_filter` 映射到 REQ-3 行下，但其断言内容属 REQ-1 的 cloaked 过滤语义（本测试两者都覆盖）；记账时按 §4 原映射即可。

## 9. 2026-10-16 测试缺陷修订（M1-C 执行者报告的两处缺陷）

到场状态说明：两处修复已在测试文件上以最小改动先行施加（M1-C 执行者紧急解锁所为）。本次由测试作者完成：**差异审查（确认无弱化）→ 修复正确性确认与同类 UB 全量复审 → 编译/运行验证 → 本记录**。经审查，两处修复与测试作者将采用的修法一致，予以确认并接管；未发现需要返工之处。

**约束遵守**：未改动任何断言、期望值、测试名、`#[ignore]` 门与 BLOCKED 语义。逐字比对结论——`enum_targets.rs`、`crates/sr-core/tests/channel_snapshot_rebind.rs`、`tests/sr-itests/Cargo.toml` 与红测试原稿**完全一致**（执行者未触碰）；`enum_audio.rs` 仅 2 处 `mut`；`common/mod.rs` 仅 1 处 HSTRING 局部化。无弱化，无需恢复。

### 9.1 缺陷 1：`tests/sr-itests/tests/common/mod.rs` 悬垂指针 UB

- **现象**：并行测试模式下 `enum_targets` 偶发失败（ProbeWindow 构造路径异常）。
- **根因**（定位）：`register_probe_class` 原写法把 `HSTRING` **临时值**的缓冲指针直接存入 `WNDCLASSW`：

  ```rust
  lpszClassName: PCWSTR::from_raw(HSTRING::from("SR_M1_RED_PROBE_WND").as_ptr()),
  ```

  该临时 HSTRING 在 `let wc = WNDCLASSW{…};` 语句结束即释放，`wc.lpszClassName` 随即**悬垂**；随后 `RegisterClassW(&wc)` 读取已释放堆内存。串行/低堆扰动时「碰巧」成功（这正是红阶段 scratch 自检未暴露的原因——UB 非必然故障）；并行测试堆复用后类名损坏 → `CreateWindowExW` 以 `ERROR_CANNOT_FIND_WND_CLASS` 失败 → probe 窗口构造 panic → 偶发失败。
- **修复（前 → 后 diff）**：

  ```diff
   fn register_probe_class() {
       CLASS_REGISTERED.call_once(|| unsafe {
           let hinst = GetModuleHandleW(None).ok().expect("GetModuleHandleW");
  +        // 绑定至局部变量使其存活到 RegisterClassW 之后（原写法 HSTRING 临时值在语句末
  +        // 释放，lpszClassName 悬垂，属 UB：并行测试下堆复用导致 ERROR_CANNOT_FIND_WND_CLASS）
  +        let class_name = HSTRING::from("SR_M1_RED_PROBE_WND");
           let wc = WNDCLASSW {
               lpfnWndProc: Some(probe_wndproc),
               hInstance: hinst.into(),
  -            lpszClassName: PCWSTR::from_raw(HSTRING::from("SR_M1_RED_PROBE_WND").as_ptr()),
  +            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
               ..Default::default()
           };
  ```

  具名局部变量的存活期覆盖 `RegisterClassW` 调用——生命周期正确。无法完全去 unsafe（Win32 FFI 固有），符合「必须 unsafe 时确保生命周期/所有权正确」。
- **同类 UB 全量复审**（common/mod.rs 全部 FFI 指针用法逐一核查，无其它同类问题）：
  - `ProbeWindow::new`：`class`/`title` HSTRING 为具名局部，存活覆盖 `CreateWindowExW` ✓
  - `monitor_cb`：`dev_name` HSTRING 为具名局部，存活覆盖 `EnumDisplaySettingsW` ✓
  - `enum_sys_windows`/`enum_sys_monitors`：LPARAM 携带的输出缓冲指针指向存活于**同步**枚举调用全过程的局部 `Vec`——非悬垂（标准 EnumWindows 惯用法）✓
  - `cloak_of_u64`/`set_cloak`/`close_window`/`GetMonitorInfoW`/`GetProcessTimes`：指针均指向覆盖调用期的具名局部 ✓
- **验证**：`cargo test -p sr-itests --test enum_targets -- --ignored` 并行模式共 4 次运行全绿（见 9.3）。

### 9.2 缺陷 2：`tests/sr-itests/tests/enum_audio.rs` 编译错误

- **现象**：M1-C 实现 `sr_enum::audio_sessions` 后，测试在类型/借用检查阶段报错（E0596 cannot borrow as mutable）。
- **根因**：`assert_entry_identity` 轮询分支中 `player_a.0.try_wait()` / `player_b.0.try_wait()` 需要 `&mut Child`，而绑定缺少 `mut`。红阶段未暴露的原因：`use sr_enum::audio_sessions::{…}` 名称解析失败使编译在**更早阶段**中止，借用检查从未执行——compile-red（unresolved import）手法只能保证所报错误集合，无法保证其后无被掩盖错误；此局限记录在案（未来红测试的编译自检应覆盖「import 解析后」路径，例如对被测符号先 stub 再全量 check 的过渡步骤）。
- **修复（前 → 后 diff）**，纯编译机制修复，无语义变化：

  ```diff
  -    let player_a = spawn_ffplay(&wav_a);
  +    let mut player_a = spawn_ffplay(&wav_a);
   …
  -    let player_b = spawn_ffplay(&wav_b);
  +    let mut player_b = spawn_ffplay(&wav_b);
  ```

- **契约对齐核对**：M1-C 实现（`crates/sr-enum/src/audio_sessions.rs`）与本报告 §3.3 契约逐项一致——`AudioSessionEntry { pid: u32, process_created_at: u64(UNIX ms), process_name: String, app_name: String }`、`list_audio_sessions() -> Result<Vec<AudioSessionEntry>, Error>`。**无契约假定需要调整**，未产生新的未决项。
- **验证**：`cargo test -p sr-itests --no-run` 通过；`--ignored` 实跑 1 passed（见 9.3）。

### 9.3 验证命令与结果（实际执行，2026-10-16，本机 Win11 25H2 build 26200.9457 / SESSIONNAME=Console）

| 命令 | 结果 |
|---|---|
| `cargo test -p sr-itests --no-run` | exit 0；enum_targets / enum_audio / lib 三目标全部编译（缺陷 2 修复的验收项） |
| `cargo test -p sr-core --test channel_snapshot_rebind -- --nocapture` | `1 passed; 0 failed`（L1，REQ-31） |
| `cargo test -p sr-itests --test enum_targets -- --ignored --nocapture` | `4 passed; 0 failed`（**并行模式**，缺陷 1 场景） |
| 同上，追加复跑 ×3 | 三次均 `4 passed; 0 failed`（累计 4 连绿） |
| `cargo test -p sr-itests --test enum_audio -- --ignored --nocapture` | `1 passed; 0 failed`（ffplay 实音源） |
| `cargo test --workspace` | exit 0；L1 全绿，L2 按门 `4+1 ignored; 0 failed`——默认套件不受污染（§4 门控生效） |

（绿由 M1-C 执行者最终负责；以上运行结果为缺陷修复的参考证据。）

### 9.4 本次新增未决/备注

7. **测试运行残留（低严重度，超出本次授权范围未改）**：`it_enum_windows_lists_visible_app_name` 遗留 `sr_m1_red_<pid>` 临时目录（原稿即未清理）；`it_list_active_audio_sources_snapshot` 的 `remove_dir_all` 先于 `player_a` 的 KillOnDrop 执行，ffplay 仍占用 WAV → 删除失败被 `let _ =` 忽略，每次运行最多残留 ~46MB 音频。本次已手工清理积压；建议后续授权将 player drop 前移至清理之前（零断言语义变化的机制微调）。
8. **红测试方法论备注**：缺陷 2 揭示 compile-red 手法对「import 可解析之后」的编译问题不设防；后续红测试交付可在报告固定加注此局限，或在验证步骤中增加 stub-编译自检。
