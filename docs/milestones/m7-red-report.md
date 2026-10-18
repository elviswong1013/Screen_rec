# M7 红测试报告（TDD 红阶段：合并/REQ-23）

- 日期: 2026-10-17
- 规格: specs/001-screen-recorder.md §2.5（REQ-23 全文：≥2 产物→单 MKV/`-map 0 -map 1 … -c copy` 全轨保留/多视频轨并存/异构允许不得误拒/不得覆盖输入/不用 `-shortest`/各文件按本地零点合并/轨道命名与默认选择规则/取消-失败-磁盘满保留输入）、§3.2（crates/sr-merge = 合并（异构多轨）/REQ-23）、§3.3（`merge_recordings(Vec<PathBuf>, out)` 命令）、§3.4（E_MERGE_INCOMPATIBLE「仅不可读/未被 MKV 接纳」）、§3.5（合并输入拒绝行）、§4（REQ-23 行）、§5 M7 行、§8 M0 冻结项（锁定 ffmpeg n8.1 release）
- 先例: docs/milestones/{m4,m5,m6}-red-report.md（契约/门/双层自检方法论）；tests/sr-itests/tests/{multi_route,audio_m6}.rs 与 common/mod.rs（helper 形态、`#[ignore]` 门、BLOCKED=fail、GATE 串行）
- 角色: **executor-backup 承担 Test Writer（红阶段）**（GLM 系 test-writer 通道额度不可用）。本任务**只写测试**：生产 src 零改动（crates/* 全部逐字未动，含 sr-merge 骨架）；未 git 提交。
- 环境实测: Windows 11 25H2，`Microsoft Windows [Version 10.0.26200.9711]`；cargo/rustc **1.99.0** msvc；ffmpeg/ffprobe = 锁定 `tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin`（§8 冻结）。仓库无 git 提交历史（全部 untracked，与 M1–M6 一致）。
- 会话注记: 工具 shell 不继承 `SESSIONNAME`；L2 实跑需 `$env:SESSIONNAME='Console'`（沿红 §8-9/M6 注记）。merge 本身无桌面/采集依赖，但按 §4「L2=交互式会话」通则仍校验（见 §4）。
- 到场状态: 本次改动前基线 `cargo test --workspace` = **exit 0**（日志 `%TEMP%\opencode\m7_baseline_ws.log`）。
- 编排者任务注记: 任务示例写 `sr_pipeline::merge::{…}`；按 spec §3.2 模块表与既有骨架，契约落 **`crates/sr-merge`**（骨架 `pub mod merge` 的 TODO(M7) 已指名本用例）——差异已在 §7-R1/§9-1 报备。

## 1. 交付物

| 文件 | 性质 | 内容 |
|---|---|---|
| `tests/sr-itests/tests/merge.rs` | 新增（25,601 字节） | L2 ×1：`it_merge_heterogeneous_multivideo_mkv`（§4 点名；正向异构合并 + 负相位非法/缺失输入/out==输入），`#[ignore]` 门 + BLOCKED=fail + 文件内 GATE |
| `tests/sr-itests/Cargo.toml` | 修改（+2 行） | 加 `sr-merge = { path = "../../crates/sr-merge" }`（**workspace 内 path 依赖**；测试侧契约目标，零外部新依赖） |
| `Cargo.lock` | 自动更新 | 仅 sr-itests 依赖表新增 `"sr-merge"`（无新增外部包） |
| `docs/milestones/m7-red-report.md` | 新增 | 本报告 |

未改动：生产 src（含 `crates/sr-merge/src/lib.rs` 骨架：`pub mod inputs`/`pub mod merge` 保持 TODO 空壳）、`specs/`、`docs/spikes/`、既有测试文件与 `common/mod.rs`（哈希核验见 §6.7；本文件 helper 全部内聚在 merge.rs，沿 M6 先例）。

## 2. REQ → 测试 → 文件映射（§4 命名优先，无自创用例名）

| REQ | 测试名 | 层 | 断言要点 |
|---|---|---|---|
| REQ-23 合并 | `it_merge_heterogeneous_multivideo_mkv`（§4 点名；「不同分辨率/帧率/长度」） | L2 | 合成异构输入（A=2 视频轨+1 音轨/640x360@30+320x180@15/48kHz2ch/≈3.0s；B=1 视频轨+2 音轨/1280x720@25/44.1kHz2ch+48kHz1ch/≈2.0s）→ `merge_recordings`：**异构不得误拒**；`MergeOutcome{out,video_tracks=3,audio_tracks=3}`；输出=单个可播 MKV（可解码、6 轨）；逐轨与源一一对应（codec/尺寸/帧率/采样率/声道/包终点 ±0.35s；起点≈0=[各文件按本地零点]）；时长≈max 输入（±0.5s）且 >min（**非 `-shortest`**、非顺序拼接）；默认轨 ≤1/类型；输入两文件逐字节未被修改/覆盖；负相位：非 MKV 输入/不存在输入 → `Err(InvalidInput)`、out==输入 → `Err(OutputUnavailable)`，均**不提交成品且输入保留** |

## 3. API 契约清单（测试即契约；供绿阶段实现）

### 3.1 `crates/sr-merge` —— `pub mod merge`（新增符号；骨架 TODO(M7) 落地）

```rust
/// REQ-23：≥2 产物 → 单 MKV；`-map 0 -map 1 … -c copy` 全轨保留（多视频轨并存）；
/// **异构（分辨率/帧率/时长/轨数不同）允许且不得误拒**；不得覆盖输入；
/// 不用 `-shortest`（保留完整时长）；各文件按本地零点合并；失败不提交（输入保留）。
pub fn merge_recordings(cfg: MergeConfig) -> Result<MergeOutcome, MergeError>;

#[derive(Clone, Debug)]
pub struct MergeConfig {
    pub inputs: Vec<PathBuf>, // ≥2（<2 行为未断言，见 §8-4）
    pub out: PathBuf,         // 不得与任一输入同路径
}

#[derive(Clone, Debug)]
pub struct MergeOutcome {
    pub out: PathBuf,          // == cfg.out
    pub video_tracks: usize,   // 成品视频轨总数（各输入之和；多视频轨并存）
    pub audio_tracks: usize,   // 成品音频轨总数（各输入之和）
}

#[derive(Debug)]
pub enum MergeError {
    /// E_MERGE_INCOMPATIBLE（§3.4「仅不可读/未被 MKV 接纳」）：不存在/不可读/空/
    /// 非 Matroska 接纳的输入。**异构布局不属于此错误。**
    InvalidInput { input: PathBuf, detail: String },
    /// out 不可安全创建/提交：与任一输入同路径（禁止覆盖输入；本红钉此分支）、
    /// 父目录不存在/不可写、已存在且无法安全提交等。
    OutputUnavailable { out: PathBuf, detail: String },
    /// ffmpeg 执行失败（非零退出；stderr 摘要）。
    SidecarFailed { detail: String },
}
```

- Tauri 命令面（§3.3 `merge_recordings(Vec<PathBuf>, out)`；进度经 Channel）由 `src-tauri` 后续实现，输出面映射到本 API；本红不钉命令层。
- `sr_merge::inputs`（骨架）保留为实现自由度：前置校验可落在其中或 merge 内部，本红不钉符号形态。

### 3.2 行为契约要点

- **异构不误拒**：两输入的分辨率/帧率/时长/轨数全不同且多视频轨并存——必须 `Ok`；校验只接受「不可读/非 MKV 接纳」类拒绝（§3.4/§3.5）。
- **全轨保留**：输出轨数 = 各输入轨数之和（3 视频 + 3 音频）；逐轨 codec/尺寸/采样率/声道随源（`-c copy`）。
- **时长口径**：输出容器时长 ≈ max(输入时长)（±0.5s）。`-shortest`（≈min）与顺序拼接（≈Σ）均在 ≤0.5s 容差外（差 ≥1.0s/≥2.0s）。
- **本地零点**：同一输入的各轨并入后起点≈0（容差见 §3.3）；次输入的轨不得整体后移约「首个输入时长」。
- **输入不可变**：成功与失败路径都不得修改/覆盖/删除输入（测试以逐字节比较复核）；`out` 与输入同路径必须拒绝。
- **失败不提交**：`Err` 时 `out` 不得存在（实现建议临时名+验证后更名；REQ-24 安全提交同精神——本红已钉，见 §7-R7）。
- **默认选择**：至多一个默认视频轨、一个默认音频轨（无歧义下限；具体哪条/命名规则未钉，见 §8-5）。
- **BLOCKED 分流**：锁定 n8.1 构建缺失/登录会话不合格（SESSIONNAME）→ BLOCKED；合成失败（构建能力缺失）→ BLOCKED（fail，不静默 skip）。

### 3.3 测试内判定口径（构造性保证 + scratch 实测）

| 口径 | 值/方法 | 依据（§6.5 实测） |
|---|---|---|
| 容器时长容差 | ±0.5s vs max 输入 | 真值合并 3.029 vs max 3.021（Δ0.008）；`-shortest` 2.020 vs 3.021（Δ1.001） |
| 逐轨终点容差 | ±0.35s | 真值合并与源轨逐包终点差 ≤0.01s；`-shortest` 下 A 轨差 1.0s |
| 轨起点区间 | [-0.2, 0.5]s | AAC priming 首包 ≈−0.021s、视频 0.000s；顺序拼接的次输入轨 >2.9s |
| 轨匹配 | 属性键多重集合（type/codec/尺寸/帧率/采样率/声道） | 6 条合成轨键两两唯一；不钉输出轨序与命名 |
| 输入不可变 | 全文件逐字节 `fs::read` 前后比较 | 文件 0.8–1.4MB 量级，无外部 hash 依赖 |

## 4. 运行命令与门控

```text
# L2（显式执行；须交互式用户会话；$env:SESSIONNAME='Console' 需可见）
cargo test -p sr-itests --test merge -- --ignored --nocapture
# 单用例示例
cargo test -p sr-itests --test merge it_merge_heterogeneous_multivideo_mkv -- --ignored --nocapture

# 全仓（红阶段 exit 101 属预期：仅 merge 的新符号 unresolved）
cargo test --workspace
```

BLOCKED 触发点：非交互会话（SESSIONNAME 缺失/非 Console）；锁定 n8.1 构建缺失/不可 canonicalize；合成命令失败（锁定构建缺 lavfi/testsrc2/sine/mpeg4/aac 能力——§6.5 已实测具备）。文件内 `GATE` Mutex 串行（单用例同样持门：与录制族 L2 同机不混跑；后续新增用例沿用）。运行本身不需要 WGC/编码器/音频设备（REQ-23 为 `-c copy` 语义）。

## 5. 依赖与锁定构建

- **零外部新依赖**：唯一依赖变化 = `tests/sr-itests/Cargo.toml` 增加 workspace 内 `sr-merge` path 依赖（契约 crate；§3.2 模块表），`Cargo.lock` 仅新增该依赖边（无新外部包）。merge.rs 仅用 std + common（无 windows/FFT 等既有依赖面）。
- 输入合成与交叉验证全部走**锁定 n8.1 release 构建**：`testsrc2/testsrc`（视频）× `sine`（音频）经原生 `mpeg4`/`aac` 编码为 MKV（无需 mediafoundation/NVENC 等 D4 链，也不依赖采集环境）；ffprobe 逐轨 + 包级时间 + 整文件解码。

## 6. 红证据（实际执行；全部日志在 `%TEMP%\opencode\`）

### 6.1 改动前基线（对照）

| 命令 | 结果 |
|---|---|
| `cargo test --workspace` | **exit 0**（`m7_baseline_ws.log`） |

### 6.2 定向红（新测试目标）

```text
cargo test -p sr-itests --test merge --no-run        → exit 101（m7_red_targeted_final.log）

error[E0432]: unresolved imports `sr_merge::merge::merge_recordings`, `sr_merge::merge::MergeConfig`,
             `sr_merge::merge::MergeError`, `sr_merge::merge::MergeOutcome`   （merge.rs:46）
error: could not compile `sr-itests` (test "merge") due to 1 previous error
```

**红因确认**：恰 1 个 error，全部为「新测试引用的预期公共符号 unresolved（实现缺失）」；无其它类别错误；新测试代码 **0 警告**（初版 `track_from_map` 带未用参数产生 1 条 `unused_variables` 警告——已修，见 §6.8）。

### 6.3 工作区红

| 命令 | 结果 |
|---|---|
| `cargo test --workspace` | **exit 101**，同一错误集（`m7_red_ws_final.log`） |

### 6.4 既有绿不退化

| 命令 | 结果 |
|---|---|
| `cargo test --workspace --exclude sr-itests` | **exit 0**：共 **13 passed / 0 failed**（`m7_l1_regress.log`） |
| `cargo test -p sr-itests --test route --test audio_isolation --test multi_route --test enum_targets --test enum_audio --test shot --test audio_m6 --no-run` | Finished、**exit 0**（M1–M6 全部 L2 目标可编译；`m7_l2_compile_regress.log`） |

### 6.5 Scratch 第 1 层：ffmpeg ground truth（仓库外 `%TEMP%\opencode\m7-scratch`；日志 `m7_scratch_layer1.log`）

| 检查 | 实测 |
|---|---|
| 合成 A（`testsrc2 640x360@30` + `testsrc 320x180@15` + `sine 440Hz/48k` → `mpeg4/aac` MKV） | exit 0；781,423 B；ffprobe：2 video（640x360@30 / 320x180@15）+ 1 audio（48k/2ch）；时长 3.021s |
| 合成 B（`testsrc2 1280x720@25` + `sine 880Hz/44.1k` + `sine 1200Hz/48k` → `mpeg4/aac`；`-ac:a:0 2 -ac:a:1 1`） | exit 0；1,384,627 B；1 video（1280x720@25）+ 2 audio（44.1k/2ch、48k/1ch）；时长 2.023s |
| 真值合并 `-map 0 -map 1 -c copy` | exit 0；6 轨（3v+3a）；时长 **3.029s**（≈max 3.021） |
| `-shortest` 变异 | 6 轨；时长 **2.020s**（≈min）⇒「非 -shortest」断言可判别 |
| 逐包终点（pts+dur） | 输入 A 轨 3.000/2.999/3.008s、B 轨 2.000/2.020/2.005s；真值合并后逐轨一致（≤0.01s 差） |
| 首包 pts | 视频 0.000s；AAC −0.021～−0.023s（priming）⇒ 起点容差区间 [-0.2, 0.5]s |
| 整文件解码 `-f null -` | exit 0（真值合并产物可播） |

### 6.6 Scratch 第 2 层：dress rehearsal（仓库外副本 `%TEMP%\opencode\sr-m7-red-ws`；ffmpeg 以 junction 挂入，target/.git 不入副本）

对**测试文件原文**（SHA256 与仓库一致：`B404C59B…EDF0E`，见 §6.7）在副本内加 scratch-only 假实现（`crates/sr-merge`：EBML 校验 + `-map N… -c copy` + ffprobe 计数）后实跑（`$env:SESSIONNAME='Console'`；首次全量编译 57.35s）：

| 轮次 | scratch 行为 | 结果 |
|---|---|---|
| 假绿 | 契约忠实实现（照 §3 语义） | **PASS（exit 0；0.89–0.90s）**：3v+3a、逐轨起点/终点与源一致（起点 0.000/−0.021/−0.023s；终点 3.000/2.999/3.008/2.000/2.020/2.005s）、时长 3.029 vs max 3.021、负相位三例全过（`m7_rehearsal_green.log` / `m7_rehearsal_green_final.log`） |
| 变异 M1 | 加 `-shortest` | **FAIL（exit 101）**：逐轨终点断言（merge.rs:462；输出 A-640x360 轨终点 2.000 vs 源 3.000）（`m7_rehearsal_mutant_shortest.log`） |
| 变异 M2 | `-map` 只取各输入首视频/首音频 | **FAIL（exit 101）**：轨布局断言（merge.rs:421；2v+2a ≠ 3v+3a）（`m7_rehearsal_mutant_partialmap.log`） |
| 变异 M3 | 去掉 EBML 接纳校验（非法输入直送 ffmpeg） | **FAIL（exit 101）**：负相位变体断言（merge.rs:506；得到 `SidecarFailed` 而非 `InvalidInput`）（`m7_rehearsal_mutant_novalidate.log`） |

⇒ 正向三断言组与负相位均有独立判别力；假绿与仓库红因（符号缺失）互证。

### 6.7 最终冻结复核与哈希

复跑（最终文件未再改动）：定向红 `exit 101`、工作区红 `exit 101`、同 1 error / 0 警告（`m7_red_targeted_final.log`、`m7_red_ws_final.log`）。

| 文件 | SHA256 | 状态 |
|---|---|---|
| `tests/sr-itests/tests/merge.rs` | `B404C59BD1B4022E7C7FF05E3D9EDBF543CAF7C0E4337EB2332D2B834AAEDF0E`（前 16：`B404C59BD1B4022E`） | **新增**（M7 基线） |
| `tests/sr-itests/Cargo.toml` | `5D339849C49ADAEFC1C94E294FD30859A756857F6002D0E0D108D540951D8018` | 修改（+sr-merge path dep） |
| `Cargo.lock` | `D6817AB08183611BDAC7AD9B2DA0531364F0F7486971238393F25D2F97A2CE4B` | 自动更新（+1 依赖边） |
| `tests/sr-itests/tests/multi_route.rs` | `56577577AE55945E…DECBE4BEE1`（M6 加固后） | 未动 |
| `tests/sr-itests/tests/audio_m6.rs` | `7D62C9ACCEAE4F1D…5254D1B10` | 未动 |
| `tests/sr-itests/tests/audio_isolation.rs` | `B33A566F1CCA2580…4C769627` | 未动 |
| `tests/sr-itests/tests/route.rs` | `D21AB7A43AD9B3DD…ECBE031D` | 未动 |
| `tests/sr-itests/tests/common/mod.rs` | `1258527C8131992F…6177CBA8` | 未动 |
| `tests/sr-itests/tests/shot.rs` / `enum_audio.rs` / `enum_targets.rs` | `7F1C581B…` / `9D75F46D…` / `FE1DF55D…` | 未动 |

生产 `crates/` 与 `src-tauri/` 零改动（mtime 扫描：本会话在仓库内仅触碰 merge.rs、Cargo.toml、Cargo.lock、本报告）。

### 6.8 测试侧自查修复

`track_from_map` 初版带未使用的 `path` 参数 → 1 条 `unused_variables` 警告；已删参收敛为「仅预期符号 unresolved、0 警告」的红因口径（§6.2）。

## 7. 关键设计取舍

| # | 取舍 | 依据 |
|---|---|---|
| R1 | 契约落 `crates/sr-merge::merge`（非任务示例的 `sr_pipeline::merge`） | spec §3.2 模块表（sr-merge=合并（异构多轨）/REQ-23）+ 既有骨架 `pub mod merge` TODO(M7) 指名本用例；见 §9-1 报备 |
| R2 | 负相位（非法/缺失输入、out==输入）并入 §4 点名用例内，不新增用例名 | 任务约束「不要自创超出 spec 的用例名」；断言按相位分节、失败可定位（§6.6 变异证实） |
| R3 | 输入用锁定构建 lavfi+原生 mpeg4/aac 合成（非真实采集产物） | 任务明确「合成异构输入」；聚焦 REQ-23 `-c copy` 语义，环境面最小（无 WGC/编码器链/音频设备） |
| R4 | 逐轨时长以**包终点**（pts+dur）判定 | MKV 无每轨 duration 头字段（ffprobe stream.duration 缺失，§6.5 实测）；包终点与源轨差 ≤0.01s |
| R5 | 轨匹配用属性键多重集合，不钉输出轨序/命名 | REQ-23 未定义输出轨序；避免过拟合 ffmpeg 映射顺序（真值恰为「输入序分组」，但不断言） |
| R6 | 默认轨仅钉「≤1/类型」 | REQ-23「默认选择规则明确」的最小可测下限；具体默认轨归属/命名属实现文档面（§9-5） |
| R7 | 钉「失败不提交（out 不存在）」 | REQ-24 安全提交（临时名→验证→rename）同精神；防失败残留半成品伪装成品；M5 回滚用例「无成品」先例 |
| R8 | scratch 双层自检：第 1 层 ffmpeg ground truth + 第 2 层假绿/3 变异 | M6 先例；证明断言组判别力（`-shortest`/部分 map/无校验三种缺陷均被独立断言拦截） |

## 8. 未决问题（返回 orchestrator；绿阶段须知）

1. **契约 crate 归属**：任务示例 `sr_pipeline::merge` vs spec §3.2 `crates/sr-merge`（骨架 TODO(M7) 已存在）。本红按 spec 落 `sr_merge::merge`；若编排者坚持管线层命名，需先修订 spec/骨架归属（不建议：会留死骨架）。
2. **`tests/sr-itests/Cargo.toml`/`Cargo.lock` 变更**：新增 workspace 内 path 依赖是引用契约 crate 的前置条件（零外部新依赖）；如「零 manifest 改动」为硬约束，需改契约归属到既有依赖 crate（同 §8-1）。
3. **取消（cancel）路径未覆盖**：§3.3 的 `merge_recordings(Vec<PathBuf>, out)` 为同步签名、无取消参数；「取消保留输入」在管线层的形态（句柄/取消 token/进程终止）未定义——本红不钉，建议绿阶段明确（或与编排者确认后扩展契约）；磁盘满同理（L2 难造满盘，失败保留输入已由非法/缺失输入覆盖）。
4. **单输入（<2）行为未断言**：契约标注 `inputs ≥2` 为前置，但 <2 的精确行为（拒绝变体/边界）未测；绿阶段可自行定并补文档，勿静默成功。
5. **轨道命名/默认选择规则未钉**：仅「≤1 默认/类型」无歧义下限；REQ-23 要求「规则明确」，建议绿阶段在实现/报告中写明命名与默认策略（如按输入序）。
6. **输出路径已存在（非输入）时行为未定义**：覆盖/拒绝未指定；本红只钉「不得覆盖输入」。建议绿阶段明确（安全提交语义建议拒绝或安全替换）。
7. **跨输入轨对齐/会话时间偏移**：v1 明确不启用（spec §6）；本红钉「起点≈0」。若未来启用偏移属契约扩展。
8. **负相位与 L2 层的错位**：合并为纯 ffmpeg 操作，无设备依赖；仍按 §4 L2 通则校验交互会话（`require_interactive_session`）。若编排者希望 merge 用例可在 Session 0 运行（如 CI），需在 spec 层为「无设备 L2」开例外。
9. **SESSIONNAME 运行注记**：本工具链 shell 不继承；L2 实跑需 `$env:SESSIONNAME='Console'`（否则 BLOCKED，非测试缺陷）。
10. **时间预算未断言**（沿 M3–M6 先例）：`merge_recordings` 为同步调用、测试无超时断言（假绿实测 0.9s 量级；合成/探测/env 开销合计）；如需「合并完成时限」承诺需 spec 层定义。

## 9. 结论

- M7 红测试就位：§4 点名的 `it_merge_heterogeneous_multivideo_mkv`（L2，`#[ignore]` 门 + BLOCKED=fail + GATE 串行）覆盖 REQ-23 全部可测语义面（异构不误拒/全轨保留含多视频轨/逐轨保真/非 `-shortest`/本地零点/输入不可变/失败不提交与错误变体）。
- 新 API 契约 (§3) 按 spec §3.2/§3.3 拟定并钉死行为语义；错误变体最小集 3 个。
- 红因正确：`cargo test --workspace`（及定向命令）**exit 101，仅 1 个预期 E0432（4 个未解析符号）、0 警告**（§6.2/§6.3/§6.7）；既有 L1 全绿（13 passed）、M1–M6 L2 目标全部可编译（§6.4）。
- 双层自检 + 判别力：ffmpeg ground truth（§6.5）+ 假绿 PASS 与 3 变异分别被独立断言拦截（§6.6）。
- 未写任何生产代码；未 git 提交；报告数字全部来自本机实跑日志（路径已列）。`merge.rs` 新基线 SHA256 前 16 = **`B404C59BD1B4022E`**。
