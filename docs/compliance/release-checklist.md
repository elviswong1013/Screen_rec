# Screen_rec 发行检查清单（G0–G4）

> **草案 — 需中国及目标市场律师审阅，不构成法律意见；主体/联系方式/适用法/管辖待填。**

- 清单版本：0.1-draft
- 整理日期：2026-10-19
- 代码基线：`HEAD 24575f2`
- 当前安装包：`Screen_rec_0.1.0_x64-setup.exe`，SHA-256 `F3A42C4E304FAECCFBF2410A58A4ACB6D486DB3443E510C4E8B252FEF3B14F2F`
- 当前性质：**内部测试包；未签名；未完成法律/专利/第三方通知清权**
- 状态标记：`[x]` 仓库内已有证据；`[~]` 部分完成/材料存在但未审阅或未集成；`[ ]` 未完成；`N/A` 有依据的不适用（需在备注说明）

> **放行总则：未完成项不得宣称正式发行。** 任何市场、任何渠道（官网、商店、企业交付、试用）在 G4 全部通过并由负责人书面批准前，只能标注为“内部测试/候选包”，不得使用“正式版/稳定版/可商用/完成全部审查”等表述。

## G0 内部测试基线

- [x] **G0-01 M18 源码验收**：M18（P4 区域视频）已由 acceptor 复审 ACCEPTED（源码验收）；已随 FFmpeg LGPL 整改内部测试包重建。证据：[m18-report.md](../milestones/m18-report.md)。owner：`[待填：工程负责人]`。
- [x] **G0-02 FFmpeg GPL 冲突整改**：移除 `chromaprint`/FFTW（GPLv2+）；`-buildconf` 无 `chromaprint`/`fftw`；无 `--enable-gpl`/`--enable-nonfree`；`libx264/libx265/libxavs2/libxvid` 禁用。证据：[ffmpeg-lgpl-remediation.md](../milestones/ffmpeg-lgpl-remediation.md)、[ffmpeg-build-info.txt](../../src-tauri/resources/licenses/ffmpeg-build-info.txt)。owner：`[待填：工程/构建]`。
- [x] **G0-03 内部测试包重建与安装自检**：NSIS 安装退出码 0；安装目录自检退出码 0；`ffmpeg/ffprobe` 命中安装目录；卸载退出码 0。证据：[ffmpeg-lgpl-package-report.md](../milestones/ffmpeg-lgpl-package-report.md)。owner：`[待填：打包/QA]`。
- [x] **G0-04 媒体/回归测试**：整改提交记录“审计/媒体回归通过”；M18 相关测试与日志已归档。证据：git `0d7ac80`、`docs/milestones/m18-*.log`。owner：`[待填：QA]`。
- [x] **G0-05 已知限制记录**：M18 报告如实记录麦克风正路径、真实 96k 捕获、多视频轨播放器矩阵等 BLOCKED/待签项，未用 SKIP 冒充通过。证据：[m18-report.md](../milestones/m18-report.md)、[l3-signoff.md](../milestones/l3-signoff.md)。owner：`[待填：产品/QA]`。
- [x] **G0-06 内部包哈希归档**：安装包 SHA-256、release 可执行文件哈希、安装目录文件哈希已记录在打包报告。证据：[ffmpeg-lgpl-package-report.md](../milestones/ffmpeg-lgpl-package-report.md)。owner：`[待填：发布工程]`。
- [ ] **G0-07 干净机安装**：在未安装过本应用、无开发环境、无预置 FFmpeg/WebView2 的干净 Windows 10 2004+/11 x64 机器上安装、录制、卸载。证据：`[待生成：CLM-01 日志/截图]`。owner：`[待填：QA]`。
- [ ] **G0-08 恶意软件/杀软误报检查**：Defender 及至少一款主流 AV 扫描安装包/安装目录；记录误报与处置。证据：`[待生成：AV 扫描记录]`。owner：`[待填：安全/QA]`。
- [ ] **G0-09 内部测试使用条款**：在 EULA 未定稿前，为内部测试者提供简短的内部评估/保密条款（不作为正式 EULA）。证据：`[待生成]`。owner：`[待填：法务/产品]`。

## G1 文档与第三方材料

- [~] **G1-01 第三方通知**：lane A 已生成 [third-party-notices.md](third-party-notices.md) 候选；需复核是否覆盖 Tauri、Rust crates、`windows` crate、NSIS、WebView2 等（FFmpeg 另见 G1-04），并逐项版本/许可证/版权/源码 URL；**尚未法律审阅/集成**。owner：`[待填：工程/法务]`。
- [~] **G1-02 第三方许可证文本**：lane A 已生成 [third-party-licenses.txt](third-party-licenses.txt) 候选；需复核随包 `licenses/` 是否提供、文本是否与实际版本一致；**尚未审阅/集成**。owner：`[待填：工程/法务]`。
- [~] **G1-03 Rust SBOM**：lane A 已生成 [sbom-rust.json](sbom-rust.json) 候选；需复核是否覆盖 Windows 运行时依赖（不是 `Cargo.lock` 全量）与构建 commit；**尚未审阅/集成**。owner：`[待填：工程]`。
- [~] **G1-04 FFmpeg 组件清单**：lane B 已生成 [ffmpeg-components.md](ffmpeg-components.md) 候选；需复核是否列明静态组件、许可证、通知/源码缺口（含 `libopenh264`、`liblcevc-dec` 等）；**尚未法律审阅/集成**。owner：`[待填：工程/法务]`。
- [~] **G1-05 FFmpeg SBOM**：lane B 已生成 [sbom-ffmpeg.json](sbom-ffmpeg.json) 候选；需复核是否对应实际随包构建（commit、配置、组件）；**尚未审阅/集成**。owner：`[待填：工程]`。
- [~] **G1-06 FFmpeg 源码 offer**：`ffmpeg-source-offer.txt` 已给出 commit `46d899d7a02bc63237472cc4897258d5ddf274ac`、SHA-256、GitHub archive、构建脚本与镜像 digest；**尚缺**至少 3 年/至 EOL 的书面承诺、联系人、介质成本、律师对 LGPLv3 安装信息的确认。证据：[ffmpeg-source-offer.txt](../../src-tauri/resources/licenses/ffmpeg-source-offer.txt)、[ffmpeg-build-info.txt](../../src-tauri/resources/licenses/ffmpeg-build-info.txt)。owner：`[待填：法务/工程]`。
- [~] **G1-07 EULA**：草案已整理（授权范围、禁止用途、用户内容、LGPL 权利、免责/责任限制 carve-out、终止、争议解决占位）；**尚缺**律师审阅、主体/联系方式/适用法/管辖填写、clickwrap 接受记录、安装包/About 集成。证据：[eula-draft.md](eula-draft.md)。owner：`[待填：法务/产品]`。
- [~] **G1-08 隐私政策**：草案已整理（本地处理、落盘、保留/清理、共享/跨境、安全限制、儿童/敏感信息）；**尚缺**律师审阅、主体/联系方式/代表/DPO、网站/支持/支付场景、发布位置与版本一致性。证据：[privacy-policy-draft.md](privacy-policy-draft.md)。owner：`[待填：法务/隐私]`。
- [~] **G1-09 录制使用指南**：草案已整理（告知/同意、员工/会议、著作权/DRM、中国/美国/EU 高层提示）；**尚缺**律师审阅、企业版模板/DPA、海外本地化。证据：[recording-usage-guidelines.md](recording-usage-guidelines.md)。owner：`[待填：法务/客服]`。
- [ ] **G1-10 发行说明与已知问题**：面向用户的版本说明、已知限制（BLOCKED 项、编码器/播放器兼容性、无自动更新、未签名）。证据：`[待生成]`。owner：`[待填：产品]`。
- [ ] **G1-11 About/版本/许可证入口**：应用内 About 页展示版本、EULA、隐私政策、第三方通知、源码 offer 链接/路径。当前 UI 未发现 About/许可证入口。证据：`ui/`、`src-tauri/src/lib.rs` 检索；`[待生成：UI 设计]`。owner：`[待填：前端/产品]`。
- [ ] **G1-12 中英文版本一致性**：如面向海外，EULA/隐私/指南需有英文版并保持实质一致；语言优先级明确。证据：`[待生成]`。owner：`[待填：法务/本地化]`。
- [ ] **G1-13 目标市场清单**：明确首发/暂缓国家/地区；逐市场确认隐私、劳动、著作权、消费者、出口/制裁要求。证据：`[待生成：市场清单]`。owner：`[待填：商务/法务]`。
- [ ] **G1-14 名称/商标检索**：`Screen_rec`/`ScreenRec` 描述性强，需做商标/商号/域名/应用商店 knockout/clearance。证据：`[待生成：检索报告]`。owner：`[待填：法务/IP]`。
- [ ] **G1-15 中国软件著作权登记（可选/建议）**：自愿登记，但企业采购/投标/维权常用；版本号需与发行版本一致。证据：`[待生成：登记证书/受理]`。owner：`[待填：法务/行政]`。
- [ ] **G1-16 出口/制裁审查**：EULA 条款、目标市场/客户筛查、加密/软件分类评估。证据：`[待生成：出口合规备忘]`。owner：`[待填：法务/贸易合规]`。
- [ ] **G1-17 退款/撤回权政策（如收费）**：中国 7 天、EU/UK 14 天、美国承诺、订阅取消；支付商/MoR 角色。证据：`[待生成]`。owner：`[待填：商务/法务]`。

## G2 产品措施（隐私/安全/知情）

- [~] **G2-01 持续录制指示**：M19b1 已实现托盘 tooltip/菜单/主窗标题按 route id 反映 recording/stopping，录制中首次关闭主窗有一次提示。**仍缺**常驻可见的浮窗/图标；L3 真实托盘/关闭已于 2026-10-23 签核。证据：[m19b1-report.md](../milestones/m19b1-report.md)、[lib.rs](../../src-tauri/src/lib.rs)。owner：`[待填：产品/前端]`。
- [~] **G2-02 开始前目标/音源摘要与第三方提示**：M19b2 已实现开始栏摘要（目标/音源/输出/格式）、敏感音源（mic/system/render_endpoint）首次两击告知、ack 勾选/取消与设置页保留；UI runner/四 runner 通过。真实 Tauri/L3 已于 2026-10-23 签核；托盘“开始”路径不在本切片告知范围。证据：[m19b2-report.md](../milestones/m19b2-report.md)、[lib.rs](../../src-tauri/src/lib.rs)。owner：`[待填：产品/前端]`。
- [~] **G2-03 文件名/manifest 脱敏**：默认 false（`Window {seq}`）已实现；manifest `identity.name` 与恢复成品名同步；设置开关 `include_window_title_in_filename` 可回退旧行为；旧 API true 兼容。**尚缺**旧会话 UI 标题显示脱敏、安装包重建。证据：[m19a-privacy-redaction.md](../milestones/m19a-privacy-redaction.md)、[privacy-policy-draft.md](privacy-policy-draft.md)。owner：`[待填：工程/产品]`。
- [~] **G2-04 日志脱敏**：默认 stderr/错误串已接入 `sr-core::redact`，保留 basename/会话 ID/E_*；`SR_DIAG_PATHS=1` 显式恢复诊断路径。workspace/L2 recovery 通过；**独立 acceptor 两次 403，未签**；UI 绑定错误的 opt-in 语义仍需确认。证据：[m19c-report.md](../milestones/m19c-report.md)。owner：`[待填：工程/验收]`。
- [ ] **G2-05 保留/清理入口**：设置页显示 `.screenrec_tmp` 占用/未完成会话；提供一键清理；明确 `keep_segments` 是调试选项；失败/崩溃残留有提示。证据：`[待生成：设计/实现]`。owner：`[待填：产品/工程]`。
- [~] **G2-06 卸载数据提示**：卸载器已有“删除应用数据”可选复选框，且不删除输出目录；**尚缺**应用内/文档明确说明静默卸载默认保留设置、成品永不自动删除、WebView2 数据不清理。证据：[ffmpeg-lgpl-package-report.md](../milestones/ffmpeg-lgpl-package-report.md)、[privacy-policy-draft.md](privacy-policy-draft.md)。owner：`[待填：产品/安装器]`。
- [ ] **G2-07 CSP/IPC 收紧**：为 `tauri.conf.json` 增加严格 CSP；评估移除 `withGlobalTauri` 或限制 IPC 来源/命令；保持输入转义。当前 `withGlobalTauri: true`、`csp: null`。证据：[tauri.conf.json](../../src-tauri/tauri.conf.json)、[privacy-policy-draft.md](privacy-policy-draft.md)。owner：`[待填：安全/工程]`。
- [ ] **G2-08 WebView2 数据管理**：提供清除缓存/崩溃数据入口或明确说明；评估卸载时清理 `EBWebView`；披露 Microsoft 处理。证据：`[待生成：设计/实现]`。owner：`[待填：工程/产品]`。
- [ ] **G2-09 敏感内容保护选项（建议）**：区域录制默认值、通知抑制提示、后期遮蔽/脱敏指引；不宣称自动脱敏。证据：`[待生成：设计]`。owner：`[待填：产品]`。
- [ ] **G2-10 安装器 WebView2 披露/离线模式**：在安装页/文档披露可能从 Microsoft 下载；评估 `offlineInstaller`/`embedBootstrapper` 或企业离线包。证据：[ffmpeg-lgpl-package-report.md](../milestones/ffmpeg-lgpl-package-report.md)、`[待生成：安装器文档]`。owner：`[待填：发布工程]`。
- [ ] **G2-11 安全评审/威胁模型**：覆盖本地 WebView、IPC、FFmpeg sidecar 路径、`SCREENREC_FFMPEG_DIR`、录制/合并文件校验、删除会话边界。证据：`[待生成：安全评审报告]`。owner：`[待填：安全]`。
- [ ] **G2-12 企业版材料（如适用）**：DPA、PIA/DPIA 模板、告知/同意模板、权限矩阵、保留策略、审计日志说明。证据：`[待生成]`。owner：`[待填：法务/企业产品]`。

## G3 专利与编码格式

- [ ] **G3-01 AVC/H.264 书面意见/许可**：确认产品是否构成 AVC encoder product/unit、PC Software 类别、Via LA 协议/报告义务、前 10 万 unit 的 $0 档是否仍需签约。当前仅有公开条款研究，不是 FTO 结论。证据：内部 lane4 研究（2026-10-19）、[Via LA AVC](https://www.via-la.com/licensing-programs/avc-h-264/)。owner：`[待填：法务/IP]`。
- [ ] **G3-02 AAC 书面意见/许可**：确认 AAC-LC 编码/解码产品是否需 Via LA AAC 许可、PC Software 费率/报告、FFmpeg native AAC 的角色。当前公开资料不足以下“欠费/不欠费”结论。证据：内部 lane5 研究（2026-10-19）、[Via LA AAC](https://www.via-la.com/aac/)。owner：`[待填：法务/IP]`。
- [ ] **G3-03 `libopenh264`/`liblcevc-dec` 组件审查**：二者随 FFmpeg 静态构建但应用不调用；需确认许可证通知、专利许可/版税、源码清单与是否应移除。证据：[ffmpeg-lgpl-remediation.md](../milestones/ffmpeg-lgpl-remediation.md)、`[待生成：legal review]`。owner：`[待填：法务/工程]`。
- [ ] **G3-04 DRM/HDCP 立场**：确认不提供绕过/去水印/解密；文档如实说明受保护内容可能黑帧；不宣传录制 Netflix/付费课程。证据：[recording-usage-guidelines.md](recording-usage-guidelines.md)。owner：`[待填：法务/产品]`。
- [ ] **G3-05 目标市场专利清单**：中国/美国/EU/英国/加拿大/澳大利亚/日本等销售/供应地的专利有效性、许可覆盖与报告义务。证据：`[待生成：市场/专利清单]`。owner：`[待填：IP/商务]`。
- [ ] **G3-06 许可费/报告流程（如签约）**：记录销量、unit 口径、年度重置、关联实体、报告/审计义务。证据：`[待生成：许可协议/流程]`。owner：`[待填：财务/法务]`。

## G4 签名、发布与放行

- [ ] **G4-01 代码签名证书与密钥管理**：OV/EV 证书、HSM/云签名、双人审批、吊销/续期流程；证书主体与 EULA/About 一致。证据：`[待生成：证书/密钥制度]`。owner：`[待填：发布工程/安全]`。
- [ ] **G4-02 签名与时间戳**：对安装包与主要可执行文件签名；SHA-256 + RFC 3161 时间戳；验证 Authenticode。当前安装包 `NotSigned`。证据：[ffmpeg-lgpl-package-report.md](../milestones/ffmpeg-lgpl-package-report.md)、`[待生成：签名验证日志]`。owner：`[待填：发布工程]`。
- [ ] **G4-03 公开校验值**：官网/发行页发布 SHA-256、签名指纹、版本/日期；与安装包一致。证据：`[待生成：发布页]`。owner：`[待填：发布/市场]`。
- [ ] **G4-04 干净机安装/升级/卸载矩阵**：Windows 10 2004+/11 x64；无 WebView2/有 WebView2；标准用户/管理员；升级/回滚；静默安装/卸载。证据：`[待生成：CLM-02 矩阵]`。owner：`[待填：QA]`。
- [ ] **G4-05 SmartScreen/杀软信誉检查**：签名后观察 SmartScreen/Defender/主流 AV；记录误报与申诉。证据：`[待生成]`。owner：`[待填：安全/QA]`。
- [ ] **G4-06 安装器联网/离线行为验证**：抓包或隔离环境验证 WebView2 下载、失败提示、离线安装路径；隐私政策/安装页披露一致。证据：`[待生成：网络验证记录]`。owner：`[待填：QA/隐私]`。
- [ ] **G4-07 法务/产品/安全签核**：EULA、隐私、第三方通知、SBOM、源码 offer、专利意见、市场清单、录制指示/脱敏、签名全部通过。证据：`[待生成：签核记录]`。owner：`[待填：负责人/法务/安全]`。
- [ ] **G4-08 发布证据归档**：源码 commit/tag、构建环境、SBOM、notices、licenses、安装包哈希、签名验证、测试日志、审批记录。证据：`[待生成：release dossier]`。owner：`[待填：发布工程]`。
- [ ] **G4-09 市场发布清单**：逐市场确认可发布/暂缓；海外代表/税务/本地化/进口商要求。证据：`[待生成：市场放行表]`。owner：`[待填：商务/法务]`。
- [ ] **G4-10 发布后监控与响应**：崩溃/投诉/隐私请求/安全漏洞/侵权通知/监管问询渠道；版本回滚与撤回流程。证据：`[待生成：响应手册]`。owner：`[待填：支持/安全/法务]`。

## 当前结论

- 当前安装包仅可用于**内部测试**；G0 大部分工程项有证据，但 G0-07/08/09 未完成。
- G1 文档中 EULA/隐私/指南只有草案；Rust notices/licenses/SBOM 与 FFmpeg 组件清单/SBOM 已有 lane A/B 候选，但均未审阅/集成；源码 offer 仍需律师确认期限/联系人/安装信息。
- G2 录制指示、文件名/manifest 脱敏、日志脱敏、CSP/IPC、清理入口均未完成；这些是隐私/安全的高优先级产品措施。
- G3 AVC/AAC 专利与 `libopenh264`/`liblcevc-dec`（LCEVC）组件尚未取得书面意见。
- G4 签名、公开校验值、干净机矩阵、法务签核均未完成。
- **在 G4 全部通过前，不得对外宣称正式发行、可商用、已清权或满足任何市场的全部法律要求。**

---

**再次声明**：本清单是工程门禁草案，不是法律意见。未完成项不得以“内部测试通过”替代；任何放行决定需负责人、法务与安全共同书面确认。
