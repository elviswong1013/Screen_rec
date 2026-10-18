# M8-C 绿实现报告（TDD 绿阶段：探测/降级、许可审计、恢复状态机（完整）、命名收尾 REQ-22/24/28/29）

- 日期: 2026-10-17（全部命令为本机实际执行；日志归档见 §8）
- 角色: **executor-backup 承担实现**（GLM 系 test-writer/executor 额度耗尽至 2026-10-22，编排者指派后备通道；沿 M5–M7 先例）
- 规格: specs/001-screen-recorder.md §2.5（REQ-22 完整 / REQ-24）、§2.7（REQ-28）、§2.8（REQ-29）、§3.2、§3.3（probe/recover/finalize/discard）、§3.4（SessionManifest v1 / 错误码）、§3.5（崩溃残留/磁盘不足）、§4、§5 M8 行、§8 M0 冻结项（S2/S3/S5）
- 红阶段基线: docs/milestones/m8-red-report.md（**§3 契约=绑定**；§6.6 rehearsal 两条回写要点 R1/R2；§8 未决 11 条）
- 环境: Windows 11 25H2，`Microsoft Windows [Version 10.0.26200.9711]`；cargo/rustc **1.99.0** msvc；锁定 ffmpeg n8.1 release；`SESSIONNAME=Console`（L2）。仓库无 git 提交历史；未 git 提交。
- **工具安装约束**：按任务指示**未安装任何工具**；cargo-deny 缺失 → `license_and_build_audit` 按契约语义 **BLOCKED**（缺口记录见 §4/§5-1，待编排者确认）。

## 1. 范围与约束遵守

- **未修改任何测试文件**：四个受控哈希会话前后逐字一致——
  `safe_commit.rs` `AA5C6A9250CC4E8F…`、`license_and_build_audit.rs` `B512E4CA4016D696…`、`probe.rs` `E4F92E98F4BBE1BF…`、`recovery.rs` `555C94943201DA5A…`；既有测试（route/multi_route/audio_isolation/audio_m6/merge）哈希亦与红阶段记录一致。
- **未改** `specs/`、`docs/spikes/`、`src-tauri/`。生产改动 = `crates/sr-pipeline`（naming/sidecar/route/lib + 新 probe/recovery）+ 其 `Cargo.toml`（windows features）+ 根新增 `deny.toml`（REQ-29 配置，属验收对象）。
- **零新增外部依赖**：`Cargo.lock` 哈希 `D6817AB08183611B…` 与红阶段一致未变；`windows` features 仅扩展（JobObjects/Performance），无新包。
- 无调试/临时内容残留（生产侧无 TEMP 诊断；recovery 测试残留目录均为**红阶段 rehearsal** 会话产物，见 §7-9）。

## 2. 变更文件清单

| 文件 | 变更 |
|---|---|
| `crates/sr-pipeline/src/probe.rs`（新增） | REQ-28：`LayerStatus` 六态 / `LayerReport` / `EncoderProbe` / `CanvasLimits` / `ProbeCapabilities` / `ProbeInjections`；`probe_capabilities`（真实分层探测：OS build、D4 链逐候选实际初始化、process loopback 试激活、麦克风枚举+短试录、WGC Monitor0 最小会话试激活、画布上限）+ `probe_capabilities_injected`（`Some`=整层替身、`None`=真实探测） |
| `crates/sr-pipeline/src/recovery.rs`（新增） | REQ-22：`MANIFEST_FILE`/`RouteIdentity`/`ManifestSegment`/`ManifestRoute`/`SessionManifest` v1、`save_manifest_atomic`/`load_manifest`（**手写受限 JSON**：仓库零新增外部依赖；畸形/损坏 → `RecoveryFailed`）、`scan_sessions`（目录重扫、0 字节/坏 EBML/活动文件排除、封口/尾段切分）、`finalize_session`（幂等、安全提交、WAV 就地封口）、`discard_session` |
| `crates/sr-pipeline/src/naming.rs` | REQ-24/D12：`SESSION_TMP_DIRNAME`、`session_tmp_dir`（纯函数）、`CommitError`、`safe_commit`（FinalExists→verify→rename；失败保留临时物） |
| `crates/sr-pipeline/src/sidecar.rs` | REQ-29：`LockedBuildFacts`/`locked_build_facts`（zip SHA256 `337B8546…C240`、buildconf 必含/禁用标记）、`BuildAudit`/`audit_locked_build`（实际运行 `-version/-buildconf/-encoders` + `certutil` 校验和 + LICENSE.txt）；D8：进程级 **Kill-on-close Job**（`spawn_sidecar` 内绑定；主进程亡 → sidecar 连带终止）；`probe_encoder_detail` 供 probe 复用 |
| `crates/sr-pipeline/src/route.rs` | REQ-22 集成：`RouteError::DiskFull`；录制会话自动建 `<output>/.screenrec_tmp/<session_id>` + 初始 manifest（start_qpc=QPC）；段/ffconcat 清单落会话目录、成品仍输出根；录制期 manifest 节流刷新（段列表/closed）；收尾成功回写 `final_path`；**写失败分类扫描完整 16KB stderr 尾部**（R1）；`current_unix_ms`/`local_utc_offset_minutes`/`qpc_100ns` 供 recovery 复用 |
| `crates/sr-pipeline/src/lib.rs` | `pub mod probe`；`recovery` 由 TODO 骨架落地；模块文档同步 |
| `crates/sr-pipeline/Cargo.toml` | windows features +`Win32_System_JobObjects`/`Win32_System_Performance`（Job 绑定 / start_qpc） |
| `deny.toml`（根，新增） | REQ-29：`[licenses] allow`（MIT/Apache-2.0 等；GPL 系零命中）+ confidence-threshold |
| `docs/milestones/m8-green-report.md` + `m8-green-*.log` ×15 | 本报告 + 原始日志归档 |

## 3. 绿前红态复现（修改前，exit 101）

| 命令 | 结果 |
|---|---|
| `cargo test -p sr-pipeline --test safe_commit --test license_and_build_audit --no-run` | **exit 101**：2×E0432（naming 3 符号 + sidecar 2 符号），与红 §6.2 逐条一致（`m8-green-red-pipeline.log`） |
| `cargo test -p sr-itests --test probe --test recovery --no-run` | **exit 101**：E0432（`sr_pipeline::probe`；recovery 12 符号）+ E0599（`RouteError::DiskFull`）（`m8-green-red-itests.log`） |

## 4. 复跑证据（冻结树；命令 + exit + 计数 + 耗时）

| # | 命令 | 结果 | 日志（docs/milestones/） |
|---|---|---|---|
| 1 | `cargo check -p sr-pipeline --all-targets` | **exit 0**（0 warning 0 error） | `m8-green-check-sr-pipeline.log` |
| 2 | `cargo test -p sr-pipeline --test safe_commit -- --nocapture` | **exit 0；2 passed**（0.02s；含跨卷 Io 分支执行） | `m8-green-l1-safe-commit.log` |
| 3 | `cargo test -p sr-pipeline --test license_and_build_audit -- --nocapture` | **所有实际审计断言通过**（工具链 1.99.0 ✓；deny.toml 白名单 MIT/Apache-2.0 ✓；版本 `n8.1.3-18-g46d899d7a0-20261015` 含 n8.1 ✓；zip SHA256 实测一致 ✓；buildconf 5 项全中 ✓；禁用编码器零命中 ✓；LICENSE.txt=LGPL v3 ✓；docs 来源 SHA256 记录命中 ✓）→ **exit 101：BLOCKED（cargo-deny 未安装，工具缺失语义；见 §5-1）** | `m8-green-l1-audit.log`（+ 初跑缺 deny.toml 中间态 `-initial.log`） |
| 4 | `cargo test -p sr-itests --test probe -- --ignored --nocapture` | **exit 0；1 passed（1.50s）**：注入矩阵 ×4 + 真实 smoke 全绿 | `m8-green-l2-probe.log` |
| 5 | `cargo test -p sr-itests --test recovery -- --ignored --nocapture --test-threads=1` | **exit 0；2 passed（47.64s）**：S1–S5 合成矩阵 + 相位 R（真实子进程强杀/Job 回收/活跃拒绝/崩溃收尾/幂等/丢弃）+ 相位 R2（编码进程杀→E_SIDECAR_EXIT→恢复收尾）+ 磁盘满注入全绿 | `m8-green-l2-recovery.log` |
| 6 | `cargo test --workspace`（任务原命令） | **exit 101**：唯一失败 = `license_and_build_audit`（cargo-deny BLOCKED）；cargo 默认 fail-fast 在失败后停止后续目标 | `m8-green-ws.log` |
| 7 | `cargo test --workspace --no-fail-fast`（完整证据） | **exit 101**：**L1 15 passed / 0 failed**（screen_rec_lib 7、drift 1、silence_pad 1、pacer 1、channel 1、ffmpeg_args 1、output_naming 1、**safe_commit 2**)；唯一失败 = audit BLOCKED；L2 门 ignored：route 5 / multi_route 4 / audio_isolation 4 / audio_m6 4 / merge 1 / probe 1 / recovery 2 / shot 4 / enum_* 5；**1 条 warning = 受控测试文件 probe.rs 的 unused import `EncoderProbe`**（见 §5-9，未改） | `m8-green-ws-nff.log` |
| 8 | 回归 `--test route --test-threads=1` | **exit 0；5 passed（60.54s）** | `m8-green-reg-route.log` |
| 9 | 回归 `--test multi_route` | **exit 0；4 passed（37.86s）** | `m8-green-reg-multi-route.log` |
| 10 | 回归 `--test audio_isolation` | **exit 0；4 passed（47.19s）** | `m8-green-reg-audio-isolation.log` |
| 11 | 回归 `--test audio_m6` | **3 passed / 1 mic BLOCKED（无采集设备；环境状态与 M6-C/M7-C 一致，非本改动引入）；exit 101（设计门）**；35.47s | `m8-green-reg-audio-m6.log` |
| 12 | 回归 `--test merge` | **exit 0；1 passed（0.87s）** | `m8-green-reg-merge.log` |

（#8–#12 各自串行；#4–#12 均在最后一次源码改动之后执行（冻结树）；运行后 `Get-Process ffmpeg,ffplay` 0 残留。）

### 4.1 关键实测值

- **probe 真实 smoke**：`os=10.0.26200.9711 supported=true selected=Some("h264_nvenc") loopback=Ok mic=Unavailable wgc=Ok canvas=7680x4320`；loopback=process loopback 试激活成功（激活即停）；mic=无采集设备（Unavailable）；wgc=Monitor 0 最小会话试激活成功（即时停止）。
- **recovery S1–S5**：manifest 原子写/往返（目录恰含 `manifest.json`+段，无临时残留）；陈旧清单重扫（封口={a0,a1}、尾段=a2 截断段）→ 成品 3.577s 可解码、封口段未改写、尾段入 `discarded` 且不删文件；幂等二跑（root mkv 数不变、成品字节不变）；0 字节/坏 EBML 排除；清单损坏/缺失→`RecoveryFailed`+保留原段+丢弃可用；WAV 残留就地封口（frames=96000，RIFF/data 长度封口、幂等）。
- **recovery 相位 R**：录制中会话 `active=true` 且 `finalize_session` 拒绝（`Active`）；`taskkill /PID <child>`（不带 /T）后 **Job 对象连带回收 sidecar（0 残留 ffmpeg）**；崩溃收尾成品 **10.100s（2 封口段）**可解码、原段保留、幂等、丢弃。**相位 R2**：`taskkill /T /F` 编码进程 → 子进程 `E_SIDECAR_EXIT` 结果 → 同进程恢复收尾成品 10.100s（§3.5 可重试成立）。
- **磁盘满注入**：`icacls /deny (OI)(CI)W` → 该路 **`E_DISK_FULL`**（exit -13；错误明细含 stderr 尾部摘要——"Permission denied" 位于**完整 16KB 尾部**而非最后 3 行，R1 要点实测生效）；失败后封口段保留且可独立读时长；`deny.restore()` 后 `finalize_session` 成功、成品可解码、原段不删。

## 5. 未决项处置（红 §8 / 任务裁定）

| # | 事项 | 处置 |
|---|---|---|
| 5-1 | **cargo-deny 缺失**（红 §8-1） | 按任务指示**未安装**；`license_and_build_audit` 在执行完所有实际审计后按契约 `BLOCKED`（fail，不静默 skip）。缺口：本机 `cargo test --workspace` 因此 exit 101（唯一失败项）。**待编排者确认**是否授权安装 cargo-deny 或裁定豁免口径；实现侧已就绪（deny.toml + 全审计断言通过）。 |
| 5-2 | 恢复与录制集成面（红 §8-2） | **自动维护**：`start_route`/`start_audio_route`/`start_routes` 经 `start_route_impl` 自动建会话目录 + manifest + Job；**每路一会话**（session_id=`{millis}_{seq}`；多路由=各自会话，逐路隔离——红 §8-8 建议「逐路独立、失败隔离」采纳）。`start_audio_only`（REQ-19 纯录音）**未**迁入会话目录（无测试要求、保持 M6 契约；如实记录）。 |
| 5-3 | pid 身份用途收窄（红 §8-3） | `identity` 按 §3.4 记录（pid+创建时间+name+path）；可收尾性判据 = **活动文件独占打开探测**（R2 要点；pid 不参与判定），同进程写失败后立即重试成立（磁盘满用例实测）。 |
| 5-4 | 磁盘满≠真 ENOSPC（红 §8-4） | ACL 写拒绝覆盖 CreateFile 失败类；真满盘（写中途 ENOSPC）仍未验证（同 S5 §12-2 注记）。 |
| 5-5 | 尾段回收策略（红 §8-5） | 取「**弃用不删**」：仅封口段参与 concat；尾段列入 `discarded` 且文件保留（「完成段永不丢弃」；remux 回收未实现）。 |
| 5-6 | WAV 成品语义（红 §8-6） | 就地封口 + `repaired_wavs` 上报；不移动/改名（实现自由面，如实记录）。 |
| 5-7 | probe 三类激活面深度（红 §8-7） | 真实实现：loopback=process loopback 试激活；mic=枚举 + 默认设备 200ms 试录（不可打开→Denied 候选）；wgc=Monitor 0 最小会话试激活（ConsentDenied→Denied）。「激活成功但无信号」检测不在本红范围。 |
| 5-8 | 审计来源锚点（红 §8-10） | 沿用「docs/ 递归任一文本」；实际锚点 = `docs/spikes/s1/s1-pipeline-throughput.md`（含 zip SHA256 全串，M0 既有记录），**未新增文件**。 |
| 5-9 | probe.rs 测试文件 1 条 warning | 受控测试文件含 `unused import: EncoderProbe`（红阶段在 unresolved 状态下未暴露）；**不修改测试文件**（哈希受控），如实记录。 |
| 5-10 | SESSIONNAME 注记（红 §8-11） | L2 以 `$env:SESSIONNAME='Console'` 运行；icacls 注入需 USERNAME（本机 `elvis`）✓。 |

## 6. 关键实现说明

- **probe**：注入语义 = `Some` 的层**整体替身**（不跑真实探测；矩阵用例因此 1.5s 完成）；`selected_encoder` = 链序首个 `init_ok`；OS 支持阈值 = build≥19041（17763→false / 26200→true 已钉）；画布上限保守 7680×4320。
- **recovery 清单**：`manifest.json` 为受限 JSON，序列化/解析为模块内手写实现（保持仓库零外部依赖纪律；字段集即 schema v1，畸形/未知结构/版本≠1 → `RecoveryFailed`）。原子写 = 唯一临时名（pid+seq）+rename，成功后零残留（S1 断言实测）。
- **封口/尾段**：候选=会话目录顶层有效 `*.mkv`（size>0 且 ffprobe 可解析）；封口=清单 `closed` 或存在更高序号有效文件；尾段=最高序号有效未封口。目录重扫与清单并集（陈旧清单不丢段）。
- **活动判定**：`share_mode(0)` 独占打开探测段/WAV——任何在写句柄冲突即 `active`（不杀文件、不修改，`Active` 拒绝收尾）。
- **收尾**：ffconcat（会话目录）→ `-c copy` 到会话目录临时物 → ffprobe 校验 → `safe_commit` 原子提交到 output 根（REQ-24 命名 `{yyyyMMdd_HHmmss}_{sanitize(name)}.mkv`）；成功后 manifest 记 `final_path`（幂等锚点）；原段全程不删。
- **route 会话集成**：段 pattern/ffconcat/清单全部迁入会话目录（`RouteHandle::segments()` 视角不变）；成品仍在输出根——M3–M7 回归（含逐字节/命名/回滚断言）全绿佐证兼容。
- **Job 绑定**：进程级单例 `CreateJobObjectW`+`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`（句柄进程生命周期保持打开）；`spawn_sidecar` 内 best-effort 绑定（失败仅告警，录制继续）。

## 7. 未决问题（如实记录）

1. **cargo-deny 缺口**（同 §5-1）：唯一失败项；`cargo test --workspace` exit 101=BLOCKED 语义（非实现缺陷）。工具安装属新工具引入，待编排者/用户确认。
2. **`license_and_build_audit` 初跑中间态**：首次运行在缺 `deny.toml` 处失败（仓库此前无该配置）；已新增配置后通过至 cargo-deny 分支（两次运行日志均归档）。
3. **probe.rs 测试侧 warning**（§5-9）：受控文件，未改。
4. **真满盘/写中途 ENOSPC**未验证（同 §5-4）；**收尾中崩溃**（「收尾中再崩」）由幂等复用分支设计覆盖但未单独构造用例；**多路由崩溃会话**的并发收尾顺序未实测（每路一目录使该面自然隔离）。
5. **manifest 时间范围为名义段区间**（i×5s..(i+1)×5s）：录制期不做逐段 ffprobe（成本）；恢复路径不使用该字段（以目录重扫+真实段文件为准）。
6. **`start_audio_only` 未接会话/manifest**（§5-2）；WAV 崩溃恢复由 `finalize_session` 的 WAV 路由支路承载（测试合成场景实测）。
7. **start_routes 回滚**：删除成品/段/清单后会话目录壳保留（含 manifest）；未被断言、不影响回滚语义（成品零残留实测于 multi_route 回归）。
8. **probe GUI 线程调用面**：`probe_capabilities` 含 subprocess/WGC/com 激活，设计为 UI 预演/后台调用（Tauri 命令接线属 src-tauri 轮次）。
9. **%TEMP% 残留**：`sr_m8_red_recovery_*` 8 个目录为**红阶段 rehearsal** 会话中间态产物（mtime 02:37–02:42，早于本会话；本会话两轮 recovery 运行均自清理完毕），未入仓库。

## 8. 日志归档（docs/milestones/）

`m8-green-red-pipeline.log` / `m8-green-red-itests.log`（红复现）、`m8-green-check-sr-pipeline.log`、`m8-green-l1-safe-commit.log`、`m8-green-l1-audit.log`（+`-initial.log` 中间态）、`m8-green-l2-probe.log`、`m8-green-l2-recovery.log`、`m8-green-ws.log`（任务原命令）/`m8-green-ws-nff.log`（--no-fail-fast 完整）、`m8-green-reg-{route,multi-route,audio-isolation,audio-m6,merge}.log`。控制台中文在日志中偶现编码折损（cp936↔UTF-8），ASCII 关键值（exit/计数/耗时/哈希/dB/字节）逐字可核。

## 9. 验收结论（对照任务验收项）

| 验收项 | 状态 | 依据 |
|---|---|---|
| 红态复现（4×E0432 + 1×E0599） | ✅ | §3 |
| L1 `cargo test --workspace` | ⚠️ **15 passed / 0 failed，唯一失败 = license_and_build_audit BLOCKED（cargo-deny 工具缺失；任务已裁定照语义执行并记录缺口）** | §4-#6/#7 | 
| `safe_commit` 2 用例 | ✅ | §4-#2（exit 0） |
| `license_and_build_audit` 实际审计面 | ✅（cargo-deny 分项 BLOCKED 记录） | §4-#3 |
| 新 L2 `probe` 全绿 | ✅ | §4-#4（1 passed） |
| 新 L2 `recovery` 全绿（2 用例） | ✅ | §4-#5（2 passed） |
| 回归 route 5/5、multi_route 4/4、audio_isolation 4/4、merge 1/1 | ✅ | §4-#8/#9/#10/#12 |
| 回归 audio_m6（3 绿 + mic 环境 BLOCKED 如实报告） | ✅（环境受限，沿 M6/M7 状态） | §4-#11 |
| 四个受控测试文件哈希复核不变 | ✅ | §1（SHA256 前 16 逐字一致；merge/route 等亦一致） |
| specs/ 未动；未 git 提交；Cargo.lock 未变 | ✅ | §1/§2 |
| 报告落盘且与实际一致 | ✅ | 本文件 + §8 原始日志（15 份） |

**结论**：M8 绿阶段达成——`sr-pipeline` 按红报告 §3 契约（绑定）真实落盘：`probe`（分层探测/注入矩阵/降级）、`recovery`（manifest 原子读写/目录重扫/幂等安全收尾/丢弃/WAV 残留封口/活动文件探测）、`naming`（会话目录/安全提交）、`sidecar`（锁定构建审计 + Kill-on-close Job）、`route`（会话集成 + `DiskFull`）。两条 rehearsal 回写要点（R1 全量 stderr 扫描、R2 活动文件探测）实测生效。新 L2 全绿、回归全绿（audio_m6 mic 沿环境 BLOCKED）；**唯一缺口 = cargo-deny 未安装导致 audit 用例 BLOCKED（任务指示不安装工具；待编排者裁定）**；测试侧 probe.rs 1 条 unused-import warning（受控文件）。建议编排者复核 §4 命令并裁定 §5-1。
