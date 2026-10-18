# M17 安装包重建（NSIS · 指纹 · 安装/自检）

> **后续更新（M17c，2026-10-19 00:10）**：acc-11 修复后已从当前源码重建并**覆盖本报告的 23:29 安装包**；最新产物、指纹与安装/自检见 `docs/milestones/m17c-package-report.md`（SHA-256 `3BF1DABE…0901`）。

- 日期：2026-10-18
- 范围：M16c 包（2026-10-18 21:26，SHA-256 `1118B8DD…290D`）之后源码完成
  **M17（P3 MP4 输出，REQ-16..20）**——设置项/收尾事务/路由与恢复/合并输入/命令快照/
  UI + 测试——证据 `docs/milestones/m17-report.md`。从当前最终源码重建 NSIS 并复验安装/自检。
- 流程：沿用 `docs/milestones/m11-c-report.md` / `m12-g-report.md` /
  `m14-package-report.md` / `m15-package-report.md` / `m15c-package-report.md` /
  `m16-package-report.md` / `m16c-package-report.md`（`npx --yes @tauri-apps/cli@2.12.3 build`，
  resources 映射 ffmpeg/ffprobe/licenses）。
- 约束遵守：只做打包与报告；未改源码/测试/`specs/`；未执行任何 `git` 写操作；
  **未跑长测/未跑测试**（L1/L2/UI 结果由编排者按 `m17-report.md` §3 既有证据使用）。
  构建前 `screen-rec`/`ffmpeg` 进程数均为 0，exe 未被锁定，未触发关闭/重试。

## 0. 结论

| 项 | 状态 | 关键证据 |
|---|---|---|
| 源码指纹 | **已记录（构建前/构建后/安装卸载后三次一致）** | §1：主清单 **29 项**（M16c 26 项 + `ui/index.html` + `crates/sr-merge/src/{inputs.rs,merge.rs}`）；构建前 23:27:30，23:30:11 与 23:35:07 两次重算全部相同（`POST_BUILD_DIFF_COUNT_VS_PRE_BUILD=0`、`FINAL_DIFF_COUNT_VS_PRE_BUILD=0`） |
| NSIS 重建 | **完成，已覆盖 M16c 21:26 旧包** | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe`；2026-10-18 23:29:50；86,612,740 B（82.60 MiB）；SHA-256 `9CB87F9F…C7FB` |
| M17 代码进包 | **已验证** | 本轮重编 `sr-merge`/`sr-pipeline`/`screen-rec`；9 个 UI 源文件的 brotli 资产解压 SHA-256 等于当前源码，且压缩字节在 release exe 与安装副本中原样存在；M17 收尾/设置/合并标记与 M16 继承标记均为 True |
| 静默安装 + 自检 | **exit 0** | `%TEMP%\sr_m17p_install`；安装 23:30:15–23:30:23（8.3 s）；ffmpeg/ffprobe 来源 `exe-adjacent`；版本 `n8.1.3-18-g46d899d7a0-20261015` |
| 卸载清理 | **完成** | `uninstall.exe /S` exit 0；安装目录已删除；无 `screen-rec`/`ffmpeg` 残留进程 |
| 产物哈希复核 | **一致** | 卸载后安装包 SHA-256 仍为 `9CB87F9F…C7FB` |
| ffmpeg source offer | **保持精确 commit** | 未改动；SHA-256 `FF1A3F3E…55808`；安装目录内同名文件哈希一致；commit `46d899d7a02bc63237472cc4897258d5ddf274ac` |
| 长测 | **未运行（按约束）** | 仅打包/安装/自检 |

**本报告描述的 23:29 安装包取代 M16c（21:26）安装包**；`m16c-package-report.md`
顶部已加指向本报告的说明。

## 1. 源码指纹（构建前记录，构建后/安装后复核一致）

记录时间：2026-10-18 23:27:30（本机）。最晚源码 mtime = **2026-10-18 23:26:01**
（`crates/sr-pipeline/src/finalize.rs`；`specs/002-recording-modes.md` 23:25:57、
`tests/sr-itests/tests/container.rs` 23:23:20、`recovery.rs` 23:22:42 紧随其后）；
源码在构建前 20 s 观察窗内哈希稳定，无 cargo/rustc 进程。最终构建开始 23:27:34，
新包 mtime = **2026-10-18 23:29:50**。

主清单 **29 项** = M16c 的 26 项 + `ui/index.html` + `crates/sr-merge/src/{inputs.rs,merge.rs}`
（`ui/index.html` 为 M16c 清单外但在本轮实际改动内；`sr-merge` 为 M17 合并输入改动新增纳入；
`merge.rs` 一并列入基线，内容未变）。

| 文件 | SHA-256 | 大小 | mtime |
|---|---|---|---|
| `Cargo.lock` | `5268A66731DC03F07653A0A3072B0E1704055742BCAB35724B3617BC66966AAF` | 122,645 | 2026-10-17 19:32:45 |
| `src-tauri/tauri.conf.json` | `95080F11734FA35F3BF28B3B51B01A4D8EA28A1AF742B4FD306FDDE040F819FC` | 899 | 2026-10-18 14:40:47 |
| `src-tauri/src/region.rs` | `A9A04F7C82E5E54894BA3ABAB0092BAA619D954DE741A6DE296C5D4310B96D24` | 13,610 | 2026-10-17 21:01:09 |
| `src-tauri/src/lib.rs` | `46882CE1257AB7775099217BE14B7CF7750352396E0124CD8230F2C6C7108CD0` | 12,040 | 2026-10-18 19:41:18 |
| `src-tauri/src/commands.rs` | `CB67A3E1857177D5E9C2FB9D2FF1D152601168424A326F53FC286B6061D6A6AC` | 84,368 | 2026-10-18 22:38:12 |
| `src-tauri/src/settings.rs` | `1BEB5F806DB3427C34E62033A9D8380329B30223F035243D695DEE2DC537A30A` | 30,111 | 2026-10-18 23:21:21 |
| `crates/sr-pipeline/src/route.rs` | `F2ECCCAFD3189C07968DB0C9B576C9469A75E905FF5E9FB91069E5CB75753A17` | 113,600 | 2026-10-18 23:21:53 |
| `crates/sr-pipeline/src/naming.rs` | `86DFA061C4E30EA5E2C936417F071785C3D7BC8EE5A340CD17C1A9685824E2FB` | 9,291 | 2026-10-18 22:33:20 |
| `crates/sr-pipeline/src/recovery.rs` | `BFC0DB515F06616A583B0C4D24BC080F62F81D47D5791EDEB93344C9F4A9D800` | 47,317 | 2026-10-18 23:22:42 |
| `crates/sr-pipeline/src/sidecar.rs` | `44FBDE79404689F6319FBCF1CAEEE95EC56AEC9DC393CF06F630ED3C5F00832B` | 17,343 | 2026-10-18 15:36:42 |
| `crates/sr-pipeline/src/finalize.rs` | `877B9880DEA5B7AF9BF3DF11EC0C34D2DFBA3D0BFABD23B917FAD67E78988151` | 34,672 | 2026-10-18 23:26:01 |
| `crates/sr-audio/src/resample.rs` | `66711E62512F189968A39ABBD40765AB6D7A9CA1C81EDD16E17CA49E5B235041` | 19,510 | 2026-10-18 15:31:02 |
| `crates/sr-audio/src/wasapi.rs` | `A45C972895CE5DA75E4B377B44EBF96C7A7749A1396BA4706A2C8C5A8F378842` | 58,508 | 2026-10-18 20:51:45 |
| `crates/sr-audio/src/slots.rs` | `19FD8BBE37599801C6895A62C5E13ED2B6E0F6E4D9349B4EDCC7D0010925DA12` | 9,218 | 2026-10-18 19:58:01 |
| `crates/sr-capture/src/wgc.rs` | `90B244810A5BED028E5233CC12B2E3C7A6504C6D543A5BD6ECF1BCC83BA12928` | 10,824 | 2026-10-17 22:28:49 |
| `crates/sr-enum/src/monitors.rs` | `DF2BB30A00148C7678436B90EF2D342717F2EF4C17420590C85D2AC57A28CD13` | 12,655 | 2026-10-18 20:01:11 |
| `crates/sr-enum/src/render_endpoints.rs` | `CC63C4E7DCC8D5DA1583DFC4CDE17EA5B80710E6D5EDC5E300F9B92B996E7244` | 11,091 | 2026-10-18 20:01:11 |
| `crates/sr-core/src/events.rs` | `3E0086BFE8C793DDB8E87222C5FA31E267AD62B9F6AA1EC5A0D4FAF5B95B3CF6` | 12,383 | 2026-10-18 20:53:04 |
| `ui/region.js` | `AAA0CD9AD104E521BDBD2B5DDACB714DB5AB25ED481BE951B739A2D40001D9A4` | 5,769 | 2026-10-17 21:01:13 |
| `ui/app.js` | `DB7C0E0566CBB6FFDEE79D42219730FC1AF69FC296EF3D888772F92C7178FFFC` | 65,741 | 2026-10-18 22:38:33 |
| `ui/api.js` | `CA70C3ABB9F3B35FA058085B0C1599806DBB2051A6D16E7EF48F489757C2EB5F` | 27,381 | 2026-10-18 22:38:29 |
| `ui/endpoint-match.js` | `EF81FB3BF3F3726B2BB656975DD1652350CCD15C1A63CA8E3A22F05F7A54BC74` | 8,202 | 2026-10-18 19:35:07 |
| `ui/route-state.js` | `D93A441AB7EB8A826E89C7DA9CBAD4CC70CE5CFCC2514861CD5FD611D6349694` | 4,202 | 2026-10-18 19:46:03 |
| `ui/index.html` | `8EA7E990F860BBC02DE1AC26DBAF0F2BC237BA55845B0EDFDC77593A65665E6A` | 8,107 | 2026-10-18 22:38:36 |
| `crates/sr-audio/tests/stall_pad_policy.rs` | `60553CAE593D3B80E4074A32647EFF70B6CC90D9C667F5DD7449DDE9F3CE0733` | 6,000 | 2026-10-17 23:42:13 |
| `tools/ui/settings-sample-rate.mjs` | `1C0F7389C5556D3AD8817BDA4895C9BC072558221CDE48759787CB5E738A2F11` | 19,702 | 2026-10-18 22:35:57 |
| `tools/ui/audio-sources-builder.mjs` | `D71D37BA06B59AEE21E15169B98B03EFAF93750AAE757E590EEEFDEBB7217747` | 30,456 | 2026-10-18 21:14:47 |
| `crates/sr-merge/src/inputs.rs` | `664BD95153C5D0B02100BFD62D03FEA943A1C0AAE07D3BB16B9273D6EF115FF6` | 1,984 | 2026-10-18 22:38:18 |
| `crates/sr-merge/src/merge.rs` | `367E81556265DE39E75ED72327D9D4422B469CBA2AF2FA3D700D4AA960A6F9BE` | 13,917 | 2026-10-17 21:00:19 |

说明：

- 构建后（23:30:11）与安装/卸载后（23:35:07）对上述 29 项重算 SHA-256，**逐项与
  构建前相同**（`m17p-fingerprint.log`：`POST_BUILD_DIFF_COUNT_VS_PRE_BUILD=0`、
  `FINAL_DIFF_COUNT_VS_PRE_BUILD=0`、`FINAL_COMPARED_ITEMS=29`）。
- 相对 M16c 包主清单（26 项）**内容变化 10 项**：

  | 文件 | M16c | M17 | 变化 |
  |---|---|---|---|
  | `src-tauri/src/commands.rs` | 81,616；`230059F6…A487` | 84,368；`CB67A3E1…A6AC` | +2,752 |
  | `src-tauri/src/settings.rs` | 22,131；`92018FD5…41C0` | 30,111；`1BEB5F80…A30A` | +7,980 |
  | `crates/sr-pipeline/src/route.rs` | 107,578；`34E7272A…4EB1` | 113,600；`F2ECCCAF…A17` | +6,022 |
  | `crates/sr-pipeline/src/naming.rs` | 7,957；`5BCA89EE…EB4B` | 9,291；`86DFA061…E2FB` | +1,334 |
  | `crates/sr-pipeline/src/recovery.rs` | 41,634；`A9BBC091…42D8` | 47,317；`BFC0DB51…D800` | +5,683 |
  | `crates/sr-pipeline/src/finalize.rs` | 10,981；`FF180983…8BA` | 34,672；`877B9880…8151` | +23,691 |
  | `ui/app.js` | 65,099；`07F0FD82…8D6E15` | 65,741；`DB7C0E05…FFFC` | +642 |
  | `ui/api.js` | 26,828；`97CCDA96…A47C1D` | 27,381；`CA70C3AB…EB5F` | +553 |
  | `ui/index.html`（M16c 清单外） | 7,863；`43C8C424…EBFA` | 8,107；`8EA7E990…5E6A` | +244 |
  | `crates/sr-merge/src/inputs.rs`（新纳入） | M16c 未列；`git diff` 11+/5− | 1,984；`664BD951…5FF6` | 当前值 |

  - 其余 18 项内容不变（含 `Cargo.lock`、`tauri.conf.json`、`region.rs`、`lib.rs`、
    `sidecar.rs`、`resample.rs`、`wasapi.rs`、`slots.rs`、`wgc.rs`、`monitors.rs`、
    `render_endpoints.rs`、`events.rs`、`region.js`、`endpoint-match.js`、`route-state.js`、
    `stall_pad_policy.rs`、`settings-sample-rate.mjs`、`audio-sources-builder.mjs`）。
  - `crates/sr-merge/src/merge.rs` 未变（`git diff --numstat` 无该文件；当前 `367E8155…F9BE`）。
  - 与 `m17-report.md` §2 的改动面一致：设置/收尾/路由/恢复/合并/命令快照/UI 与测试。
- 任务清单外的同期改动（未计入 29 项主指纹，也不随包；与 M16c 处理一致）：
  `tests/sr-itests/tests/sync_dual.rs`、新增 `tests/sr-itests/tests/container.rs`、
  `specs/002-recording-modes.md`。补充哈希：

  | 文件 | SHA-256 | 大小 | mtime |
  |---|---|---|---|
  | `tests/sr-itests/tests/sync_dual.rs` | `EAD7829167B67F07BDB974410FFF55BA319CBAB806C2809EB8F395E835846780` | 30,469 | 2026-10-18 23:21:21 |
  | `tests/sr-itests/tests/container.rs` | `2BC0825204033E8F95677E53B3D95AEDD6D45C9E79C4936DAEFC485B9B40A300` | 39,515 | 2026-10-18 23:23:20 |
  | `specs/002-recording-modes.md` | `7FE5C369DC1C3ACABFE0BF7B80847EDAB7D94AF3540523FE7FDC0E5888D0495B` | 29,453 | 2026-10-18 23:25:57 |

  这些为 M17 既有工作树内容；本任务只打包，未改这些文件；bundle resources 仍仅
  ffmpeg/ffprobe/licenses，测试与 spec 不随安装包。

打包资源（本轮未改）：

| 文件 | SHA-256 | 说明 |
|---|---|---|
| `src-tauri/resources/licenses/ffmpeg-source-offer.txt` | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` | 精确 commit + 四种获取方式；3,411 B |
| `src-tauri/resources/licenses/FFmpeg-LICENSE.txt` | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | 与构建包 LICENSE.txt 一致 |

## 2. 构建

工具链：Node v24.18.0 / npm 11.16.0；rustc 1.99.0 / cargo 1.99.0；
`tauri-cli 2.12.3`（npx）。

```
npx --yes @tauri-apps/cli@2.12.3 build
→ Compiling sr-merge v0.1.0
→ Compiling sr-pipeline v0.1.0
→ Compiling screen-rec v0.1.0
→ Finished `release` profile [optimized] target(s) in 22.84s
→ Built application at: ...\target\release\screen-rec.exe
→ Info Patching ...\screen-rec.exe with bundle type information: nsis
→ Running makensis to produce ...\target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe
→ Finished 1 bundle at: ...\Screen_rec_0.1.0_x64-setup.exe (82.60 MiB)
→ TAURI_BUILD_EXIT=0
```

- 构建开始：2026-10-18 **23:27:34**；结束：**23:29:50**（135.7 s；
  `m17p-build-nsis.log`）。Rust 段 22.84 s；仅重编 `sr-merge`/`sr-pipeline`/`screen-rec`
  （依赖 crate 未变，未重编）。
- 构建前 `screen-rec.exe` 进程数 0、`ffmpeg` 进程数 0，exe 未被锁定，未触发重试。
- 日志中 `node.exe : Info ...` 与 `NativeCommandError` 为 PowerShell 将 npx 的
  informational stderr 包装为错误记录；`TAURI_BUILD_EXIT=0` 且产物完整（日志首尾均有说明）。

产物（覆盖旧包）：

| 项 | 旧包（M16c，被覆盖） | 新包（M17，本轮） |
|---|---|---|
| 路径 | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe` | 同左 |
| 构建时间（mtime） | 2026-10-18 21:26:39 | **2026-10-18 23:29:50** |
| 大小 | 86,634,086 B（82.62 MiB） | **86,612,740 B（82.60 MiB，−21,346 B）** |
| SHA-256 | `1118B8DD5FFC82ECFA2BFCE7560EB83AA4342391C039F6CD9A3864BDADAA290D` | **`9CB87F9FCF313555F20A3122729AF7BE717B8BCE5E43B4717AC43A9A4E90C7FB`** |

- release 主程序：`target\release\screen-rec.exe`，15,042,048 B（较 M16c 的
  15,006,720 B 增加 35,328 B），mtime 23:29:50，
  SHA-256 `B88EF8A2AB6FA1FAB2A51112768F0181F15BC4A0647D78EE4E7B8E949B93511B`。
- 安装包比 M16c 小 21,346 B，而 release 主程序大 35,328 B：NSIS 压缩率/元数据差异
  抵消并超过主程序增量；安装清单完整（§3），不是缺文件。

**UI 进包证据**：本轮 Tauri 重新生成 `tauri-codegen-assets`
（目录 `target\release\build\screen-rec-ee450db19d850100\out\tauri-codegen-assets`，
更新时间 23:27:39）；解压后 SHA-256 与当前源码逐项相等，压缩字节在 release exe 与
安装副本中原样存在（`m17p-ui-integrity.log`）：

| 源码 | 资产文件（brotli） | 压缩 B | 解压 B | 解压 SHA-256 | release / installed |
|---|---|---:|---:|---|---|
| `ui/app.js` | `5a36d1bb…2f14.js` | 17,491 | 65,741 | `DB7C0E05…FFFC` | true / true |
| `ui/api.js` | `8db0f721…b66d.js` | 8,836 | 27,381 | `CA70C3AB…EB5F` | true / true |
| `ui/index.html` | `fc013e19…bf1e.html` | 2,390 | 8,107 | `8EA7E990…5E6A` | true / true |
| `ui/endpoint-match.js` | `e8565f57…52a1.js` | 2,750 | 8,202 | `EF81FB3B…BC74` | true / true |
| `ui/route-state.js` | `23061bfb…fd8db.js` | 1,479 | 4,202 | `D93A441A…9694` | true / true |
| `ui/region.js` | `a15c6368…8dd3b.js` | 2,246 | 5,769 | `AAA0CD9A…D9A4` | true / true |
| `ui/region.html` | `2c24e205…3f21b.html` | 532 | 1,053 | `95543207…777B` | true / true |
| `ui/region-geometry.js` | `585d3e0f…121d.js` | 1,046 | 2,568 | `CBF3CB71…2294` | true / true |
| `ui/styles.css` | `59ad2112…3d12a.css` | 4,028 | 17,827 | `C54A57FF…A506C` | true / true |

进一步核验：

- 构建目录共 23 个资产；9 个当前源码资产全部解压哈希相等并命中 release/安装 exe，
  其余 14 个为历史/孤立资产（含 M16c 的 `app.js` `854b8101…51ce.js`、
  `api.js` `36760c3e…f6841.js`、`index.html` `9bc926e8…176e.html`），
  在 release exe 与安装副本中均 **未命中**（`inExe=false`），证明包内 UI 不是旧缓存。
- M16c 源码哈希 `07F0FD82…8D6E15`（app）、`97CCDA96…A47C1D`（api）、
  `43C8C424…EBFA`（index）对应的资产均未进包；当前源码哈希见 §1。
- 二进制标记（release 与安装副本，UTF-8 字节搜索，均 True；`m17p-markers.log`）：
  - M17：`MP4 校验失败`、`remux.mp4.tmp`、`concat.mkv.tmp`、`收尾峰值`、
    `会话级占用失败`、`output_container`、`ftyp`、`finalize_session`；
  - 继承 M16：`render_endpoint 音源仅适用于显示器目标`、`显示器目标没有任何音源`、
    `纯录音不接受 render_endpoint 音源`、`音频采集端点失效`；
  - 继承既有：`E_INVALID_AUDIO_PLAN`、`stop_notice`、`keep_segments`、
    `audio_sample_rate`、`SR_DIAG_AV`。
- 说明：release 链接产物未检出 `+faststart`/`movflags` 字面量（对应
  `build_remux_args` 在 `libsr_pipeline…rlib` 中存在），本轮**未将其作为进包判据**；
  M17 收尾事务以 `MP4 校验失败`/`remux.mp4.tmp`/`concat.mkv.tmp`/`收尾峰值`/
  `会话级占用失败` 等生产字符串 + UI 资产 + 重编日志判定。该观察列入 §6 #1。

完整日志：`docs/milestones/m17p-build-nsis.log`（UTF-8；`TAURI_BUILD_EXIT=0`）。

## 3. 静默安装 + 安装目录自检

```
安装：Screen_rec_0.1.0_x64-setup.exe /S /D=C:\Users\elvis\AppData\Local\Temp\sr_m17p_install
→ installer_exit=0；install_dir_exists=True（安装 2026-10-18 23:30:15–23:30:23，8.3 s）
```

安装目录清单（实测）：

| 文件 | 大小 | mtime | SHA-256 |
|---|---:|---|---|
| `screen-rec.exe` | 15,042,048 B | 2026-10-18 23:28:00 | `26402243AD073779AE5501089739C2581AD12ADD518AB61940C2D32551A1769D` |
| `uninstall.exe` | 79,390 B | 2026-10-18 23:30:22 | `B8712ADAAC20514E4751D05CF4152537EA2E3BAA2B079256F386EBE0A4C6B387` |
| `ffmpeg\ffmpeg.exe` | 134,092,288 B | 2026-10-15 12:43:48 | `3B57117B384DF98F2B8D01876DF0A596A0B6EBF45F756B30F15D6F65AB8251C5` |
| `ffmpeg\ffprobe.exe` | 133,887,488 B | 2026-10-15 12:43:48 | `5B9724EE7A115D5A8458D41BD2E0CE6206847E2B76105E822061A4421322CDE9` |
| `licenses\FFmpeg-LICENSE.txt` | 7,651 B | 2026-10-15 12:43:48 | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` |
| `licenses\ffmpeg-source-offer.txt` | 3,411 B | 2026-10-17 20:19:36 | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` |

- 安装副本 `screen-rec.exe` 与 release exe 大小相同（15,042,048 B），哈希不同
  （`264022…1769D` vs `B88EF8…3511B`）：Tauri 向 release 主程序写入 bundle 类型信息后
  再打包（M12-G/M14/M15/M15c/M16/M16c 同现象）。
- 安装副本二进制标记：§2 的 M17/M16/既有标记全部 True；9 个 UI 资产压缩字节
  在安装副本中同样原样存在（`IN_INSTALLED_EXE=true`）。
- ffmpeg/ffprobe/licenses 哈希与 M16c 包逐一相同；source offer 未变。

`--self-check`（安装目录运行；完整输出 `docs/milestones/m17p-self-check.log`，
2026-10-18 23:30:40–23:30:41）：

```
[self-check] 搜索根（按优先级）
  env  SCREENREC_FFMPEG_DIR: (未设置)
  tauri resource_dir: (未设置)
  exe 同目录: C:\Users\elvis\AppData\Local\Temp\sr_m17p_install
  开发回退: ...\Screen_rec\crates\sr-core\../../tools/ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin
[self-check] ffmpeg: ...\sr_m17p_install\ffmpeg\ffmpeg.exe（来源：exe-adjacent）
[self-check]   -version: ffmpeg version n8.1.3-18-g46d899d7a0-20261015 ...
[self-check] ffprobe: ...\sr_m17p_install\ffmpeg\ffprobe.exe（来源：exe-adjacent）
[self-check]   -version: ffprobe version n8.1.3-18-g46d899d7a0-20261015 ...
SELF_CHECK_EXIT=0
```

- `screen-rec.exe` 为 GUI 子系统：使用 `Start-Process -Wait
  -RedirectStandardOutput/-RedirectStandardError` 取得完整输出与 `SELF_CHECK_EXIT=0`。
  重定向输出文件为 UTF-8；控制台按本地代码页显示中文可能乱码，不影响日志文件内容。

卸载与产物复核：

```
uninstall.exe /S → uninstall_exit=0；dir_exists_after_uninstall=False
残留进程数：screen-rec=0，ffmpeg=0
安装包 SHA-256（卸载后）→ 9CB87F9FCF313555F20A3122729AF7BE717B8BCE5E43B4717AC43A9A4E90C7FB（与构建时一致）
```

- 为让 marker 日志同时覆盖安装副本，23:35 再次静默安装 → 标记扫描 → 卸载；
  `reinstall_exit=0`、`reuninstall_exit=0`、目录删除、产物哈希仍为
  `9CB87F9F…C7FB`（`m17p-install.log`）。该重复安装不影响首次安装/自检证据。

## 4. ffmpeg source offer（保持 M12-C 精确说明）

- 文件未改动：SHA-256 `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808`，3,411 B。
- 精确版本：`FFmpeg n8.1.3-18-g46d899d7a0` = tag `n8.1.3` + 18 commits，commit
  `46d899d7a02bc63237472cc4897258d5ddf274ac`（文件第 7/29/34/49 行均保留）。
- 四种获取方式保留：git checkout / GitHub commit 归档 / 上游 tag 对照（注明少 18 commits）/
  BtbN 构建脚本与日期。
- 随包验证：安装目录 `licenses/ffmpeg-source-offer.txt` 哈希 = 源文件哈希（§3）；
  `--self-check` 打印的 ffmpeg/ffprobe 版本与该说明一致。

## 5. 变更文件清单

| 文件 | 变更 |
|---|---|
| `docs/milestones/m17-package-report.md` | 本报告（新增） |
| `docs/milestones/m17p-build-nsis.log` | NSIS 构建完整原始输出 + 包装时间/哈希（`TAURI_BUILD_EXIT=0`，新增） |
| `docs/milestones/m17p-fingerprint.log` | 29 项源码指纹三次记录、产物/资源哈希（新增） |
| `docs/milestones/m17p-ui-integrity.log` | 23 个 codegen 资产解压/源码比对、release 与安装副本命中、旧缓存排除（新增） |
| `docs/milestones/m17p-markers.log` | release/安装副本 M17/M16/既有二进制标记（新增） |
| `docs/milestones/m17p-self-check.log` | 安装副本 self-check 原始输出（新增） |
| `docs/milestones/m17p-install.log` | 安装清单/哈希、self-check、卸载、重复安装与产物复核（新增） |
| `docs/milestones/m16c-package-report.md` | 顶部加一段指向本报告（其 21:26 旧包已被覆盖） |

未改任何源码/测试/`specs/`；§1 三次指纹一致可证。

> 说明：`docs/milestones/*.log` 被 `.gitignore` 的 `*.log` 忽略（与 M11–M16c 报告相同），
> 日志已落盘但默认不进入 git；如需提交日志，请编排者显式 `git add -f`。

## 6. 未决 / 风险（回交编排者）

1. **release 产物未检出 `+faststart`/`movflags` 字面量**：`build_remux_args` 在
   `libsr_pipeline…rlib` 中存在，但 release exe/安装副本中未见该字符串；本任务未做
   release 级 MP4 运行验证，不能据此判定 release 的 MP4 路径失效（M17 L2 证据在
   test profile）。若验收要求安装包内 MP4 收尾可执行，建议编排者安排
   release 模式 `it_route_container_mp4` 或 §L3 播放验证后再宣称 release P3 通过。
2. **未签名**：NSIS 安装包仍未签名（同 M11-C/M12-G/M14/M15/M15c/M16/M16c），
   生产分发需证书与签名步骤。
3. **未在干净机安装**：本机已有 WebView2；安装器对 WebView2 前置的引导未在无运行时
   机器审计（同前序报告）。
4. **包内 UI 未做运行时点击复验**：本任务只做“构建 + 安装 + `--self-check`”+ 资产/标记核验；
   M17 UI runner 证据（容器下拉/说明/保存重载 + mock 三路径 `.mp4`）在 `m17-report.md` §3
   的 `m17-ui.log`。若验收要求安装包内真实点击回归，需 GUI 运行（不在本任务范围）。
5. **M17 报告中 `it_route_display_endpoint_plus_mic_tracks` 仍 BLOCKED**（本机无采集设备），
   与 M16 相同，不计通过；属源码/测试验收问题，不是打包问题。
6. **长测未运行**：按约束未跑电池/sync/soak；本报告不据此宣称长稳。
7. **测试/spec 改动未计入 29 项主指纹**：`tests/sr-itests/tests/{sync_dual.rs,container.rs}`、
   `specs/002-recording-modes.md` 为 M17 既有工作树内容（补充哈希见 §1）；bundle resources
   不变，测试/spec 不随包。
8. **安装后 exe 哈希与 release exe 不同**：Tauri 写入 bundle 类型信息后再打包；
   安装副本与构建目录副本因此不必逐字节相同（同 M15c/M16/M16c）。

## 7. 证据索引（本机实际）

| 证据 | 内容 |
|---|---|
| `docs/milestones/m17p-build-nsis.log` | `npx @tauri-apps/cli@2.12.3 build` 原始输出；23:27:34–23:29:50，exit 0，82.60 MiB |
| `docs/milestones/m17p-fingerprint.log` | 29 项 SHA-256 构建前 23:27:30、构建后 23:30:11、安装卸载后 23:35:07；`POST_BUILD_DIFF_COUNT_VS_PRE_BUILD=0`、`FINAL_DIFF_COUNT_VS_PRE_BUILD=0`；产物/资源哈希 |
| `docs/milestones/m17p-ui-integrity.log` | 23 个 codegen 资产解压 SHA-256、9 个当前源码命中、旧 M16c 资产 `inExe=false`、release/安装副本压缩字节命中 |
| `docs/milestones/m17p-markers.log` | M17/M16/既有二进制标记的 release/安装副本 UTF-8 字节搜索结果 |
| `docs/milestones/m17p-self-check.log` | 安装副本 self-check 原始 stdout/stderr（`SELF_CHECK_EXIT=0`，23:30:40） |
| `docs/milestones/m17p-install.log` | 安装清单/哈希、安装/卸载时间与退出码、重复安装 marker 核验、产物哈希复核 |
| 源码指纹 | §1（`Get-FileHash <file> -Algorithm SHA256`） |
| 安装/卸载 | §3（`installer_exit=0`、`uninstall_exit=0`、目录已删除、产物哈希复核一致） |
| M17 行为证据 | `docs/milestones/m17-report.md` §3（L1/L2/UI 日志） |
| M16c/M16 行为证据 | `docs/milestones/m16c-package-report.md`、`m16-report.md` |
