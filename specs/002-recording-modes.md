# 002 - Screen_rec 录制模式扩展（SDD）

Status: Draft
Created: 2026-10-18
Updated: 2026-10-18（plan-reviewer R1：三率域/基础比率/测试覆盖/serde 与 UI 契约；R2：分源捕获策略、实际率传递与计数域、四档测试、JSON 边界容错、查询契约定稿）
PRD: `docs/prd/002-recording-modes.md`（Approved 2026-10-18）

## 1. 范围（本版本）

本版本仅详规 **P1 采样率可选**（PRD AC-1..3）。P2–P5 见 §7 占位：各期开始前补充详规并重新过 plan-reviewer 与用户批准。

**对 001 spec 条款的保留/覆盖声明**：
- 001 REQ-14/18（音轨身份与隔离）、REQ-19（纯录音）、REQ-26（漂移核语义：校正阈值、Hold/Track 行为）全部**保留**，仅采样率数值域参数化。
- 001 中 `/48000` 字面值的归属按本版 §3 REQ-7 的计数域规则区分：输入计数域跟随**实际捕获率**（不再固定 48000），输出域跟随设定值；既有测试断言（如 sync_dual 固定断言 `aac,48000,2`）在默认 48k 基线下保持不变，非 48k 由本 spec 新增参数化用例覆盖。

## 2. 三个采样率域（术语定稿）

| 域 | 定义 | 取值来源 |
|---|---|---|
| **引擎混音率** | 音频引擎对设备混音后的实际采样率 | 渲染/采集设备 `GetMixFormat`；**进程环回不可查询**（wasapi 0.26 限制，`crates/sr-audio/src/wasapi.rs:388-389`），跟随目标进程所在引擎 |
| **捕获率** | 采集核实际协商到的格式采样率 | 分源策略见 REQ-7；**启动握手随 `StreamMsg::Ready` 上报实际值**（REQ-8），路由不得假设 |
| **输出率** | 成品音轨采样率 | 用户设置（REQ-1），默认 48000；重采样完成 捕获率→输出率 |

## 3. 需求（REQ）

- **REQ-1（AC-1）设置项**：`audio_sample_rate: u32` ∈ {44100, 48000, 88200, 96000}，默认 48000。
  - 缺字段：具名默认函数（参照 `default_fps` 模式，`src-tauri/src/settings.rs:103-107`），`Settings::default()` 同步。
  - **字段级容错反序列化**：负数/字符串/null/溢出等坏类型只把本字段降级为默认，**不得**重置整份设置（修复现状 `settings.rs:100-107,189-195` 的整体重置）；磁盘加载与 `set_settings`（`src-tauri/src/lib.rs:38-59`）复用同一归一化函数。
  - 录制启动读取一次设置快照；**在录路由不随保存变更**。
- **REQ-2（AC-1）设置页 UI**：
  - 下拉四档 + 原生率只读展示 + 升采样提示（所选 > 原生率：「升采样（不提升真实音质）」）。
  - 原生率来源如实标注：渲染端点 / 麦克风采集设备 = 设备混音率；进程环回 = 「跟随引擎混音率（通常 48k）」，不虚构单一原生率。
  - 查询命令定稿：`get_endpoint_mix_rate(device_id: Option<String>) -> Option<{ rate: u32, kind: "render"|"capture", device_name: String }>`；MTA 线程执行；注册 invoke_handler；与 `probe_capabilities` 失败隔离；`None` → UI 显示「未知」，**未知值不得用于推导重采样比率**；UI 查询失败与捕获初始化自身查询失败分开处理。
  - 刷新时机：设置页打开时 + 麦克风设备选择变更时（`ui/app.js:917-920,945`）。
  - `ui/api.js` real/mock 双接线显式透传新命令返回（独立于 `normalizeProbe` 的字段挑选）；设置控件由 `ui/app.js` 动态生成（`index.html` 仅为挂载点）。
- **REQ-3（AC-2）输出率生效**：
  - MKV 内 AAC：sidecar 输入 `-ar` 参数化（现状硬编码 48000，`crates/sr-pipeline/src/sidecar.rs:25-29,57-90`）。
  - 纯录音 WAV：写线程接入重采样（现状直写捕获样本，`crates/sr-pipeline/src/route.rs:594-598,728-759`；`crates/sr-audio/src/wav.rs:85-94` 接受采样率参数，头部单测不构成透传证据）。
  - 重采样走 DriftResampler 扩展接口（REQ-6）；捕获率==输出率且 ppm==0 时保持现有恒等直通（不改变 48k 基线行为）。
- **REQ-4（AC-2）验证口径**：ffprobe 断言成品**所有音轨**采样率 + 时长 + 已知音调 FFT 检出（防止"改标签未转换"假通过）。
- **REQ-5（AC-3）回归**：48k 基线回归全绿（route / multi_route / audio_isolation / recovery / probe / merge / m11_route_commit / m11_audio_only_recovery / multi_route_audio_dual / **audio_m6** / sync_dual）；44.1k 与 96k 各补 isolation、recovery、内容同步子集。L2 逐套件 `-- --ignored --nocapture --test-threads=1`，`SESSIONNAME=Console`；环境缺失记 BLOCKED，**不算通过**。
- **REQ-6（实现基础）DriftResampler 基础比率**：
  - 接口扩展 `new(channels, in_rate, out_rate)`；`base = out_rate / in_rate`；漂移叠加 `ratio = base / (1 + ppm/1e6)`（ppm 限幅 ±1000 不变，是相对基础比率的微调）。
  - `base != 1` 时 0ppm **不得**直通；补齐静音、延迟记账、停止 flush 覆盖转换路径。
  - rubato 5.0.1 支持任意正初始比率，无新依赖。
  - 措辞修正：96k 是 48k 整数二倍；**44.1k/88.2k 才是非整数路径**。
- **REQ-7（AC-1/3）分源捕获格式策略**：
  - **System（设备环回）/ Mic（采集设备）**：查询设备 `GetMixFormat` → 请求其原生**采样率**（**仅采样率原生**，继续请求 f32/双声道——现解码固定双声道四字节，`wasapi.rs:421,464-471`，原生声道/位深会越界）；查询失败回退 48000/f32/双声道 + autoconvert（现状行为）。
  - **Process（进程环回）**：固定请求 48000/f32/双声道 + autoconvert（GetMixFormat 不可用）。
  - 捕获初始化查询失败 → 回退 48000 并记日志，不阻断录制。
- **REQ-8（AC-3）实际捕获率传递与计数域**：
  - `StreamMsg::Ready` 携带实际捕获格式 `{rate, channels}`（现状不携带，`wasapi.rs:671-681,715-717`）；路由据此构造 `DriftResampler::new(channels, capture_rate, out_rate)`。
  - 输入计数（Pacer / `device_frames`）在**捕获率域**；漂移比较统一换算到秒（帧数 ÷ 各自率），消除固定 `/48.0`（`crates/sr-pipeline/src/route.rs:1371-1372,1393-1407`）；管道/WAV/AAC 载荷在输出率域。
  - 同步捕获路径（`wasapi.rs:529,559-565` 固定 48k 目标帧数/返回率）随本里程碑参数化。
  - 设置快照 → `start_routes` / `start_audio_only` / 托盘 `start_from_tray`（`src-tauri/src/commands.rs:720-723,743-750,849`）均携带输出率。

## 4. 测试计划（REQ → 测试映射）

| REQ | 测试 | 层 |
|---|---|---|
| REQ-1 | `unit_settings_sample_rate_*`：真实 JSON 边界（缺字段/负数/字符串/null/溢出→48000 且其余字段保留）；磁盘与 set_settings 两入口同归一化；四档往返；启动快照不随保存变更 | L1 |
| REQ-6 | `unit_resample_base_ratio_*`（具名一组，**明示输入/输出率对**）：96k→48k（降采样）、48k→96k、48k→44.1k、96k→44.1k；输出帧数、分块连续性、±ppm/恢复 Hold、延迟记账、停止 flush；base=1 时 0ppm 恒等直通不变 | L1 |
| REQ-7 | `unit_capture_format_strategy_*`（具名一组，**查询结果可注入**）：System/Mic 查询成功→按原生率请求；查询失败→48000+autoconvert 回退；Process→不查询固定 48000；`StreamMsg::Ready` 携带实际捕获格式 {rate, channels} 传递至调用方（`crates/sr-audio/src/wasapi.rs:386-422,715-754` 边界）；同步捕获原生率的帧数/返回率 | L1 |
| REQ-8 | `unit_drift_mixed_rates_no_false_correction`：捕获 96k / 输出 48k 稳态输入 → 漂移 corrections ≈ 0（不产生虚假校正） | L1 |
| REQ-3/4 | `it_route_sample_rate_{44100,88200,96000}`：录制 ~10s → ffprobe **全音轨**采样率 + 时长 + 已知音调 FFT（`#[ignore]`，串行）；48k 基线既有用例保持 | L2 |
| REQ-3/4 | `it_wav_sample_rate_{44100,88200,96000}`：纯录音 WAV ffprobe 实际率 + 已知音调 FFT + 时长一致（**帧数 ÷ 率 = 时长**） | L2 |
| REQ-5 | 48k 基线回归清单（REQ-5 全列，含 audio_m6）；44.1k/96k isolation+recovery+内容同步子集；sync_dual 参数化：保留 48k 基线断言 `aac,48000,2`，新增 96k 变体断言 `aac,96000,2`（`tests/sr-itests/tests/sync_dual.rs:596,716-748`）。**audio_m6 率域迁移**：`:549-559,635-643,733-742` 当前把原始捕获固定解释为 48k——改为使用实际捕获率计算时长/裁剪/FFT；成品默认 48k 断言保留 | L2 |
| REQ-2 | 设置页行为 runner（**新建具名脚本** `tools/ui/settings-sample-rate.mjs`，headless Chrome + CDP，命令 `node tools/ui/settings-sample-rate.mjs`；M9 已验证本机 headless Chrome 可用）：五态 `<`/`=`/`>` 原生率 / 未知 / 保存重载 / 查询失败；real/mock 断言 | L1/静态 |

执行命令口径：L1 `cargo test --workspace`；L2 逐套件 `cargo test -p sr-itests --test <suite> -- --ignored --nocapture --test-threads=1`（`SESSIONNAME=Console`，独占机器，运行期保持唤醒）。

## 5. Out of scope

- 端点独占模式捕获、位深/声道数选择、每路不同采样率（全局单设置）。
- P2–P5 任何实现。

## 6. 风险

- **AAC 高采样率**：plan-reviewer 两轮只读实测（2026-10-18，锁定 ffmpeg `n8.1.3-18-g46d899d7a0-20261015`）：native `aac` 明列支持 88200/96000；四档合成音→AAC→内存 MKV→ffprobe→解码往返 exit 0。编码器接受性有依据；**完整录制/分段/恢复链路仍由 L2 验证**，不得把 route 失败归因 AAC。
- **原「96k 仅 WAV」降级出口已删除**：违背 Approved AC-2；如未来确需必须走需求变更重新批准。
- **原生高率捕获本机不可验证**：本机端点为 48k；96k 捕获行为以 REQ-6/REQ-8 单测 + 合成覆盖，真实 96k 端机留 L3。**96k 输出测试 ≠ 验证 96k 捕获**。
- **捕获率参数化吞吐**：引擎 96k 时 PCM 吞吐翻倍——背压/水位路径由 48k 基线回归 + 96k 子集覆盖。
- **进程环回无独立物理率**：UI 标注「跟随引擎混音率（通常 48k）」，不虚构来源精度。

## 7. 后续期占位（P2–P5）

| 期 | 内容 | PRD AC | 详规时点 |
|---|---|---|---|
| P2 | 全屏音轨（屏幕↔HDMI/DP 端点关联） | AC-4..5 | P1 验收后 |
| P3 | MP4 输出（MKV 分段 + 收尾流拷贝 remux） | AC-6..8 | P2 后 |
| P4 | 区域视频录制（音轨跟随音轨模式） | AC-9..11 | P3 后 |
| P5 | 窗口内容区（UIA spike 门控，仅 Chrome/Edge + 手动微调） | AC-12..14 | spike 先行 |

## 8. 里程碑

| 阶段 | 内容（REQ） | 通过条件 |
|---|---|---|
| M15（P1） | 采样率可选（REQ-1..8） | §4 全绿 + acceptor |
