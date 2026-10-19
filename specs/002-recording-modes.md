# 002 - Screen_rec 录制模式扩展（SDD）

Status: Approved
Created: 2026-10-18
Updated: 2026-10-19（P1 acc-8；P2 acc-10；P3 acc-11/12/13 修复闭环；P4 详规 v5 待审）
PRD: `docs/prd/002-recording-modes.md`（Approved 2026-10-18）

## 1. 范围（本版本）

**P1 采样率已实现**（acc-8 ACCEPTED）；**P2 全屏音轨已实现**（acc-10 ACCEPTED）；**P3 MP4 输出已实现**（acc-11/12/13 修复闭环；AC-8 人工项待用户）；**P4 区域视频已实现**（M18，2026-10-19；acceptor ACCEPTED，源码验收；安装包未重建）；P5 见 §7 占位。

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

### 3.1 P1 采样率（已实现）

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

### 3.2 P2 全屏音轨（v7，已批准；M16 已实现）

- **REQ-9（AC-4）渲染端点枚举**：新命令 `list_render_endpoints() -> Vec<{device_id, friendly_name, mix_rate: Option<u32>, is_active, container_id: Option<String>}>`（WASAPI 枚举 + `Device::as_immdevice()` 属性存储读 `PKEY_Device_ContainerId`；MTA；枚举失败返回空 + 日志）。`mix_rate` 允许未知。显示器目标音源区展示：**该屏输出端点（自动匹配，显示匹配状态）** / 系统混合 / 麦克风 / 追加应用。
- **REQ-10（AC-4）屏幕↔端点匹配（ContainerId 优先，含元数据链路）**：
  - **元数据链路**：保留 GDI 显示器枚举的稠密顺序 → 以现有 `szDevice`（`crates/sr-enum/src/monitors.rs:94`）与 DisplayConfig `viewGdiDeviceName` **按设备名字符串关联**（禁止两个枚举列表按位置配对）→ 取目标友好名与 `monitorDevicePath` → SetupAPI/CM devnode → `DEVPKEY_Device_ContainerId`。所需 windows crate feature 在 `crates/sr-enum/Cargo.toml` 增补。`MonitorOut` JSON 增 `friendly_name`/`device_path`/`container_id`，`ui/api.js` real/mock 接线；元数据失败仅降级字段（缺名回退「Monitor N」、container_id=None），**不丢显示器、不改 index/几何**。
  - **匹配纯函数** `match_endpoint(monitor_meta, endpoints) -> MatchResult`：GUID 规范化（去花括号/小写），全零 GUID 视为缺失；① ContainerId 非空且唯一 active 命中 → `Matched`；② 同 ContainerId 多端点 → 取 active 中名称最接近者，否则 `Ambiguous{candidates}`；③ 无 ContainerId → 仅 active 端点参与名称 hint（空串/回退名不作证据）→ `MatchedByHint`（标注「名称匹配，仅供参考」）；④ 无关联 → `Unmatched`（预选系统混合，文案「未能自动匹配」）；⑤ `Ambiguous` → **不取枚举第一项**，UI 呈现候选由用户确认，确认前用系统混合并明示。
  - **手选状态身份**：按稳定 `device_path` 保存（UI key 从稠密 index 改为 device_path）；**缺 path 时用 `idx:{稠密index}` 临时键**，临时键状态不跨刷新复用（防两台无元数据显示器合并/串屏）；每显示器独立状态；刷新后手选不覆盖、index 交换不串屏。
- **REQ-11（AC-4）端点环回捕获**：音源 JSON `{kind:"render_endpoint", device_id}`——设备混合音源（System 类）；捕获 = 该端点设备环回（wasapi 0.26 `get_device`→`iaudioclient` Capture/Shared 自动 LOOPBACK）；启动校验设备存在且为 Render（否则启动失败、明确错误、不产生路段）；采集格式继承 P1 REQ-7；`StreamMsg::Ready` 携带实际格式；新 CaptureTarget 纳入枚举/分发/命令注册链路，`ui/api.js` real/mock 双接线。
- **REQ-12（AC-4）失效传播（跨线程桥接 + 双结果契约）**：
  - **检测**：(a) `IMMNotificationClient` OnDeviceRemoved / OnDeviceStateChanged（非 active/disabled）按 device_id 过滤；(b) 采集核 `AUDCLNT_E_DEVICE_INVALIDATED` HRESULT（不再吞错，`wasapi.rs:487-500` 改造）→ `StreamEvent::EndpointLost {device_id, reason}`。**静音/无包不判失效**。
  - **跨线程桥接**：`StreamEvent` 由音频**写线程**消费（`route.rs:1463-1468`）；写线程收到 EndpointLost 后更新共享 `stop_reason=E_TARGET_LOST(endpoint=<名>)` 并经既有停止通道通知 **route worker**；worker 据 stop_reason 走优雅停路（EOF/收尾），仅停该路，其它路继续；不自动换端点。
  - **交付无缝性**：`ChannelHub::subscribe` 改为**先注册接收端再读快照**（或快照携带 seq、客户端检测间隙重取）——注册窗口内的终态不得丢失；`publish_alert` 对失效订阅**逐订阅清理**，不得因单个死订阅丢弃对活订阅的投递（`events.rs:122-139,158-167` 现状需改）。**快照生成与 Hub 提交串行化**（同一锁内完成）或以权威 revision 拒绝旧提交——A 取旧快照/B 提交终态的交错不得回退缓存（`commands.rs:506-524,548-552` 现状需改）。UI 侧：**迟到旧快照不得覆盖已收到的终态**（`app.js:683-687` 直接覆盖路径需改，以 route 终态标记为准，仅合并未终态字段）。确定性测试：订阅与终态发布交错、死订阅并存、旧快照迟到、**旧快照提交后新订阅仍得终态**四场景。
  - **清理**：遵循 M14c 语义——成品验证通过后才清理（keep_segments 门控）；删除失败仅日志、不改成功归因；收尾失败保留原段。保留矩阵三分支分别断言：成功+keep=false 已清理 / 成功+keep=true 保留 / 收尾失败保留。真实拔插/禁用留 L3。
- **REQ-13（AC-5）音轨模式与 sources 组装（逐项勾选；管线契约不变）**：
  - 管线仍只接收 sources（不新增 mode 字段）。**每个音源条目有独立 selected 状态**（endpoint/system/mic/apps）；自动匹配与回退只是**预选**，不是强制添加——用户可只留麦克风、取消系统、或全部取消（→ REQ-14 拒绝路径）。既有 `srcSystem/srcMic/srcApps` 勾选语义保留并扩展 endpoint 条目。
  - **勾选语义（含隔离模式矩阵）**：每个音源条目有独立 selected 状态（endpoint/system/mic/apps）；自动匹配与回退只是**预选**，不是强制。**sources = 恰好所有已勾选条目**（诚实包含所选音轨，不静默丢弃）：隔离 + 显示器匹配 → 预选仅端点；用户再勾麦克风 → 2 轨（与窗口隔离+麦克风行为一致）；未勾端点 → 预选系统混合；全部取消 → REQ-14 拒绝路径。全音轨切换回隔离时，勾选状态保留为用户可见状态并如实计入（UI 明示，不做隐式清除）。
  - 组装顺序：已勾选条目按 UI 标签序 [endpoint, system, mic, apps]。
  - 命令 DTO：显示器沿用传输契约 `mode=all_tracks` + 实际勾选 sources（命令层 `mode=isolated` 拒绝行为不变，`commands.rs:1150-1160`）；窗口目标 + 端点 → 拒绝；纯录音（`plan_audio_only`）不接受端点 kind。
  - **去重与钉住**：后端启动时**一次解析并钉住** System 的实际 device_id，**该 resolved ID 进入捕获目标**（激活用 `get_device(id)`，不得再次查询默认端点，替代 `wasapi.rs:347-356` 的激活时查询）；在轨数/管道分配前按实际 ID 去重（endpoint==system ⇒ 单轨，保留端点的失效监控语义；≠ ⇒ 两条，轨序与 UI 标签一致）。**激活边界证据**：通过生产激活路径的可注入捕获工厂/等价探针断言——激活调用 `get_device(钉住ID)` 且**无默认设备查询**；钉住设备失效 → 该路失败，**不回退**默认设备。
  - 成品 ffprobe 验证采样率/声道/时长 + 已知音调 FFT。
- **REQ-14（AC-5）无音源显式提示**：显示器目标零音源 → 启动被拒 + 明确提示（文案含端点选项引导）。
- **REQ-15（回归）**：既有全部 L1/L2 回归绿；P1 采样率 × 端点音源覆盖 48k 与 96k 两档组合。

## 3. 需求（REQ）

### 3.3 P3 MP4 输出（v5，已批准；M17 TDD）

- **REQ-16（AC-6）设置项**：`output_container: "mkv" | "mp4"`，默认 mkv；serde 字段级容错（同 audio_sample_rate 模式）；设置页下拉「成品格式」+ 说明「录制内部始终 MKV 分段（崩溃安全）；MP4 为收尾流拷贝；多音轨 MP4 播放器通常选一轨播放（不自动混音）」；`ui/api.js` real/mock 接线（**mock 的正常/提前终态/恢复路径需按设置返回容器**，现状硬编码 MKV `api.js:324,407-408,500`）；保存重载与在录快照测试。
- **REQ-17（AC-6）收尾事务**：
  - **启动冻结**：启动时读取容器设置并冻结进路由配置（旧入口默认 MKV；**不得丢 out_rate**；托盘同路径，`commands.rs:872-879`、`route.rs:498-515` 需扩）。**最终名预留从启动即按所选容器扩展名**（`route.rs:1274-1285,1386` 的 `.mkv` 硬编码需改）。
  - **流程**：concat 到会话目录内 `<name>.concat.mkv.tmp`（`.tmp` 后缀防恢复重扫当段，`recovery.rs:640-660,764-768`）→ 统一基础校验 → remux：`ffmpeg -hide_banner -nostdin -i <tmp> -map 0 -c copy -movflags +faststart -f mp4 <out>.tmp`（**`-map 0` 保全部流**；显式 `-f mp4`）→ MP4 校验（容器族 mov/mp4/m4a/…、**轨数=实际去重后 sources 期望值**、时长>0）→ `safe_commit` 最终 MP4 → 按 keep 清理（含临时物）。
  - **会话级占用（正常/恢复共用）**：收尾事务**全程**（含 remux 阶段）持有会话级占用（会话目录内占用标记 + 进程内注册表）；活动探测以该占用为准并覆盖 `.tmp`/remux 阶段（扩展 `recovery.rs:585-600,714-718`）；失败后立即释放、允许同进程立即重试；**不得仅凭 PID 存活或 `.tmp` 存在判断**。测试：remux 暂停时扫描=active、第二次收尾被拒；失败后立即可重试且原段不重复处理/清理。
  - **超时与批量回滚协调**：仅**批量启动回滚侧**显式取消/丢弃后台收尾，且取消与提交串行化、返回失败前确认 sidecar 已回收；**通用 `wait_finalized` 契约不变**（超时后后台继续，不得取消正常长录制，`route.rs:212-222`、`commands.rs:680-688`）。测试两类：①延迟 remux+第二路启动失败 → 回滚返回后无迟到成品无遗留 sidecar；②普通等待超时后仍可正常停止并成功收尾。
  - **失败矩阵**：spawn 失败/非零退出/超时/MP4 校验失败/提交冲突 → 保留原段、不发布成品；临时物由调用层清理；`safe_commit` 自身失败契约不变（`naming.rs:119-144`）。remux 独立超时预算（非 concat 60s）、stderr 排空、sidecar Job 生命周期；**磁盘峰值 ≈3 份媒体**（原段+concat+临时 MP4）需收尾前预检。
- **REQ-18（AC-7）恢复、幂等与合并**：`finalize_session` **执行时**一次读取 output_dir/keep_segments/output_container（非扫描时，`commands.rs:1118-1121,1516-1519` 需改）；容器设置仅作用于未完成路由；已有有效 `final_path` → 同口径复核后复用，**不因设置变化再生成另一容器**；恢复始终流拷贝既有采样率（不用当前设置重编码）。合并输出保持 MKV（`merge.rs:285-290`）；**MP4 成品可参与合并**（最小支持本产品 MP4：MP4+MP4、MP4+MKV→MKV；`crates/sr-merge/src/inputs.rs:25-32` 的 EBML 强制改为容器探测）；UI 合并页说明兼容性与多轨 MP4 限制（`index.html:107,120-122`）。
- **REQ-19（AC-8）成品可播 + 人工验收**：ffprobe 断言容器族（`format_name` 属 `mov,mp4,m4a,3gp,3g2,mj2`，**不得断言 =="mp4"**）、逐轨率/声道/内容 FFT、整文件解码、时长容差、同步（复用 `sync_dual.rs:569-660` 断言口径）。**具名 L3 人工验收**：系统自带播放器播放 MP4 并记录 OS/播放器版本/画面/声音/同步。**高采样率风险**：Windows AAC Decoder 不支持 >48kHz（官方文档）——88.2k/96k MP4 存在播放兼容风险；**不得为过验收静默降采样或转码**，不兼容时提交用户裁决并记录为已知限制。
- **REQ-20（回归）**：全部既有回归（含 P1 四档率、P2 端点链路、两套 UI runner）；mkv 默认路径零行为变化（sync_dual 48k 基线）。

### 3.4 P4 区域视频录制（v5，已批准；M18 TDD）

- **REQ-21（AC-9）区域目标与选择流程**：
  - **尺寸口径**：输出尺寸 = 选区物理像素**向下取偶吸附**（NV12/编码约束）；覆盖层实时显示吸附后**最终尺寸**并要求确认，禁止确认后静默少像素。**最小有效尺寸与编码器支持**：<64×64 或编码器不支持的尺寸 → 启动失败并明确提示（不静默缩放/补边）；编码器探测仍用 256×256，实际尺寸在区域路启动时以真实参数初始化验证。
  - **单屏限制**：v1 仅接受完全位于单个显示器内的矩形；跨屏/空洞/越界 → 明确拒绝（截图跨屏拼合能力不受影响）。Gate 2 待确认限制之一。
  - **overlay record 协议（含初始化上下文与 ACL）**：`open_region_overlay(mode, request_id)` async（WebView2 重入约束；默认 screenshot 兼容；request_id 主窗生成随调用传入）；**overlay 初始化上下文经 `get_region_context()` 拉取 `{mode, request_id}`**（首次创建与复用窗口一致；复用异模式以最新代次为准，旧代次结果丢弃）；就绪顺序：主窗建立监听并确认就绪 → open → overlay 拉取上下文 → 用户操作；record 确认 = `confirm_region_record(request_id, rect)` 返回载荷 `{request_id, rect, display{device_name, device_path}}` 并经 `region_target_added` 事件交付主窗（交付前代次复核、一次性交付）；`cancel_region_record(request_id)` 使迟到确认失效；event.listen 所需最小 ACL 授权（`core:event:allow-listen/unlisten` 或既有 Channel 等价路径）写入 capabilities 并以**真实 Tauri 验证**（现状 capabilities `{}`，Tauri 2.12.4 ACL 会拒绝）；record 路径绝不进截图命令；确认成功 → 隐藏（DwmFlush+≥150ms）→ 交付 → 结束生命周期；失败提示可见。
  - 录制启动仍走标准入口；`ui/api.js` real/mock + 命令注册同步。
- **REQ-22（AC-9）裁剪链与 DTO**：
  - **IPC DTO 定稿（含截图兼容）**：`TargetIn::Region { display_device: String, rect: {x,y,w,h} }`——`display_device` 为**稳定设备路径**（P2 device_path；路径不可得回退 **GDI `szDevice`**——`MonitorEntry`/`MonitorOut` 需新增 `gdi_device_name` 保留链路，**不得用友好名代替**；两者皆缺 → 启动拒绝）。共享字段以 serde 默认值保持**旧截图 JSON 反序列化兼容**，录制入口强制校验；`commands.rs:74-80` 与映射 `:1404-1412` 同步扩展（现状丢弃 rect）；旧截图 JSON 兼容回归测试入 §4。
  - **坐标与边界**：帧内 crop = 桌面 rect − 显示器原点（左上）；**调用前校验** `0 ≤ start < end ≤ frame 尺寸`（锁定库 `frame.rs:188-218` 不检查终点越界）；显示器不存在/矩形不包含/越界 → 启动失败或停路并明确原因。
  - **裁剪**：`Frame::buffer_crop` BGRA 帧内裁剪 → 同尺寸 NV12；pacer/最近帧/CFR 不变；禁止整屏缩放冒充裁剪；区域路 `video.canvas_*` 不生效（区域尺寸=唯一编码尺寸，无 letterbox）；窗口/显示器路径不变。
- **REQ-23（AC-10）音轨矩阵（防隔离泄音）**：区域隔离模式 = **仅所选应用**各自独立轨，不隐式加入设备混音；端点音源仅全音轨模式可选（按 P2 匹配/手选，与系统同 ID 去重；§5 的「端点适用于显示器目标」在本条区域全音轨例外）；全音轨 = 系统+麦克风+追加应用（+可选端点）；`buildAudioPlan` 为区域拆独立分支（`app.js:570-602` 不得直接复用）；命令层接受矩阵扩展时保留窗口/纯录音拒绝与 P2 勾选语义（`commands.rs:1322-1333`）；零音源拒绝。端点仅全音轨为 Gate 2 待确认限制之二。
- **REQ-24（AC-11）失效传播、恢复与合并**：
  - **检测与顺序（含闩锁）**：显示器失效来源 = WGC `on_closed` + 显示器设备/几何通知（由路由线程订阅/轮询并在路由结束时清理；DisplayConfig 现仅快照查询需扩展）；捕获生产者**在标记 capture_ended 之前同步闩锁 first_cause**（`RegionSourceLost{reason}`）；worker 检测到 capture_ended 后**再次读取 first_cause** 再决定收尾——杜绝「检查为空 → 事件入队 → 结束」竞态；E_TARGET_LOST 含区域原因（复用 P2 notice）；无新帧/静止桌面不得判失效；分辨率变化/crop 越界 → 停路保留原因、不无限重复旧帧或重新缩放；音频端点失效保留 P2 语义。
  - **恢复/合并无区域分支**（前提：标准路由 + start_routes_impl 快照入口；恢复按清单 streams；合并全轨流拷贝支持异构尺寸）；区域+窗口并发允许。

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
| REQ-9 | `it_list_render_endpoints`：真实枚举字段有效（container_id 可空、mix_rate 可未知）；无设备 → 空列表不失败 | L2 |
| REQ-10 | `unit_render_endpoint_match_*`（**调用生产匹配函数**，注入显示器元数据+端点列表）：ContainerId 唯一命中 / 多端点取优 / 名称 hint（仅 active） / 零命中回退系统混合 / 全零 GUID / 缺名空串 / 同分 / 全 inactive / 顺序无关 / Ambiguous 不取首项 | L1 |
| REQ-10 | `it_monitor_metadata_real`：生产枚举→元数据→DTO——本机显示器友好名可见且 ≥1 台 monitor container_id 与某端点 container_id 相等（证明未实现成恒 None）；元数据失败降级不丢显示器 | L2 |
| REQ-10 | UI 状态（tools/ui runner 扩展，单独执行）：手选覆盖保持、刷新不覆盖、**index 交换不串屏**（device_path 键）、**双屏同时缺元数据不合并/临时键不复用**、每显示器独立 | L1/静态 |
| REQ-11/13 | `it_route_endpoint_audio_fft_nondefault`：向非默认端点渲染已知音调 → ~10s 显示器目标录制 → FFT 命中该端点音调且与系统混合不同（防忽略 device_id）；48k/96k 两档；第二端点不可用 → BLOCKED | L2 |
| REQ-12 | `it_route_endpoint_invalid_at_start`：无效/非 Render → 启动失败、无路段、明确错误；`it_route_endpoint_runtime_lost_keeps_others`：**经生产传播链注入**（通知/HRESULT）→ 该路 E_TARGET_LOST、`stop_notice` 可读（成功与收尾失败两分支都断言）、其它路继续；保留矩阵三分支断言（成功+keep=false 已清理 / 成功+keep=true 保留 / 收尾失败保留）；静音无包不误停；在线事件交付 / 重订阅快照恢复 / 终态早于启动回包（未知 route_id 缓存）/ 旧快照提交后新订阅仍得终态——四场景；**迁移 `crates/sr-core/tests/channel_snapshot_rebind.rs:64-77`**：活订阅收到首条及后续告警（原「死订阅丢首条」契约作废），保留失败容忍与节流覆盖；真实拔插留 L3 | L2 |
| REQ-13 | `tools/ui/audio-sources-builder.mjs`（**具名 Node/CDP runner**，点击真实 UI 捕获实际 start DTO）：隔离预选仅端点；端点+麦克风勾选→2 源；取消端点→系统预选；全取消→REQ-14；模式切换勾选如实保留；刷新→启动间默认设备变化 UI 不提前省略；失败告警 UI 映射（成品路径+告警并存） | L1/静态 |
| REQ-13 | `unit_system_target_resolution_pinned_*`（**route 目标映射边界，可注入默认解析器**）：解析后默认设备再变化 → 仍捕获钉住 ID（激活不二次查询，wasapi.rs:347-356 路径替换）；去重后轨数与端点失效监控语义；`unit_commands_audio_source_validation_*`（窗口/纯录音拒绝、隔离 mode 映射、去重时机） | L1 |
| REQ-13 | `it_route_display_endpoint_tracks`：**仅预选端点 → 1 音轨**；**端点+麦克风勾选 → 2 音轨**；全音轨去重后 N 轨（端点≠系统时两条）；模式切换后按实际成品 ffprobe 轨数断言（非仅 UI DTO） | L2 |
| REQ-14 | 零音源显示器目标：命令层拒绝 + UI 提示可见（runner） | L1/静态 |
| REQ-15 | 回归清单（§3.1 REQ-5 全列）+ 48k/96k × 端点音源组合 | L2 |
| REQ-16 | `unit_settings_output_container_*`：缺字段→mkv；坏值→mkv 且其余字段保留；往返持久化；设置页保存重载/在录快照（UI runner）+ mock 三路径按容器返回 | L1 |
| REQ-17 | `unit_remux_args_*`：`-map 0`/`-c copy`/`+faststart`/`-f mp4`/`.tmp` 路径；`unit_finalize_container_failures_*`：**可注入 remux runner 在 MKV 校验之后**制造 spawn 失败/非零/超时/校验失败/提交冲突 → 原段保留、无成品、临时物清理 | L1 |
| REQ-17 | `it_batch_rollback_no_late_commit`（**确定性注入**：可注入 remux runner 延迟 → 第二路启动失败）→ 返回失败前 sidecar 已回收、回滚后无迟到提交/无遗留成品；`it_wait_finalized_timeout_not_cancel`（普通 `wait_finalized` 超时）→ 录制**未被取消**，随后停止并成功收尾 | L2 |
| REQ-17/19 | `it_route_container_mp4`：短录 → .mp4 成品；ffprobe 容器族、逐轨率/声道/FFT、整文件解码、时长容差；keep=false 清理；**四档率流拷贝夹具**；**多音轨 MP4（端点≠系统 2 轨）**；端点失效后的 MP4 收尾；并发名预留+提交冲突（`m11_route_commit.rs:116,208,254` 惯例） | L2 |
| REQ-18 | `it_recovery_container_mp4`（**仅 L2 媒体侧**）：恢复产出容器与设置一致、有效 final_path 幂等复用（不重生另一容器）；**设置变更时机断言全部迁命令层 L1**：`unit_commands_recovery_settings_snapshot_once`（注入生产读取边界，断言 **Settings 快照提供者总共调用一次、output_dir/keep_segments/output_container 三字段来自同一快照**）；`it_merge_mp4_inputs`：MP4+MP4、MP4+MKV→MKV，输出 matroska | L1/L2 |
| REQ-19 | `it_sync_dual_container_mp4`（**具名 MP4 同步变体，分析 MP4 本身**）：覆盖开始/中段/结束/段边界偏差与漂移断言（复用 `sync_dual.rs:569-660` 口径），MKV 基线保留；L3 人工：系统播放器播放 MP4（OS/播放器/画面/声音/同步）；88.2k/96k 不兼容时用户裁决（不静默降采样） | L2/L3 |
| REQ-20 | 回归清单明确：P1 四档率 + P2 端点链路 + route/recovery/m11_*/sync_dual 48k + 两套 UI runner（mkv 默认零变化） | L2 |
| REQ-21 | `tools/ui/region-record.mjs`（**具名 Node/CDP runner**）：拖拽→吸附后尺寸预览+确认→目标出现；Esc→无目标/无文件/无残留；跨屏/越界拒绝；重复打开异模式状态正确；迟到确认（request_id 失效）丢弃；隐藏时序（E2E 像素断言复用）；`unit_region_record_geometry_*`（150% DPI+向下取偶+单屏包含+<64×64 拒绝） | L1/静态 |
| REQ-22 | `tests/sr-itests/tests/region_video.rs`：`it_region_video_rect`（~10s，非零偏移/非屏幕宽高比/选区内外不同图案）→ ffprobe 尺寸=吸附后物理矩形、时长、**裁剪内容断言**（排除整屏缩小/裁错位）；48k/96k 各一；`it_region_size_unsupported_rejects`（编码器不支持尺寸→启动失败不缩放/补边） | L2 |
| REQ-22 | **真实 GUI E2E 承担贯通断言**（扩展 `tools/e2e/region-e2e.ps1` mode=record）：经 record 入口开录 → 首帧与后续帧无覆盖层（NV12/H.264 帧断言，不复用 PNG 阈值）；**同次 E2E 断言成品轨数与所选/未选音调（FFT）**——DTO→命令→成品贯通证据；区域尺寸非整屏 | L2/E2E |
| REQ-22 | `unit_region_ipc_dto_*`（命令层 L1）：display_device 稳定路径→GDI szDevice 回退→皆缺拒绝；桌面→帧内坐标公式与越界校验；**旧截图 JSON 反序列化兼容回归**；`unit_commands_source_rejection_*`：区域隔离下端点直接 IPC 拒绝、窗口/纯录音端点拒绝矩阵（不得用手拼管线配置替代） | L1 |
| REQ-23 | `it_region_video_audio_modes`：隔离=仅所选应用（无设备混音泄漏，FFT 负例）；全音轨含系统+麦克风+应用（+端点同/不同 ID 去重）；端点仅全音轨（拒绝矩阵见命令层 L1 行）；零音源拒绝；麦克风缺设备仅 BLOCKED 该分项 | L2 |
| REQ-24 | `it_region_display_lost_keeps_others`（失效→E_TARGET_LOST+notice、其它路继续；**失效后收尾失败仍保留 notice**）；**`it_region_latch_interleaving`（屏障式确定性交错：worker 读空→生产者闩锁 first_cause→capture ended→worker 重读；成功与收尾失败两分支均保留 notice）**；`it_region_video_recovery`（**强杀录制宿主子进程**，复用 `recovery.rs:510-571` 手法→恢复→尺寸/全部流/解码正确）；`it_region_container_mp4`；`it_region_merge`（异构尺寸）；区域+窗口并发；分辨率变化/crop 越界用例 | L2 |

执行命令口径：L1 `cargo test --workspace`；L2 逐套件 `cargo test -p sr-itests --test <suite> -- --ignored --nocapture --test-threads=1`（`SESSIONNAME=Console`，独占机器，运行期保持唤醒）。

## 5. Out of scope

- 端点独占模式捕获、位深/声道数选择、每路不同采样率（全局单设置）。
- P5 任何实现。
- P4 的跨屏选区（v1 仅单屏；截图跨屏能力不受影响）。
- P2 是 001「仅默认端点」窄范围（001:54,297）的扩展：端点音源适用于显示器目标（窗口目标的隔离限制不变）；**P4 例外**：区域目标在全音轨模式下同样可选端点（隔离模式仍仅所选应用，见 §3.4 REQ-23）。

## 6. 风险

- **AAC 高采样率**：plan-reviewer 两轮只读实测（2026-10-18，锁定 ffmpeg `n8.1.3-18-g46d899d7a0-20261015`）：native `aac` 明列支持 88200/96000；四档合成音→AAC→内存 MKV→ffprobe→解码往返 exit 0。编码器接受性有依据；**完整录制/分段/恢复链路仍由 L2 验证**，不得把 route 失败归因 AAC。
- **原「96k 仅 WAV」降级出口已删除**：违背 Approved AC-2；如未来确需必须走需求变更重新批准。
- **原生高率捕获本机不可验证**：本机端点为 48k；96k 捕获行为以 REQ-6/REQ-8 单测 + 合成覆盖，真实 96k 端机留 L3。**96k 输出测试 ≠ 验证 96k 捕获**。
- **捕获率参数化吞吐**：引擎 96k 时 PCM 吞吐翻倍——背压/水位路径由 48k 基线回归 + 96k 子集覆盖。
- **进程环回无独立物理率**：UI 标注「跟随引擎混音率（通常 48k）」，不虚构来源精度。
- **显示器↔端点关联**：主依据为 PnP ContainerId（本机只读实测：LS24A600N、U27G4WS 各自与对应 NVIDIA 音频端点同 ContainerId）；名称仅作 hint。克隆/同型号双屏/其它驱动的 ContainerId 完整性留 L3，歧义交用户确认。
- **端点失效通知时序**：removed/disabled/invalidated 通知在驱动层可能延迟或缺失——L2 以注入验证传播逻辑，真实拔插留 L3。
- **MP4 收尾资源峰值**：原段 + concat MKV + 临时 MP4 ≈ 3 份媒体体积；faststart 有第二遍 I/O；收尾前做空间预检，remux 独立超时与 Job 生命周期（不套用 concat 60s 预算）。
- **高采样率 MP4 播放兼容**：Windows AAC Decoder 不支持 >48kHz（官方文档）——88.2k/96k MP4 可能无法在系统播放器播放；不得静默降采样/转码，不兼容提交用户裁决（L3 记录）。

## 7. 后续期占位（P2–P5）

| 期 | 内容 | PRD AC | 详规时点 |
|---|---|---|---|
| P2 | 全屏音轨（屏幕↔HDMI/DP 端点关联） | AC-4..5 | **详规 §3.2 v7 已批准（2026-10-18）；M16 已实现（TDD 红→绿，见 §8）** |
| P3 | MP4 输出（MKV 分段 + 收尾流拷贝 remux） | AC-6..8 | **§3.3 v5 已批准（2026-10-18）；M17 已实现（TDD 红→绿，见 §8）** |
| P4 | 区域视频录制（音轨跟随音轨模式） | AC-9..11 | **§3.4 v5 已批准（2026-10-19）；M18 已实现，acceptor ACCEPTED（源码验收，见 §8）** |
| P5 | 窗口内容区（UIA spike 门控，仅 Chrome/Edge + 手动微调） | AC-12..14 | spike 先行 |

## 8. 里程碑

| 阶段 | 内容（REQ） | 通过条件 |
|---|---|---|
| M15（P1） | 采样率可选（REQ-1..8） | **已实现，acc-8 ACCEPTED（2026-10-18）**（L1/L2 证据：`docs/milestones/m15-report.md`；audio_m6 麦克风正路径本机无采集设备 → BLOCKED） |
| M16（P2） | 全屏音轨（REQ-9..15） | **已实现，acc-10 ACCEPTED（2026-10-18）**：acc-9 两项 P1（终态单调/Ambiguous 首项）修复后复审通过；详见 `docs/milestones/m16-report.md` |
| M17（P3） | MP4 输出（REQ-16..20） | **已实现（acc-11/12/13 修复闭环，2026-10-19）**：container 14/14（debug）；acc-12 两项 P2 修复经 acc-13 复审通过；**AC-8 系统播放器人工记录待用户**；端点+麦克风组合因本机无采集设备 → BLOCKED；L3 见 `docs/milestones/m17-report.md` |
| M18（P4） | 区域视频录制（REQ-21..24） | **已实现，acceptor ACCEPTED（源码验收，2026-10-19）**：L1 全绿；L2 `region_video` 10/10；E2E record（含真实 UI 隔离双应用）+ 区域截图回归全 PASS；M18b 生命周期修复后复审通过；**安装包未重建**；麦克风/真实 96k 捕获/多视频轨播放器矩阵仍 BLOCKED/待签；详见 `docs/milestones/m18-report.md` |
