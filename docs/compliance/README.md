# Screen_rec 工程合规文档索引

> **草案 — 需中国及目标市场律师审阅，不构成法律意见；主体/联系方式/适用法/管辖待填。**

本目录保存 Screen_rec 的工程合规草案、发行检查清单与开放缺口台账。**本目录所有文档均为草案/候选材料，不是法律意见，也不构成任何发行许可。**这里的“草案/候选”只表示工程侧已整理的文本或证据，**不表示已经通过法律审阅，也不表示可以在任何市场正式发行**。

- 文档版本：0.1-draft
- 整理日期：2026-10-19
- 代码基线：仓库 `HEAD 24575f2`（M18 源码验收 + FFmpeg LGPL 整改内部测试包）
- 内部测试包：`Screen_rec_0.1.0_x64-setup.exe`，SHA-256 `F3A42C4E304FAECCFBF2410A58A4ACB6D486DB3443E510C4E8B252FEF3B14F2F`，**未签名**
- 适用对象：工程、产品、法务、发行与企业管理读者
- 状态标记：`草案` = 需律师审阅；`候选` = 工程候选材料，需复核；`待生成` = 由其他 lane 产出，当前索引保留位置

## 1. 本目录文档

| 文档 | 用途 | 状态 | 主要读者 |
|---|---|---|---|
| [eula-draft.md](eula-draft.md) | 最终用户许可协议草案：授权范围、禁止用途、用户内容、开源组件、免责与责任限制、终止、争议解决 | 草案 | 法务、产品、发行 |
| [privacy-policy-draft.md](privacy-policy-draft.md) | 隐私政策草案：本地处理事实、落盘位置、保留/清理、用户控制、共享/跨境、安全限制、联系方式 | 草案 | 法务、隐私、产品、工程 |
| [recording-usage-guidelines.md](recording-usage-guidelines.md) | 面向用户/企业管理员的高层录制指南：告知与同意、员工/客户/会议场景、著作权/DRM、保留期限、法域提示 | 草案 | 用户、企业管理员、客服、法务 |
| [release-checklist.md](release-checklist.md) | G0–G4 发行门禁清单：证据、owner、未完成项 | 工程清单 | 产品、发行、工程、法务 |
| [project-license-decision.md](project-license-decision.md) | 项目自身许可证决策备忘：MIT/Apache、专有、双许可/开放核心三种方案与待决问题 | 决策备忘 | 负责人、法务、商务 |
| [open-items.md](open-items.md) | 剩余缺口台账：状态、证据、依赖、owner、下一步 | 工程台账 | 产品、发行、法务、工程 |

## 2. 第三方与 SBOM 材料（其他 lane 维护）

以下文件名按 lane A/B 约定保留；本索引不复制其内容。五个文件均已生成“候选”版本，但仍需法律审阅、内容复核与安装包集成；不得把候选当作最终通知/SBOM。

| 文件 | 预期内容 | 状态 |
|---|---|---|
| [third-party-notices.md](third-party-notices.md) | Rust/Tauri/Windows crate 等第三方通知候选（具体覆盖范围以文件为准） | 候选（lane A 已生成，需法律审阅/集成） |
| [third-party-licenses.txt](third-party-licenses.txt) | Rust 依赖许可证文本或获取说明 | 候选（lane A 已生成，需法律审阅/集成） |
| [sbom-rust.json](sbom-rust.json) | Rust 运行时依赖 SBOM（建议 CycloneDX/SPDX） | 候选（lane A 已生成，需复核） |
| [ffmpeg-components.md](ffmpeg-components.md) | 随包 FFmpeg/ffprobe 静态组件、许可证、源码/通知缺口 | 候选（lane B 已生成，需法律审阅/集成） |
| [sbom-ffmpeg.json](sbom-ffmpeg.json) | FFmpeg 构建与组件 SBOM | 候选（lane B 已生成，需复核） |

配套的仓库内材料：

- [FFmpeg 源码获取说明](../../src-tauri/resources/licenses/ffmpeg-source-offer.txt)
- [FFmpeg 构建信息](../../src-tauri/resources/licenses/ffmpeg-build-info.txt)
- [FFmpeg 构建脚本](../../src-tauri/resources/licenses/ffmpeg-build-script.sh)
- [FFmpeg LGPL 整改报告](../milestones/ffmpeg-lgpl-remediation.md)
- [内部测试包打包报告](../milestones/ffmpeg-lgpl-package-report.md)

## 3. 建议阅读顺序

- **负责人/发行**：先读 [release-checklist.md](release-checklist.md) 与 [open-items.md](open-items.md)，再读 [project-license-decision.md](project-license-decision.md) 做许可证与商业模式决策。
- **法务/隐私**：先读 [privacy-policy-draft.md](privacy-policy-draft.md) 与 [eula-draft.md](eula-draft.md)，再用 [recording-usage-guidelines.md](recording-usage-guidelines.md) 校对用户场景。
- **工程**：先读 [release-checklist.md](release-checklist.md) 的 G0/G1/G2，再对照 `third-party-*`、`sbom-*`、`ffmpeg-components.md` 与源码获取说明补齐材料。
- **企业客户**：先读 [recording-usage-guidelines.md](recording-usage-guidelines.md)，再按 [open-items.md](open-items.md) 确认录制指示、保留/删除、审计与支持边界。

## 4. 事实基础与边界

本目录的工程事实来自仓库代码、既有里程碑报告与内部法律研究底稿（2026-10-19）。关键边界如下：

1. **本地处理**：录制、截图、编码、合并由本机完成；应用业务代码未发现账号、云上传、分享链接、遥测/分析、崩溃上报或自动更新功能。该结论是**静态代码检索结论**，未做抓包或运行时网络验证。
2. **安装器可能联网**：WebView2 运行时缺失时，NSIS 安装器可能从 Microsoft 下载 bootstrapper；WebView2/Edge/Windows 自身也可能产生证书、更新或诊断类网络行为。不能把产品描述为“绝对不产生任何网络请求”。
3. **敏感元数据**：录制成品文件名可能包含窗口标题；`.screenrec_tmp` 会话的 `manifest.json` 可包含目标名、录制器 PID、录制器 exe 路径、分段绝对路径与时间。日志可能包含路径/标题。
4. **无应用级加密/ACL**：输出文件与设置依赖目标目录继承的 NTFS ACL；应用未实现 DPAPI/EFS/应用级访问控制。
5. **默认卸载不删成品**：卸载器默认只删安装目录；确认页可选删除应用数据；静默卸载默认不勾选；输出目录中的录制/截图不会被卸载器删除。
6. **当前只能作为内部测试包**：安装包未签名；EULA/隐私政策/第三方通知/SBOM 尚未完成审阅与集成；AVC/AAC 专利与海外市场尚未清权。任何“可正式发行”的表述都必须等 [release-checklist.md](release-checklist.md) 的 G4 门禁完成并由负责人书面批准。

## 5. 使用与维护规则

- 本目录所有链接使用仓库相对路径；外部法条链接仅作为定位入口，条文以官方现行文本为准。
- 代码、数据流、第三方组件、安装器行为或目标市场发生变化时，同步更新受影响文档并提升版本号。
- 任何草案在律师审阅前不得删除顶部声明，不得改成“最终版”，不得作为安装包 clickwrap 直接发布。
- 不得加入结论性表述（如“通过全部审查”“满足所有市场要求”“可在所有市场发行”）；不得把工程自检当作法律意见。
- 第三方 notices/SBOM/源码 offer 的维护责任在 lane A/B；本索引只做入口与状态记录。
- 发现链接失效、事实与代码不一致或新的数据流时，记入 [open-items.md](open-items.md)，不要静默删除。

## 6. 相关仓库证据

- 产品与限制：[README.md](../../README.md)
- 规格：[SDD 001](../../specs/001-screen-recorder.md)、[SDD 002](../../specs/002-recording-modes.md)
- 里程碑：[M18 报告](../milestones/m18-report.md)、[M17e 安装包报告](../milestones/m17e-package-report.md)、[M14 恢复/清理报告](../milestones/m14-report.md)、[L3 签核](../milestones/l3-signoff.md)
- FFmpeg 整改：[整改报告](../milestones/ffmpeg-lgpl-remediation.md)、[内部测试包报告](../milestones/ffmpeg-lgpl-package-report.md)
- 构建配置：[Cargo.toml](../../Cargo.toml)、[tauri.conf.json](../../src-tauri/tauri.conf.json)

---

**再次声明**：本目录是工程草案与待办索引，不是法律意见，也不构成对任何具体录制行为、发行市场或商业模式的结论。主体、联系方式、适用法与管辖均待填写，发布前必须由目标市场律师审阅。
