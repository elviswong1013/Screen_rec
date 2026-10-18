# M5 红测试报告（TDD 红阶段：多路同时录制 REQ-12 的 M5 任务范围）

- 日期: 2026-10-16
- 规格: specs/001-screen-recorder.md §2.3（REQ-12 全文）、§3.1/§3.2/§3.3（start_routes 批量契约/管道协议/ffmpeg 基准）、§3.4/§3.5、§4（REQ-12 行测试映射）、§5 M5 行（含三路停中）、§9 T1（保底 2 路 1080p30）、§8 M0 冻结项（S1：双/三路 30fps 实测通过）
- 先例: docs/milestones/m3-red-report.md 与 m4-red-report.md（契约/门/双层自检方法论）、m3/m4-green-report（现有 route API 面貌）、docs/test-fix/2026-10-16-m4-audio-l2-av.md（MTA 保活修复——本会话时间线相关，见 §6.4）
- 角色: Test Writer（红阶段）。本任务只写测试，未写任何生产实现（生产 src 零改动）。
- 环境实测: Windows 11 25H2，build **10.0.26200.9711**（UBR 自 M3/M4 时代的 9457 升至 9711，见 m4 AV 取证文档时间线注记），SESSIONNAME=Console；rustc/cargo 1.99.0 msvc；sidecar/ffprobe=锁定 `tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin`（§8 冻结）。
- 到场状态: M4 三处测试缺陷已由测试作者修复（silence_pad_late_packet.rs:99、audio_isolation spawn_cmd_ffplay/bp 切片——本机复验 L1 全绿）；M4 遗留 AV 已由生产侧 `wgc.rs` MTA 保活修复（今日 12:37 落盘）。仓库无 git 提交历史（全部 untracked）。

## 1. 交付物

| 文件 | 性质 | 内容 |
|---|---|---|
| `tests/sr-itests/tests/multi_route.rs` | 新增（928 行） | L2 ×4：两路隔离停一（含目标窗口关闭优雅停止+可选音频）/ 三路停中 / 单路故障隔离 / 批次全成功或回滚 |
| `docs/milestones/m5-red-report.md` | 新增 | 本报告 |

未改动：生产 src（sr-capture/sr-pipeline 等全部未动）、specs/、docs/spikes/、既有测试（M1–M4 逐字未动；`common/mod.rs` 零改动——新 helper 全部内聚在 multi_route.rs，沿 M3/M4 先例）、`tests/sr-itests/Cargo.toml`（**零新依赖**——复用 sr-pipeline/sr-audio/windows 现有 deps，无 FFT 需求故未触 rustfft）。未 git 提交。

## 2. REQ → 测试 → 文件映射（§4 命名优先；拆分子用例按任务要求命名清晰）

| REQ-12 条款 | 测试名 | 层 | 断言要点 |
|---|---|---|---|
| ≥2 路同时录制（§9 T1 保底 **2 路 1080p30**）；每路独立输出文件、独立停止；「视频+可选音频」 | `it_two_routes_isolated_stop_one`（§4 点名） | L2 | 2 路 1080p30 批量启动：路 A=视频+应用音轨（ffplay 440Hz）、路 B=纯视频；~5.5s **关闭路 B 目标窗口**（REQ-12：目标窗口关闭 → 该路**优雅停止并保留内容**，wait_finalized=Ok）；路 A 不受影响继续录到 ~9s 并正常收尾；两成品路径互异、各在自己输出目录、命名含各自 sanitize(标题)；A 恰 1 音轨（aac/48k/2ch、去首尾 RMS ≥0.005 非静音）、B 0 音轨；dur_A > dur_B+1.5s；两路非黑+运动+**各自标记色存在（≥2%）且其它路标记色不存在（≤0.5%）**；整文件可解码 |
| 含三路停中（§4/§5 M5 点名） | `it_three_routes_stop_middle` | L2 | **3 路 1080p30**（S1 实测三路可行；超 T1 保底按本机能力）批量启动；~6s `request_stop` **中间一路** → 收尾 Ok；两侧路不受影响继续录到 ~10s → 全部收尾 Ok；三成品两两互异、各自目录、各自标题 token、各自 span ±2s；每路恰 1 视频轨 0 音轨、非黑+运动+标记色各自正确互不串扰；两侧时长 > 中间路+1.5s（「其余不受影响」的时长级证据） |
| 单路故障不影响其它路 | `it_multi_route_single_fault_isolated`（子用例，任务授权命名） | L2 | 2 路 640x360 录制中 `taskkill /T /F` **其中一个** ffmpeg sidecar（Toolhelp32 只给父子关系不给 sidecar→路由映射，故**对称断言**）：恰 1 路 `Err(SidecarExit)`（强匹配）且保留段非空、非 0 字节、首段可独立读时长+可解码；恰 1 路 Ok 且成品有效（时长≈span、非黑+运动+own 标记） |
| 批次启动全成功或回滚（§3.3 start_routes） | `it_routes_batch_start_all_or_rollback`（子用例） | L2 | `[有效目标, 无效 hwnd 0xDEADBEEF]` 批量 → 必须整体失败（Err(TargetInvalid)，非 Ok）；**回滚完成**：Err 后 10s 轮询内无 ffmpeg 子进程残留；有效路输出目录不得残留成品（段文件按保留策略可留——「成品=已完成的录制」不得存在，REQ-24 提交语义：未获批次接受的产物不得提升到输出目录） |

内容级判定手法（每路身份）：被录目标为**自带唯一标记色的运动窗口**（专用泵线程 33ms 定时器 + 静态 50%×50% 标记块 + 全高移动亮条；路 0 绿 (60,220,90) / 路 1 品红 (220,60,200) / 路 2 青 (60,200,220)；**工作区 3 等分显式槽位铺排、互不遮挡**）。采样解码帧（3fps/64x36，M3 同款）箱式判定：本路标记 ≥2% 像素、其它路 ≤0.5%；标记色与图案元素（灰底/白条）在判定域两两不相交，且对 BT.601/BT.709 矩阵互换与 limited/full range 漂移均保有 ≥30 单位裕度（手算 + scratch 实测双确认）。窗口创建经 `WM_NCCREATE/lpCreateParams → GWLP_USERDATA` 传递路序号（标准 Win32 模式）。

## 3. API 契约清单（测试即契约；`sr_pipeline::route` 的 M5 扩展面）

### 3.1 `crates/sr-pipeline` —— `pub mod route`（M5 扩展，测试假定新增；**既有 `start_route`/`start_audio_route`/`RouteConfig`/`AudioRouteConfig` 逐字不动**——M3/M4 测试以全字段字面量构造）

```rust
/// M5 多路单路配置（§3.4 RouteConfig 的多路投影）：video 必填（D6 固定画布）；
/// audio 空 = 纯视频路，非空 = 每 App 源一条独立音轨（M4 隔离模式语义）——
/// 即 REQ-12 的「视频+可选音频」。
pub struct MultiRouteConfig {
    pub target: RouteTarget,                            // 既有枚举（M3）
    pub video: VideoCfg,                                // 既有（M3）
    pub audio: Vec<sr_audio::wasapi::AudioSourceSpec>,  // 空 = 无音轨
    pub output_dir: PathBuf,
}

/// 批量启动多路（§3.3 `start_routes` 的管线层对应物）：**全成功或回滚**。
/// - Ok(handles)：与 cfgs **同序同长**；每路均已到达「输出开始」（§3.1 启动协议
///   逐路完成；顺序启动合法——scratch 实测 3 路 1080p 顺序启动 <1s）。
/// - Err：任一路启动失败（无效目标 → TargetInvalid；编码器/授权/环回环境缺失
///   → 对应变体）时，**已启动各路全部停止清理后**才返回 Err——测试断言 Err 后
///   10s 内无 ffmpeg 子进程残留、有效路无提升到输出目录的成品。
/// - 句柄/收尾/错误语义复用 M3/M4（request_stop / wait_finalized / segments /
///   SidecarExit 强匹配）。
pub fn start_routes(cfgs: Vec<MultiRouteConfig>) -> Result<Vec<RouteHandle>, RouteError>;
```

### 3.2 契约要点（行为层）

- **多路独立性**：每路独立 sidecar/管道组/段 pattern/成品（M3-green ROUTE_SEQ 已具备进程内唯一性）；任一路停止（request_stop）、目标关闭（优雅 EOF）、故障（SidecarExit）均不影响其它路的录制与收尾。
- **BLOCKED 分流**：`EncoderUnavailable`（§9 T1）/`ConsentDenied`（WGC）/`LoopbackUnavailable`（音频路）→ 测试按环境缺失记 BLOCKED（fail）；`TargetInvalid` 在回滚用例为**断言目标**（无效 hwnd 的地面真值，M3-green 已实现该映射）。
- **回滚的「完成」判定**：Err 返回即视为回滚已完成（测试给 10s 轮询窗口容让进程退出收敛）；未接受的批次不得在输出目录留下成品（REQ-24：成品提升=会话级提交动作；段/临时产物按保留策略可留）。
- **绿阶段实现提示（scratch 已验证的最小形态）**：逐路 `start_route_impl` 顺序启动；失败路径对已启动各路 request_stop + wait_finalized + 删除成品/段（严格回滚）。若实现选择「保留产物供恢复」的回滚语义，须与本测试作者协商（见 §8-4）。

## 4. 运行命令与门控

```text
# L2（显式执行；须交互式用户会话 + WGC + 可用编码器 + 锁定 n8.1；音频子断言另需默认渲染端点）
cargo test -p sr-itests --test multi_route -- --ignored --nocapture
# 单用例示例
cargo test -p sr-itests --test multi_route it_three_routes_stop_middle -- --ignored --nocapture

# 全仓（红阶段 exit 101 属预期：仅 multi_route 的 E0432）
cargo test --workspace
```

BLOCKED 触发点：非交互会话；锁定 ffmpeg n8.1 构建缺失（ffbin canonicalize 失败）；start_routes 报 EncoderUnavailable（§9 T1）/ConsentDenied（WGC 被拒）/LoopbackUnavailable（默认端点/进程环回不可用）。文件内 `GATE` Mutex 使 4 个 L2 用例串行（M4 R5 先例；多路用例并发叠加会超出已验证并发面）。

## 5. 依赖与锁定构建

- **零新增依赖**：multi_route.rs 仅使用 sr-itests 既有 deps（sr-pipeline/sr-audio/windows features 已含所需 Win32 模块；无 FFT——音频判定用轨道形状+RMS，内容级 FFT 属 M4 已覆盖范围，M5 不重复）。
- ffmpeg/ffprobe/ffplay：锁定 `tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin`（§8 冻结；经 `CARGO_MANIFEST_DIR` 相对定位后 canonicalize，M3/M4 同款）。测试音 WAV 用 `sine=...,volume=0.25` 合成 48k 立体声 f32（M4 R2 同款防削波）。

## 6. 红灯证据（实际执行，2026-10-16）

### 6.1 `cargo test --workspace` → **exit 101**（仅预期未解析符号；cargo 于 sr-itests 编译失败后未再调度测试目标——M3/M4 红阶段同现象）

```text
error[E0432]: unresolved imports `sr_pipeline::route::start_routes`, `sr_pipeline::route::MultiRouteConfig`
  --> tests\sr-itests\tests\multi_route.rs:53:5
   |
53 |     start_routes, Finalized, MultiRouteConfig, RouteError, RouteHandle, RouteTarget, VideoCfg,
error: could not compile `sr-itests` (test "multi_route") due to 1 previous error
```

### 6.2 定向命令（exit 101，同一 E0432；rustc 仅附「相似名」建议——`start_route`/`AudioRouteConfig` 恰证明命名沿既有先例）

```text
cargo test -p sr-itests --test multi_route --no-run
error[E0432]: unresolved imports `sr_pipeline::route::start_routes`, `sr_pipeline::route::MultiRouteConfig`
  --> tests\sr-itests\tests\multi_route.rs:53:5
error: could not compile `sr-itests` (test "multi_route") due to 1 previous error
```

**红因确认**：全部错误 = 新测试引用的预期公共符号 unresolved（实现缺失），无任何其它类别错误，新测试代码 0 警告（ws 副本全量编译证实，§6.5）。

### 6.3 既有绿不退化

| 命令 | 结果 |
|---|---|
| `cargo test -p sr-core -p sr-enum -p sr-capture -p sr-audio -p sr-pipeline -p sr-shot -p screen-rec` | **exit 0**：screen_rec_lib 7、channel 1、pacer 1、silence 1、drift 1、args 1、naming 1 全绿 |
| `cargo test -p sr-itests --test route --test audio_isolation --test enum_targets --test enum_audio --test shot --no-run` | Finished、exit 0（M1–M4 全部 L2 目标可编译，无回归） |

### 6.4 时序敏感 helper 双层自检（M3 §9.4 / M4 §6.4–6.5 方法论；scratch 位于仓库外 `%TEMP%\opencode\sr-m5-red-scratch`，不进仓库）

**第 1 层（实跑）**——`helper/`（独立 bin，用既有 start_route/start_audio_route 演练）先以 [diag] 单路录制 + 原生分辨率帧块图取证，发现并定位了三个问题（对红测试定型起决定作用）：

1. **生产缺陷 A（M3 遗留，仓库仍在，本红测试将捕获）**：`crates/sr-capture/src/canvas.rs` 的 `bgra_to_nv12` 把 Cb/Cr 按**平面布局**（Cb 全平面 + Cr 全平面 = I420 形状）写入 **NV12（交错）**缓冲，而管道声明 `-pix_fmt nv12` → 解码侧色度错乱（标记块亮度保留、色度散落错位）。M3/M4 的 luma-only 断言（std/运动）不可能发现；diag 实测：灰底/亮条完好，标记色呈 2×2 平铺错位。修法（scratch `ws/` 已验证）：`uv_i = (y/2)*w + (x/2)*2; uv_plane[uv_i]=Cb; uv_plane[uv_i+1]=Cr;`。
2. **生产缺陷 B（间歇 flake 根因，仓库仍在）**：`build_encode_args` **未钉视频码率**（NVENC 默认 ~2Mbps）→ 1080p30 高运动内容下色度被间歇性压垮（实测 ~1/8 轮某路 own 标记占比 0.097→0.0000，亮度/运动/时长全部正常）。scratch 以 `-b:v 12M` 验证：**8/8 轮消除**。绿阶段须钉码率/质量模式（S2 矩阵语境）并复跑本套件。
3. **已知已修复缺陷的时间线注记**：scratch `ws/` 副本制作于 12:2x，早于 M4 AV 修复（`wgc.rs` MTA 保活，12:37 落盘，docs/test-fix/2026-10-16-m4-audio-l2-av.md）进入仓库——演练中段出现的 0xc0000005（恒定偏移、windows-rs `link!`/FactoryCache 区、`mov rax,[rax+0x70]`、WER+PE 反汇编+PDB 行号取证）即该已知缺陷在过期副本上的复现；同步当前 wgc.rs 后**零复现**（≥16 个全量轮次）。**与本红测试无关，仓库现版已含修复。**

**第 1 层（续）——ws 全量 dress rehearsal**（`ws/` = 仓库副本 + 上述两处 scratch-only 生产补丁 + `start_routes` 委托桩（逐路 start_route_impl、失败即停+收尾+删产物），跑**本红测试文件原文**）：

| 阶段 | 配置 | 结果 |
|---|---|---|
| 轮 A | chroma 修复 + 桩 v1（回滚=仅停止） | 3/4 + 回滚用例如实失败（残留成品）→ 确认严格回滚契约必要 |
| 轮 B | 桩 v2（严格回滚）+ 50% 标记 + 阈值 0.02/0.005/0.005 | 踩过期 wgc.rs 的已知 MTA AV（§6.4-3） |
| 轮 C | 同步 MTA 修复 | 5/5 全量绿 |
| 轮 D/E | 仓库文件原文（GWLP 变体）；+显式槽位 | 7/8、7/8——剩余 ~1/8 flake → 定位为缺陷 B（码率） |
| **轮 F（终态）** | **+scratch 码率补丁（-b:v 12M）** | **8/8 全量绿（37s/轮）** |

终态实测（br1 轮）：三路 own 标记占比 0.097/0.103/0.097（阈值 ≥0.02，~5× 裕度）、其它路 0.0000（≤0.005）；两路 0.097/0.103；故障幸存路 0.103；时长 Δ≤0.62s（容差 2s）；三路 10.600/6.500/10.533s vs span 10.073/6.000/10.073s；音轨 RMS 0.0216（阈值 0.005）；运动对 ≈100%；强杀后恰 1 路 SidecarExit、故障路首段 6.100s 可播。

**第 2 层（stub-编译）**——对仓库测试文件原文在 ws（真实 sr-pipeline 副本 + 委托桩）`cargo test --no-run`：**0 error 0 warning**；该层在迭代中抓到并修正测试自身缺陷 3 处（scratch 证实后回移仓库）：①`SYSTEM_PARAMETERS_UPDATE_FLAGS` 名称误写（正名 `SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS`，且 fwinini 位置类型修正）；②槽位初版辅助函数未用告警（内聚进创建闭包）；③早期 `other_max` 数值类型歧义（scratch `0.0` 需 `f64` 注记——仓库文件本就正确）。

## 7. 关键设计取舍

| # | 取舍 | 依据 |
|---|---|---|
| R1 | 每路内容身份 = 唯一标记色静态块 + 箱式判定（own ≥2%/other ≤0.5%） | REQ-12「各自独立、互不干扰」的内容级编码：目标串线/共享会话/误录显示器均逃不过（own 缺失）；构造性保证 + 对矩阵/范围漂移 ≥30 单位裕度；阈值经 8 轮实测标定（own ~0.10） |
| R2 | 故障用例对称断言（恰 1 Err/恰 1 Ok） | Toolhelp32 只给父子关系，无 sidecar→路由映射知识；若实现日后经 RouteHandle 暴露 sidecar pid 可再加强（§8-7） |
| R3 | 「停一路」用**关闭目标窗口**承载（纯视频路 B） | REQ-12 明文条款「目标窗口关闭 → 该路优雅停止并保留内容」——M1–M4 从未实测过的路径，本红首次钉死（wait_finalized=Ok+内容保留）；request_stop 路径由三路/故障用例覆盖 |
| R4 | 标记窗口显式槽位（工作区 3 等分、互不遮挡、宽度防御性收进槽内） | 演练中 CW_USEDEFAULT 级联叠放曾伴随「内容缩成 ~60% 画布+黑边」的捕获异常（WGC 对被遮挡 GDI 窗口的表面节流嫌疑，见 §8-3）；显式铺排后未再复现；兼得几何确定性 |
| R5 | 批量回滚断言 = 无 sidecar 残留（10s 轮询）+ 无提升成品 | 「全成功或回滚」的可观测完成判据；段文件保留策略不钉（REQ-22 恢复语义），「成品=已完成的录制」不得存在（REQ-24 提交语义） |
| R6 | 文件内 GATE 串行 4 用例 | M4 R5 先例；4 用例合计最多 3 路 1080p 并发，叠加并发会超出 S1/S2 已验证并发面 |
| R7 | min_frames 与 ±2s 时长包络一致（floor((span−2s)×3fps)） | 两断言不自相矛盾（时长断言放行的最短时长也须过帧数下限） |
| R8 | 音频断言 = 轨道形状（1 vs 0 轨、aac/48k/2ch）+ 去首尾 RMS ≥0.005 | 「可选音频」的独立输出语义；内容级 FFT 隔离属 M4 `it_audio_multitrack_ffprobe_fft` 已覆盖范围，M5 不重复；RMS 阈值取数字静音（~1e-4）与实测（0.022）之间（端点增益随机器浮动） |
| R9 | 三路用 1080p30、两路保底语义 1080p30、故障/回滚用 640x360 | §9 T1 保底=2 路 1080p30；三路 1080p30 有 S1 实测背书（一次性调度尖峰 ≤263ms）+ 本机 8 轮演练验证；故障/回滚语义与分辨率无关，缩短套件时长（全套 4 用例 ~37s） |
| R10 | `start_routes` 为增量 API（新 struct，不动 RouteConfig/AudioRouteConfig） | M3/M4 测试以全字段字面量构造既有 config——增字段会破坏既有测试编译（M4 R11 同款约束）；统一化重构留 §8-6 协商 |

## 8. 未决问题（返回 orchestrator / 绿阶段须知）

1. **生产缺陷 A（阻绿，必须修）**：`canvas.rs` NV12 色度交错布局错误（§6.4-1；M3 遗留、M3/M4 测试盲区）。本红 4 用例的内容级断言将在绿阶段如实失败直至修复。修法已在 scratch 验证（一行索引修正）。
2. **生产缺陷 B（阻绿稳定性，必须修）**：编码命令未钉视频码率 → 1080p 色度间歇压垮（§6.4-2；~1/8 轮）。绿阶段须钉码率或质量参数（S2 矩阵语境下标定；scratch 用 12M 验证）并复跑本套件多轮。
3. **WGC 对被遮挡 GDI 窗口的表面节流观察**（内容缩成 ~60% 画布+黑边，含窗口标题；演练轮 D/E 各见 1 次）：测试侧已用显式槽位规避；根因与恢复语义属 REQ-13（遮挡/表面重建）范围，绿阶段不强制处理，记录备考。
4. **回滚产物的契约口径**：本红钉「未获批次接受的路不得在输出目录留下成品」（REQ-24 提交语义推论）。若实现倾向「回滚保留产物供恢复」需与本测试作者协商改断言（如改为「不得有正在进行的录制 + 产物仅限临时目录」）。
5. **目标窗口关闭 + 音频路同在**的语义未钉（本红以纯视频路承载窗口关闭条款）：带活动音轨的路在目标窗口关闭后应整路优雅收尾（音轨一并 EOF）——合理的 REQ-12 语义延伸，留 M6/M8 或绿阶段协商补测。
6. **MultiRouteConfig 与 §3.4 统一 RouteConfig 的收敛**：红测试新增第 3 个 config 结构（M3 RouteConfig/M4 AudioRouteConfig/M5 MultiRouteConfig）。绿阶段可重构为统一形态（如 video: Option + audio plan），但须保持 M3/M4/M5 三套测试零改动可编译（全字段字面量约束）——与 m3-red §8-4 / m4-red §8-5 同款协商点。
7. **sidecar→路由映射未暴露**：故障用例因此只能对称断言；若 RouteHandle 暴露 sidecar pid（m3-red §8-7 同款契约扩张），可加强为「精确停指定路的 sidecar」。
8. **边界未钉**：空 cfgs 的 start_routes（本红不构造）；单元素批次（平凡情形）；Monitor/Region 目标的多路（本红全用 Window——Monitor 多路属 REQ-2/13 交叉，Region 未实现）。
9. **阈值标定环境**：标记/运动/RMS 阈值基于本机（build 26200.9711 + NVENC）8 轮实测，own ~0.10 vs 阈 0.02（5× 裕度）；换编码器链（QSV/AMF 软件兜底）或极端主题下若逼近阈值，凭实测证据调整并记录（不放宽断言语义）。
10. **时间预算未断言**（沿 M3/M4 先例）：启动时延（3 路 1080p 顺序启动实测 <1s）、停止响应 ≤2s、收尾时长无显式断言；wait_finalized 120s 超时隐含宽松上限。
