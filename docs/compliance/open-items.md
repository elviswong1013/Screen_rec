# Screen_rec 合规开放项台账

> **草案 — 需中国及目标市场律师审阅，不构成法律意见；主体/联系方式/适用法/管辖待填。**

- 台账版本：0.1-draft
- 整理日期：2026-10-19
- 代码基线：`HEAD 24575f2`
- 当前发行状态：内部测试包，未签名、未清权；**不是正式发行版**
- 状态：`未开始` / `进行中` / `待审阅` / `待集成` / `已完成` / `N/A（需说明）`
- 优先级：P0 = 正式发行/收费前阻断；P1 = 公开发行前/同期；P2 = 规模化/新市场
- 负责人：`[待填：产品/法务/工程/安全/财务]`

> 本台账只记录缺口与证据，不把“内部测试通过”当作“正式发行完成”。已完成项见第 2 节，**不得重复列为未完成**。

## 1. 开放项

| ID | 优先级 | 缺口 | 当前状态 | 证据 | 依赖 | 负责人 | 下一步 |
|---|---|---|---|---|---|---|---|
| OI-01 | P0 | Rust 第三方通知缺失 | 待审阅/待集成 | lane A 在 HEAD `24575f2` 确认 Windows 运行时 162 个第三方 crate，`src-tauri/resources/licenses/` 仅有 FFmpeg 文件；已生成 [third-party-notices.md](third-party-notices.md) 候选 | 法律审阅；安装包/About 集成 | `[待填：工程/法务]` | 按 Windows 运行时 unit graph（非 `Cargo.lock` 全量）复核 notices 覆盖；律师复核 MIT/Apache/BSD/MPL/Unicode/Zlib 通知形式；随包 `licenses/` 集成 |
| OI-02 | P0 | FFmpeg 静态组件通知/源码清单缺失 | 待审阅/待集成 | lane3 报告：随包 `ffmpeg.exe` 静态链接多个第三方库；`LICENSE.txt` 只有 LGPLv3 文本；`libopenh264`/`liblcevc-dec` 仍启用；应用不调用；lane B 已生成 [ffmpeg-components.md](ffmpeg-components.md)、[sbom-ffmpeg.json](sbom-ffmpeg.json) 候选 | 法律审阅；构建方提供依赖清单/许可证；安装包集成 | `[待填：工程/法务]` | 复核逐组件版本/许可证/版权/源码 URL；评估 `libopenh264`/`liblcevc-dec` 的专利/通知义务或在正式构建中移除 |
| OI-03 | P0 | H.264/AVC 专利书面意见/许可 | 未开始 | 内部 lane4 公开条款研究（Via LA AVC；PC Software；前 10 万 unit $0 档；是否需签约/报告不明确） | 法务/IP；Via LA 书面答复；目标市场清单 | `[待填：法务/IP]` | 确认产品是否构成 encoder product/unit、PC Software 类别、协议/报告/费用；取得书面意见，不把公开条款当 FTO |
| OI-04 | P0 | AAC 专利书面意见/许可 | 未开始 | 内部 lane5 研究：项目实际输出 AAC-LC；Via LA AAC 收费触发点为 encoder/decoder 产品销售/供应；公开 FAQ 不足以下结论 | 法务/IP；Via LA 书面答复；目标市场清单 | `[待填：法务/IP]` | 确认 AAC-LC 编码/解码产品是否需许可、PC Software 费率/报告、FFmpeg native AAC 角色；取得书面意见 |
| OI-05 | P0 | EULA 未定稿/未集成 | 待审阅/待集成 | [eula-draft.md](eula-draft.md)；当前安装包无 EULA 接受页；`Cargo.toml` 许可声明未定 | 项目许可证决策（[project-license-decision.md](project-license-decision.md)）；律师；clickwrap 设计 | `[待填：法务/产品]` | 填主体/联系方式/适用法/管辖；律师审阅；集成安装器/首启；归档接受记录 |
| OI-06 | P0 | 隐私政策未定稿/未发布 | 待审阅/待集成 | [privacy-policy-draft.md](privacy-policy-draft.md)；应用/网站/About 均未发布政策 | 律师；主体/代表/DPO；网站/支持/支付场景 | `[待填：法务/隐私]` | 填控制者/代表/联系方式/保留期限/跨境；律师审阅；发布到安装包/About/网站并保持版本一致 |
| OI-07 | P0 | 录制指示/知情提示不足 | **部分完成（M19b1 状态可查询+关闭提示；无浮窗/图标；L3 待签）** | [m19b1-report.md](../milestones/m19b1-report.md)：托盘 tooltip/菜单/主窗标题按 route id 反映 recording/stopping；录制中首次关闭主窗一次提示；纯函数/工作区测试通过 | 开始栏告知（b2）；常驻浮窗/图标；L3 真实托盘/关闭 | `[待填：产品/前端]` | 实施 M19b2；补真实托盘/关闭 L3；再评估浮窗/图标 |
| OI-08 | P0 | 文件名/manifest 含窗口标题 | **部分完成（M19a ACCEPTED；旧会话 UI/托盘/开始提示待后续）** | [m19a-privacy-redaction.md](../milestones/m19a-privacy-redaction.md)：默认 `Window {seq}`，Settings 开关默认 false，manifest/恢复命名同步；旧 API true 保持兼容；acceptor ACCEPTED 2026-10-23 | 旧会话 UI 显示；托盘/开始前提示；安装包重建 | `[待填：工程/产品]` | 后续把恢复列表旧标题显示脱敏；接入托盘/开始提示；重建内部测试包 |
| OI-09 | P0 | stderr 日志可能含路径/标题 | 未开始 | 设置解析失败打印完整路径；收尾/ffprobe/恢复错误常带成品/分段绝对路径；未统一脱敏 | 日志规范；工程实现 | `[待填：工程]` | 错误串只保留 basename/哈希/`<redacted>`；文档说明 stderr 可能被父进程捕获；检查 `SR_DIAG_AV` 输出 |
| OI-10 | P0 | 代码签名缺失 | 未开始 | 安装包 Authenticode `NotSigned`；SHA-256 已记录但未公开 | 证书/HSM/预算；发布流程 | `[待填：发布工程/安全]` | 采购 OV/EV 证书；HSM/云签名；SHA-256 + RFC 3161 时间戳；签名验证；发布校验值 |
| OI-11 | P0 | 海外市场未清权 | 未开始 | 内部 lane8：美国州级 all-party 差异、EU 员工同意限制、UK/CA/AU/JP 独立规则；未指定首发国家 | 商务市场清单；当地律师 | `[待填：商务/法务]` | 确定首发/暂缓市场；逐市场隐私/劳动/著作权/消费者/出口审查；准备本地化文本/代表 |
| OI-12 | P0 | About/许可证入口/安装包集成缺失 | 未开始 | UI/代码检索未发现 About/版本/许可证/第三方通知入口；`tauri.conf.json` 仅打包 FFmpeg licenses，不含 notices/SBOM/EULA/隐私 | UI 设计；打包资源清单；lane A/B 文件 | `[待填：前端/发布工程]` | 增加 About 页：版本、EULA、隐私、notices、licenses、source offer；安装包 `licenses/` 同步打包 |
| OI-13 | P0 | 第三方通知/许可证文本/SBOM 未集成 | 待审阅/待集成 | lane A 已生成 [third-party-notices.md](third-party-notices.md)、[third-party-licenses.txt](third-party-licenses.txt)、[sbom-rust.json](sbom-rust.json)；lane B 已生成 [ffmpeg-components.md](ffmpeg-components.md)、[sbom-ffmpeg.json](sbom-ffmpeg.json)；均未集成安装包/About | 打包配置；法律审阅 | `[待填：工程/发布]` | 复核候选内容；加入安装包 `licenses/`、About/网站；版本与构建 commit 对应 |
| OI-14 | P0 | FFmpeg source offer 期限/联系人/安装信息待确认 | 进行中 | [ffmpeg-source-offer.txt](../../src-tauri/resources/licenses/ffmpeg-source-offer.txt) 已有 commit/SHA/archive/构建脚本；缺 3 年/至 EOL 书面承诺、联系人、介质成本、LGPLv3 安装信息律师确认；`ffmpeg-build-info.txt` 仍写镜像 `:latest`，与 source-offer 的 digest 不一致 | 律师；发布/支持流程 | `[待填：法务/工程]` | 补齐书面 offer 文本；提供稳定 URL/联系人；确认“对应源码”与可重链接材料；下次重建时把 build-info 固定为 source-offer 的 digest 并更新哈希；随每次升级更新 |
| OI-15 | P1 | 清理/保留入口不足 | 未开始 | 应用无成品自动过期；`.screenrec_tmp` 失败/崩溃残留；无临时文件占用/一键清理 UI；`keep_segments` 为调试选项但缺显著警告 | 产品/工程设计 | `[待填：产品/工程]` | 设置页显示占用/残留；一键清理；保留期限提示；文档说明默认卸载不删成品 |
| OI-16 | P1 | WebView2 数据/卸载披露不足 | 进行中 | 卸载器有可选“删除应用数据”，但静默卸载默认不勾选；`EBWebView` 不清理；文档未充分披露 | 安装器/文档 | `[待填：产品/发布]` | 安装/卸载页说明；评估清理 `EBWebView`；隐私政策/帮助中心同步 |
| OI-17 | P1 | CSP/IPC 安全加固 | 未开始 | `tauri.conf.json` `withGlobalTauri: true`、`csp: null`；本地页面注入可调用应用命令（未发现已确认注入点，属纵深防御缺口） | 安全/工程 | `[待填：安全/工程]` | 增加严格 CSP；限制 IPC 来源/命令；评估移除 `withGlobalTauri`；保持输入转义 |
| OI-18 | P1 | 干净机/AV/SmartScreen 验证 | 未开始 | 内部包安装/自检在同机完成；未做干净 VM、AV 误报、SmartScreen 信誉测试 | QA 环境；签名 | `[待填：QA/安全]` | 干净 Windows 10 2004+/11 x64 矩阵；Defender/主流 AV；签名后 SmartScreen 观察 |
| OI-19 | P1 | 企业版材料/DPA/DPIA 模板 | 未开始 | 当前无 DPA、PIA/DPIA 模板、权限矩阵、员工/客户告知模板、审计日志说明 | 企业产品/法务 | `[待填：法务/企业产品]` | 按目标客户准备 DPA、DPIA、告知/同意模板、保留/访问矩阵；行业场景另审 |
| OI-20 | P1 | 名称/商标检索 | 未开始 | `Screen_rec`/`ScreenRec` 描述性强；未做商标/商号/域名/应用商店 clearance | 法务/IP；预算 | `[待填：法务/IP]` | CNIPA/USPTO/EUIPO 等 knockout/clearance；评估改名/注册策略；保留检索报告 |
| OI-21 | P1 | 出口/制裁审查 | 未开始 | EULA 有占位条款；未做软件分类/客户/市场筛查 | 贸易合规/法务 | `[待填：法务/贸易合规]` | 评估 EAR/OFAC/EU/中国规则；准备最终用户/用途筛查与文档 |
| OI-22 | P2 | 中国软著登记（可选/建议） | 未开始 | 未登记；企业采购/投标/维权常用 | 法务/行政；版本号 | `[待填：法务/行政]` | 以发行版本号申请登记；保留源码/文档样本；升级评估新登记 |
| OI-23 | P2 | 退款/撤回权/订阅流程（如收费） | 未开始 | 收费模式未定；EULA 仅有占位 | 商务/法务/支付商 | `[待填：商务/法务]` | 中国 7 天、EU/UK 14 天、美国承诺、订阅取消；支付商/MoR 角色 |
| OI-24 | P2 | 网站/ICP/公安备案/税务（如自建站/收费） | 未开始 | 未确定是否自建下载/支付/客服 | 商务/财务 | `[待填：商务/财务]` | 评估 ICP/公安备案/MLPS/发票/税务；或使用合规 MoR/平台 |
| OI-25 | P2 | 安全研究/VDP/投诉响应流程 | 未开始 | 无公开安全联系/VDP；无投诉/执法请求 SLA | 安全/法务/支持 | `[待填：安全/法务]` | 发布 security@/privacy@；VDP safe harbor；投诉/泄露/执法请求手册 |

## 2. 已完成项（不得重复列为未完成）

| 项 | 状态 | 证据 | 说明 |
|---|---|---|---|
| FFmpeg `chromaprint`/FFTW（GPLv2+）移除 | 已完成 | [ffmpeg-lgpl-remediation.md](../milestones/ffmpeg-lgpl-remediation.md)；[ffmpeg-build-info.txt](../../src-tauri/resources/licenses/ffmpeg-build-info.txt) | 仅从 BtbN LGPL 配置移除 `--enable-chromaprint`；`-buildconf` 无 `chromaprint`/`fftw`；`-muxers` 无 chromaprint |
| GPL/nonfree 组件禁用 | 已完成 | [ffmpeg-build-info.txt](../../src-tauri/resources/licenses/ffmpeg-build-info.txt) | 无 `--enable-gpl`/`--enable-nonfree`；`libx264/libx265/libxavs2/libxvid` 禁用；`libfdk-aac` 禁用 |
| FFmpeg 定制构建与 source offer 更新 | 已完成（工程侧） | [ffmpeg-source-offer.txt](../../src-tauri/resources/licenses/ffmpeg-source-offer.txt)；[ffmpeg-build-script.sh](../../src-tauri/resources/licenses/ffmpeg-build-script.sh) | commit `46d899d7a02bc63237472cc4897258d5ddf274ac`；构建日期 2026-10-19；SHA-256/镜像 digest/构建脚本；律师确认仍见 OI-14 |
| 内部测试包重建/安装/自检/卸载 | 已完成（内部测试） | [ffmpeg-lgpl-package-report.md](../milestones/ffmpeg-lgpl-package-report.md) | 安装退出码 0；自检退出码 0；卸载退出码 0；安装包 SHA-256 `F3A42C4E…B14F2F`；**未签名，不是正式发行** |
| M18 源码验收 | 已完成（源码验收） | [m18-report.md](../milestones/m18-report.md) | acceptor 复审 ACCEPTED；麦克风/真实 96k/播放器矩阵等 BLOCKED 项仍如实记录 |
| 安装包哈希记录 | 已完成（工程记录） | [ffmpeg-lgpl-package-report.md](../milestones/ffmpeg-lgpl-package-report.md) | 安装包/release exe/安装目录文件哈希；公开校验值仍见 OI-10/G4-03 |
| 第三方许可证文本（FFmpeg LGPL/GPLv3） | 已完成（随包） | `src-tauri/resources/licenses/FFmpeg-LICENSE.txt`、`GPLv3.txt` | 仅覆盖 FFmpeg 文本；Rust/其他组件通知仍见 OI-01/OI-02/OI-13 |

> 注意：`libopenh264`、`liblcevc-dec` 等静态组件并未因 chromaprint/FFTW 修复而自动清权；它们仍属于 OI-02/OI-03 的开放范围。

## 3. 发布门槛摘要

- **P0 全部关闭** + G4 全部通过 + 负责人/法务/安全书面签核，才可称为正式发行。
- 仅完成 G0 工程自检，不构成对 EULA/隐私/第三方通知/专利/市场清权的完成。
- 任何“未签名内部测试包”不得以“正式版/稳定版/可商用/完成全部审查”对外描述。
- 若某项因商业模式决定为 N/A（如不收费则退款条款），需在对应文档写明依据与批准人。

## 4. 更新规则

- 每次代码、依赖、构建、安装器、数据流、目标市场变化时更新本台账。
- 新缺口必须有 ID、证据、owner、下一步；关闭时保留证据链接与日期，不删除历史。
- 与 [release-checklist.md](release-checklist.md) 的 G0–G4 编号交叉引用；避免同一事项在两处状态不一致。
- 第三方 notices/SBOM/source offer 的实际文件由 lane A/B 维护；本台账只记录状态与依赖。

---

**再次声明**：本台账不是法律意见，也不表示任何缺口已经通过法律审阅。主体、联系方式、适用法与管辖待填；正式发行前必须完成 P0 与 G4 门禁。
