# M17e 安装包重建（NSIS · 指纹 · 安装/自检）

- 日期：2026-10-19
- 范围：M17c 包（2026-10-19 00:10:36，SHA-256 `3BF1DABE…0901`）之后源码完成
  **acc-12 两项 P2 修复（M17d）**——`tools/ui/settings-sample-rate.mjs`（真实录制中
  改容器的正/反双向快照检查）、`crates/sr-pipeline/src/finalize.rs`
  （`finalize_space_policy`：MP4 硬预检 / MKV 仅日志 + 两个单测）；证据
  `docs/milestones/m17-report.md` §8。从当前最终源码重建 NSIS 并复验安装/自检。
- 流程：沿用 `docs/milestones/m17c-package-report.md` / `m17-package-report.md` /
  `m16c-package-report.md` 等（`npx --yes @tauri-apps/cli@2.12.3 build`，
  resources 映射 ffmpeg/ffprobe/licenses）。
- 约束遵守：只做打包与报告；未改源码/测试/`specs/`；未执行任何 `git` 写操作；
  **未跑长测/未跑测试**（L1/L2/UI 结果由编排者按 `m17-report.md` §8 既有证据使用）。
  构建前 `screen-rec`/`ffmpeg`/`ffprobe` 进程数均为 0，exe 未被锁定，未触发关闭/重试。

## 0. 结论

| 项 | 状态 | 关键证据 |
|---|---|---|
| 源码指纹 | **已记录（构建前/构建后/安装卸载后三次一致）** | §1：主清单 **29 项**；构建前 00:36:18，构建后 00:38:59 与安装卸载后 00:40:23 两次重算全部相同（`POST_BUILD_DIFF_COUNT_VS_PRE_BUILD=0`、`FINAL_DIFF_COUNT_VS_PRE_BUILD=0`、`FINAL_COMPARED_ITEMS=29`） |
| NSIS 重建 | **完成，已覆盖 M17c 00:10 旧包** | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe`；2026-10-19 00:38:31；86,631,008 B（82.62 MiB）；SHA-256 `B47803C3…4429` |
| M17e 代码进包 | **已验证** | 本轮重编 `sr-pipeline`/`screen-rec`；acc-12 新增生产字符串（MKV「按既有行为继续（不硬拒绝）」/ 收尾空间预算 / 新增空间预算 / 总峰值）在 release 与安装副本中均为 True；9 个 UI 源文件的 brotli 资产解压 SHA-256 等于当前源码，且压缩字节在 release exe 与安装副本中原样存在（9/9） |
| 静默安装 + 自检 | **exit 0** | `%TEMP%\sr_m17ep_install`；安装 00:39:27–00:39:34（7.3 s）；ffmpeg/ffprobe 来源 `exe-adjacent`；版本 `n8.1.3-18-g46d899d7a0-20261015`；`SELF_CHECK_EXIT=0` |
| 卸载清理 | **完成** | `uninstall.exe /S` exit 0（1.2 s）；安装目录已删除；无 `screen-rec`/`ffmpeg`/`ffprobe` 残留进程 |
| 产物哈希复核 | **一致** | 卸载后安装包 SHA-256 仍为 `B47803C3…4429`，大小/时间不变 |
| 指纹相对 M17c | **内容变化 2 个文件（主清单 2 项）** | §1：`finalize.rs`、`settings-sample-rate.mjs`；其余 27 项主清单哈希与 M17c 相同；补充清单中的测试/spec 未变，`settings-sample-rate.mjs` 为同一 runner（重复计入补充清单） |
| ffmpeg source offer | **保持精确 commit** | 未改动；SHA-256 `FF1A3F3E…55808`；安装目录内同名文件哈希一致；commit `46d899d7a02bc63237472cc4897258d5ddf274ac` |
| 长测 | **未运行（按约束）** | 仅打包/安装/自检 |

**本报告描述的 2026-10-19 00:38 安装包取代 M17c（2026-10-19 00:10）安装包**；
`m17c-package-report.md` 顶部已加指向本报告的说明。

## 1. 源码指纹（构建前记录，构建后/安装后复核一致）

记录时间：2026-10-19 00:36:18（本机）。最晚源码 mtime = **2026-10-19 00:30:37**
（`tools/ui/settings-sample-rate.mjs`；核心源码最新为 `crates/sr-pipeline/src/finalize.rs`
00:30:20）；构建开始 00:36:23，距最晚源码改动约 5 分 46 秒，观察窗内无源码写入。
最终构建结束 00:38:31，新包 mtime = **2026-10-19 00:38:31**。

主清单 **29 项**与 M17c/M17 报告一致（含 `ui/index.html` 与
`crates/sr-merge/src/{inputs.rs,merge.rs}`）。

| 文件 | SHA-256 | 大小 | mtime |
|---|---|---|---|
| `Cargo.lock` | `5268A66731DC03F07653A0A3072B0E1704055742BCAB35724B3617BC66966AAF` | 122,645 | 2026-10-17 19:32:45 |
| `src-tauri/tauri.conf.json` | `95080F11734FA35F3BF28B3B51B01A4D8EA28A1AF742B4FD306FDDE040F819FC` | 899 | 2026-10-18 14:40:47 |
| `src-tauri/src/region.rs` | `A9A04F7C82E5E54894BA3ABAB0092BAA619D954DE741A6DE296C5D4310B96D24` | 13,610 | 2026-10-17 21:01:09 |
| `src-tauri/src/lib.rs` | `46882CE1257AB7775099217BE14B7CF7750352396E0124CD8230F2C6C7108CD0` | 12,040 | 2026-10-18 19:41:18 |
| `src-tauri/src/commands.rs` | `CB67A3E1857177D5E9C2FB9D2FF1D152601168424A326F53FC286B6061D6A6AC` | 84,368 | 2026-10-18 22:38:12 |
| `src-tauri/src/settings.rs` | `1BEB5F806DB3427C34E62033A9D8380329B30223F035243D695DEE2DC537A30A` | 30,111 | 2026-10-18 23:21:21 |
| `crates/sr-pipeline/src/route.rs` | `517E70830E91AF456F1F801AF37EF79B07D5206EA6E04577DECA6BD04CF8A93F` | 116,223 | 2026-10-18 23:56:38 |
| `crates/sr-pipeline/src/naming.rs` | `020CAE3AFF5328FC0FB19E8696F1E785439D679782AEE0866BF6345FC8D46D2A` | 11,664 | 2026-10-18 23:45:19 |
| `crates/sr-pipeline/src/recovery.rs` | `5D1C45F4419DDB9BC0C63A83090DBAF64591AD8FBB90B85C417038A18F359D57` | 56,097 | 2026-10-18 23:56:49 |
| `crates/sr-pipeline/src/sidecar.rs` | `44FBDE79404689F6319FBCF1CAEEE95EC56AEC9DC393CF06F630ED3C5F00832B` | 17,343 | 2026-10-18 15:36:42 |
| `crates/sr-pipeline/src/finalize.rs` | `801E356701ECCFE32F92B85C3F14DB64004BC43BF75F58EE36508CE3A2869298` | 54,001 | 2026-10-19 00:30:20 |
| `crates/sr-audio/src/resample.rs` | `66711E62512F189968A39ABBD40765AB6D7A9CA1C81EDD16E17CA49E5B235041` | 19,510 | 2026-10-18 15:31:02 |
| `crates/sr-audio/src/wasapi.rs` | `A45C972895CE5DA75E4B377B44EBF96C7A7749A1396BA4706A2C8C5A8F378842` | 58,508 | 2026-10-18 20:51:45 |
| `crates/sr-audio/src/slots.rs` | `19FD8BBE37599801C6895A62C5E13ED2B6E0F6E4D9349B4EDCC7D0010925DA12` | 9,218 | 2026-10-18 19:58:01 |
| `crates/sr-capture/src/wgc.rs` | `90B244810A5BED028E5233CC12B2E3C7A6504C6D543A5BD6ECF1BCC83BA12928` | 10,824 | 2026-10-17 22:28:49 |
| `crates/sr-enum/src/monitors.rs` | `DF2BB30A00148C7678436B90EF2D342717F2EF4C17420590C85D2AC57A28CD13` | 12,655 | 2026-10-18 20:01:11 |
| `crates/sr-enum/src/render_endpoints.rs` | `CC63C4E7DCC8D5DA1583DFC4CDE17EA5B80710E6D5EDC5E300F9B92B996E7244` | 11,091 | 2026-10-18 20:01:11 |
| `crates/sr-core/src/events.rs` | `3E0086BFE8C793DDB8E87222C5FA31E267AD62B9F6AA1EC5A0D4FAF5B95B3CF6` | 12,383 | 2026-10-18 20:53:04 |
| `ui/region.js` | `AAA0CD9AD104E521BDBD2B5DDACB714DB5AB25ED481BE951B739A2D40001D9A4` | 5,769 | 2026-10-17 21:01:13 |
| `ui/app.js` | `DB7C0E0566CBB6FFDEE79D42219730FC1AF69FC296EF3D888772F92C7178FFFC` | 65,741 | 2026-10-18 22:38:33 |
| `ui/api.js` | `CA70C3ABB9F3B35FA058085B0C1599806DBB2051A6D16E7EF48F489757C2EB5F` | 27,381 | 2026-10-19 00:31:15（内容未变，见下） |
| `ui/endpoint-match.js` | `EF81FB3BF3F3726B2BB656975DD1652350CCD15C1A63CA8E3A22F05F7A54BC74` | 8,202 | 2026-10-18 19:35:07 |
| `ui/route-state.js` | `D93A441AB7EB8A826E89C7DA9CBAD4CC70CE5CFCC2514861CD5FD611D6349694` | 4,202 | 2026-10-18 19:46:03 |
| `ui/index.html` | `8EA7E990F860BBC02DE1AC26DBAF0F2BC237BA55845B0EDFDC77593A65665E6A` | 8,107 | 2026-10-18 22:38:36 |
| `crates/sr-audio/tests/stall_pad_policy.rs` | `60553CAE593D3B80E4074A32647EFF70B6CC90D9C667F5DD7449DDE9F3CE0733` | 6,000 | 2026-10-17 23:42:13 |
| `tools/ui/settings-sample-rate.mjs` | `090D01D6F339CF47B38F355F2FD6A1C5331AA85A33D721288C0C840A6623234E` | 23,874 | 2026-10-19 00:30:37 |
| `tools/ui/audio-sources-builder.mjs` | `D71D37BA06B59AEE21E15169B98B03EFAF93750AAE757E590EEEFDEBB7217747` | 30,456 | 2026-10-18 21:14:47 |
| `crates/sr-merge/src/inputs.rs` | `664BD95153C5D0B02100BFD62D03FEA943A1C0AAE07D3BB16B9273D6EF115FF6` | 1,984 | 2026-10-18 22:38:18 |
| `crates/sr-merge/src/merge.rs` | `367E81556265DE39E75ED72327D9D4422B469CBA2AF2FA3D700D4AA960A6F9BE` | 13,917 | 2026-10-17 21:00:19 |

说明：

- 构建后（00:38:59）与安装/卸载后（00:40:23）对上述 29 项重算 SHA-256，**逐项与
  构建前相同**（`m17ep-fingerprint.log`：`POST_BUILD_DIFF_COUNT_VS_PRE_BUILD=0`、
  `FINAL_DIFF_COUNT_VS_PRE_BUILD=0`、`FINAL_COMPARED_ITEMS=29`）。
- 相对 M17c 包主清单 **内容变化 2 项**，均落在 M17d/acc-12 修复面内：

  | 文件 | M17c | M17e | 变化 |
  |---|---|---|---|
  | `crates/sr-pipeline/src/finalize.rs` | 46,417；`4CDB2CA6…6E00` | 54,001；`801E3567…9298` | +7,584 |
  | `tools/ui/settings-sample-rate.mjs` | 20,952；`4A2067DB…FEF6` | 23,874；`090D01D6…234E` | +2,922 |

  - 其余 27 项主清单内容不变（含 `route.rs`/`naming.rs`/`recovery.rs`/`commands.rs`/
    `settings.rs`/`ui/*`/`inputs.rs`/`merge.rs`）。
  - `ui/api.js` mtime 变为 00:31:15，但 SHA-256 与 M17c 完全相同
    （`CA70C3AB…EB5F`）：M17d 红测期间曾临时变异该文件并在恢复后取指纹，
    仅时间戳变化，内容未变。
  - 本轮增量构建仅重编 `sr-pipeline`/`screen-rec`；`sr-merge` 及其他依赖 crate 源码未变。
- **补充清单（runner + 测试文件 + spec）**：

  | 文件 | SHA-256 | 大小 | mtime | 相对 M17c |
  |---|---|---|---|---|
  | `tools/ui/settings-sample-rate.mjs`（runner） | `090D01D6F339CF47B38F355F2FD6A1C5331AA85A33D721288C0C840A6623234E` | 23,874 | 2026-10-19 00:30:37 | +2,922（同主清单） |
  | `tools/ui/audio-sources-builder.mjs`（runner） | `D71D37BA06B59AEE21E15169B98B03EFAF93750AAE757E590EEEFDEBB7217747` | 30,456 | 2026-10-18 21:14:47 | 未变 |
  | `tests/sr-itests/tests/sync_dual.rs` | `EAD7829167B67F07BDB974410FFF55BA319CBAB806C2809EB8F395E835846780` | 30,469 | 2026-10-18 23:21:21 | 未变 |
  | `tests/sr-itests/tests/container.rs` | `106281444BACF601CD41B0C60F9A50413121CC1F79F2FD4D212DE15CEAD0199D` | 46,905 | 2026-10-19 00:03:10 | 未变 |
  | `specs/002-recording-modes.md` | `7FE5C369DC1C3ACABFE0BF7B80847EDAB7D94AF3540523FE7FDC0E5888D0495B` | 29,453 | 2026-10-18 23:25:57 | 未变 |

  - 测试/spec 为既有工作树内容；本任务只打包，未改这些文件；bundle resources 仍仅
    ffmpeg/ffprobe/licenses，测试/spec 不随安装包。
- **相对 M17c 报告合计内容差异 2 个文件**：主清单 2 项（`finalize.rs`、
  `settings-sample-rate.mjs`）；补充清单另 3 项（两个测试文件、spec）全部未变；
  runner 在补充清单中与主清单重复，不重复计数。
- 源码指纹采集后至构建/安装/卸载完成，内容未再变化；三次哈希一致可证本任务未改源码。

打包资源（本轮未改）：

| 文件 | SHA-256 | 说明 |
|---|---|---|
| `src-tauri/resources/licenses/ffmpeg-source-offer.txt` | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` | 精确 commit + 四种获取方式；3,411 B |
| `src-tauri/resources/licenses/FFmpeg-LICENSE.txt` | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | 与构建包 LICENSE.txt 一致 |

## 2. 构建

工具链（本轮实测）：Node v24.18.0 / npm 11.16.0；rustc 1.99.0 / cargo 1.99.0；
`tauri-cli 2.12.3`（npx）。

```
npx --yes @tauri-apps/cli@2.12.3 build
→ Compiling sr-pipeline v0.1.0
→ Compiling screen-rec v0.1.0
→ Finished `release` profile [optimized] target(s) in 21.19s
→ Built application at: ...\target\release\screen-rec.exe
→ Info Patching ...\screen-rec.exe with bundle type information: nsis
→ Running makensis to produce ...\target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe
→ Finished 1 bundle at: ...\Screen_rec_0.1.0_x64-setup.exe (82.62 MiB)
→ TAURI_BUILD_EXIT=0
```

- 构建开始：2026-10-19 **00:36:23**；结束：**00:38:31**（128.8 s；
  `m17ep-build-nsis.log`）。Rust 段 21.19 s；仅重编 `sr-pipeline`/`screen-rec`。
- 构建前 `screen-rec.exe` 进程数 0、`ffmpeg`/`ffprobe` 进程数 0，exe 未被锁定，未触发重试。
- 构建日志中的 informational stderr 属 npx/tauri 正常输出；`TAURI_BUILD_EXIT=0` 且产物完整。

产物（覆盖旧包）：

| 项 | 旧包（M17c，被覆盖） | 新包（M17e，本轮） |
|---|---|---|
| 路径 | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe` | 同左 |
| 构建时间（mtime） | 2026-10-19 00:10:36 | **2026-10-19 00:38:31** |
| 大小 | 86,688,765 B（82.67 MiB） | **86,631,008 B（82.62 MiB，-57,757 B）** |
| SHA-256 | `3BF1DABEDCBF5EDD2F1196D55F6298775227273757675A2BA6878C8CDAC40901` | **`B47803C3950ABE5657CAFE882226F9C017F68E1546D360B322D534906C914429`** |

- release 主程序：`target\release\screen-rec.exe`，15,051,264 B（较 M17c 的
  15,050,752 B 增加 512 B），mtime 00:38:31，
  SHA-256 `224E4989DE2CD8BE86DAE6AC97B6D483E8199ED2F294D2EAE4207DEB8C6E6ADC`。
- 安装包比 M17c 小 57,757 B、而 release 主程序仅增大 512 B：NSIS 压缩率随重编后的
  指令/调试字节变化属正常波动；安装清单完整（§3），新增生产字符串/UI 资产均已在
  release 与安装副本中逐一验证（见下），不是缺文件或旧包未覆盖。

**M17e 代码进包证据**：

- 本轮重编 `sr-pipeline`/`screen-rec`（见构建日志）；acc-12 新增生产字符串在 release
  exe 与安装副本中均为 True（`m17ep-markers-release.log` / `m17ep-markers-installed.log`）：

  | 标记 | 归属 | release | installed |
  |---|---|---|---|
  | `按既有行为继续（不硬拒绝）` | P2-B MKV 仅日志 | True | True |
  | `收尾空间不足（可用 ` | P2-B 预算日志 | True | True |
  | `收尾空间预算：新增 ` | P2-B 预算日志 | True | True |
  | `新增空间预算 ` | P2-B/P1-3 MP4 错误归因 | True | True |
  | `总峰值 ` | P2-B/P1-3 诊断 | True | True |
  | `磁盘空间不足（收尾需新增 ` | P1-3 增量空间预算 | True | True |
  | `容器族不符：format_name=` | P1-3 容器族校验（继承） | True | True |
  | `[route] 批量回滚：收尾等待超时；继续等待 worker 终止以确认 sidecar 回收` | P1-2 确定性回收（继承） | True | True |
  | `成品扩展名不受支持（` | P1-3 未知扩展名拒绝（继承） | True | True |
  | `已取消（批量回滚），未提交成品` | P1-2 提交门取消（继承） | True | True |
  | `批量回滚：后台收尾已结束` | P1-1 回滚分支（继承） | True | True |
  | `remux.mp4.tmp` / `concat.mkv.tmp` / `会话级占用失败` / `output_container` | M17 继承 | True | True |
  | `render_endpoint 音源仅适用于显示器目标` / `E_INVALID_AUDIO_PLAN` / `audio_sample_rate` | M16/既有继承 | True | True |
  | `MP4 校验失败` / `收尾峰值` / `取消太晚` | M17/acc-11 旧术语或仅注释 | False | False |

  - 旧术语 `MP4 校验失败` / `收尾峰值` 在 acc-11 已替换为通用校验与增量预算措辞，
    `取消太晚` 仅存在于注释；三者 False 为预期，不是功能回退。

- **UI 进包证据**：`ui/*` 源码自 M17c 后未变，Tauri 复用 23 个 `tauri-codegen-assets`
  （`target\release\build\screen-rec-ee450db19d850100\out\tauri-codegen-assets`）。
  本轮逐项复验（`m17ep-ui-integrity.log`）：9 个当前源码资产全部解压哈希相等，
  压缩字节在 release exe 与安装副本中均命中（9/9 `PASS`）；其余 14 个历史/孤立资产
  在 release exe 中 `inExe=false`，证明包内 UI 不是旧缓存。
- `tools/ui/settings-sample-rate.mjs` 属 runner，**不随安装包**；其 M17d 改动以源码
  指纹 + `m17-report.md` §8 的 red→green 证据为准（本任务未重跑 UI runner）。

完整日志：`docs/milestones/m17ep-build-nsis.log`（UTF-8；`TAURI_BUILD_EXIT=0`）。

## 3. 静默安装 + 安装目录自检

```
安装：Screen_rec_0.1.0_x64-setup.exe /S /D=C:\Users\elvis\AppData\Local\Temp\sr_m17ep_install
→ installer_exit=0；install_dir_exists=True（安装 2026-10-19 00:39:27–00:39:34，7.3 s）
```

安装目录清单（实测）：

| 文件 | 大小 | mtime | SHA-256 |
|---|---:|---|---|
| `screen-rec.exe` | 15,051,264 B | 2026-10-19 00:36:46 | `2F6D6F96F05287EB3FEFBC2FB084CD0FE97409726B06F72A39F34BCB903D95A9` |
| `uninstall.exe` | 79,390 B | 2026-10-19 00:39:34 | `B8712ADAAC20514E4751D05CF4152537EA2E3BAA2B079256F386EBE0A4C6B387` |
| `ffmpeg\ffmpeg.exe` | 134,092,288 B | 2026-10-15 12:43:48 | `3B57117B384DF98F2B8D01876DF0A596A0B6EBF45F756B30F15D6F65AB8251C5` |
| `ffmpeg\ffprobe.exe` | 133,887,488 B | 2026-10-15 12:43:48 | `5B9724EE7A115D5A8458D41BD2E0CE6206847E2B76105E822061A4421322CDE9` |
| `licenses\FFmpeg-LICENSE.txt` | 7,651 B | 2026-10-15 12:43:48 | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` |
| `licenses\ffmpeg-source-offer.txt` | 3,411 B | 2026-10-17 20:19:36 | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` |

- 安装副本 `screen-rec.exe` 与 release exe 大小相同（15,051,264 B），哈希不同
  （`2F6D6F96…95A9` vs `224E4989…6ADC`）：Tauri 向 release 主程序写入 bundle 类型信息后
  再打包（M12-G/M14/M15/M15c/M16/M16c/M17/M17c 同现象）。
- 安装副本二进制标记：§2 的 M17e/M17c/M17/M16/既有标记全部为预期值
  （`m17ep-markers-installed.log`）；9 个 UI 资产压缩字节在安装副本中同样原样存在
  （9/9 `inInstalledExe=true`）。
- ffmpeg/ffprobe/licenses 哈希与 M16c/M17/M17c 包逐一相同；source offer 未变。

`--self-check`（安装目录运行；完整输出 `docs/milestones/m17ep-self-check.log`，
2026-10-19 00:39:39–00:39:40）：

```
[self-check] 搜索根（按优先级）
  env  SCREENREC_FFMPEG_DIR: (未设置)
  tauri resource_dir: (未设置)
  exe 同目录: C:\Users\elvis\AppData\Local\Temp\sr_m17ep_install
  开发回退: ...\Screen_rec\crates\sr-core\../../tools/ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin
[self-check] ffmpeg: ...\sr_m17ep_install\ffmpeg\ffmpeg.exe（来源：exe-adjacent）
[self-check]   -version: ffmpeg version n8.1.3-18-g46d899d7a0-20261015 ...
[self-check] ffprobe: ...\sr_m17ep_install\ffmpeg\ffprobe.exe（来源：exe-adjacent）
[self-check]   -version: ffprobe version n8.1.3-18-g46d899d7a0-20261015 ...
SELF_CHECK_EXIT=0
```

- `screen-rec.exe` 为 GUI 子系统：使用 `Start-Process -Wait
  -RedirectStandardOutput/-RedirectStandardError` 取得完整输出与退出码。重定向输出文件为
  UTF-8；控制台按本地代码页显示中文可能乱码，不影响日志文件内容。

卸载与产物复核：

```
uninstall.exe /S → uninstall_exit=0（1.2 s）；dir_exists_after_uninstall=False
残留进程数：screen-rec=0，ffmpeg=0，ffprobe=0
安装包 SHA-256（卸载后）→ B47803C3950ABE5657CAFE882226F9C017F68E1546D360B322D534906C914429（与构建时一致）
```

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
| `docs/milestones/m17e-package-report.md` | 本报告（新增） |
| `docs/milestones/m17ep-build-nsis.log` | NSIS 构建完整原始输出 + 包装时间/哈希（`TAURI_BUILD_EXIT=0`，新增） |
| `docs/milestones/m17ep-fingerprint.log` | 29 项主清单 + 补充清单三次记录、产物/资源哈希（新增） |
| `docs/milestones/m17ep-ui-integrity.log` | 23 个 codegen 资产解压/源码比对、release 与安装副本命中（新增） |
| `docs/milestones/m17ep-markers-release.log` | release exe M17e/M17c/M17/M16/既有标记 + 旧术语复核（新增） |
| `docs/milestones/m17ep-markers-installed.log` | 安装副本 M17e/M17c/M17/M16/既有标记（新增） |
| `docs/milestones/m17ep-self-check.log` | 安装副本 self-check 原始 stdout/stderr（`SELF_CHECK_EXIT=0`，新增） |
| `docs/milestones/m17ep-install.log` | 安装清单/哈希、安装/卸载时间与退出码、产物哈希复核（新增） |
| `docs/milestones/m17c-package-report.md` | 顶部加一段指向本报告（其 00:10 旧包已被覆盖） |

未改任何源码/测试/`specs/`；§1 三次指纹一致可证。

> 说明：`docs/milestones/*.log` 被 `.gitignore` 的 `*.log` 忽略（与 M11–M17 报告相同），
> 日志已落盘但默认不进入 git；如需提交日志，请编排者显式 `git add -f`。

## 6. 未决 / 风险（回交编排者）

1. **未签名**：NSIS 安装包仍未签名（同 M11-C/M12-G/M14/M15/M15c/M16/M16c/M17/M17c），
   生产分发需证书与签名步骤。
2. **未在干净机安装**：本机已有 WebView2；安装器对 WebView2 前置的引导未在无运行时
   机器审计（同前序报告）。
3. **包内 UI 未做运行时点击复验**：本任务只做“构建 + 安装 + `--self-check`”+ 资产/标记核验；
   M17d 的 UI 双向快照检查（真实录制中改容器 → 收尾仍按启动快照）只在源码 runner
   中验证，证据见 `m17-report.md` §8.1 / `m17d-ui-settings.log`。若验收要求安装包内
   真实点击回归，需 GUI 运行（不在本任务范围）。
4. **长测未运行**：按约束未跑电池/sync/soak；本报告不据此宣称长稳。
5. **release 级 MP4 收尾未运行**：本次仅验证 acc-12 相关生产字符串与 UI 资产进包，
   未在安装包内执行 MP4 收尾或播放器验证。
6. **acc-12 修复的运行时证据来自源码测试**：M17d 的 red→green 证据在
   `m17-report.md` §8.2（`m17d-red-p2b.log` / `m17d-green-p2b.log`）与 §8.3；
   本任务未在安装包内重跑；包内证据为构建重编日志 + 生产字符串 + UI 资产哈希。
7. **测试/spec/runner 改动未计入 29 项主指纹**：`tests/sr-itests/tests/{sync_dual.rs,container.rs}`、
   `specs/002-recording-modes.md`、两个 runner 为既有工作树内容（补充哈希见 §1）；
   bundle resources 不变，测试/spec 不随包；runner 不随包。
8. **安装后 exe 哈希与 release exe 不同**：Tauri 写入 bundle 类型信息后再打包；
   安装副本与构建目录副本因此不必逐字节相同（同 M15c/M16/M16c/M17/M17c）。
9. **安装包体积变化非单调**：本轮安装包比 M17c 小 57,757 B，而 release exe 增大 512 B；
   已用安装清单、生产字符串标记与 9/9 UI 资产命中排除缺文件/旧缓存，压缩率波动为
   最可能解释。

## 7. 证据索引（本机实际）

| 证据 | 内容 |
|---|---|
| `docs/milestones/m17ep-build-nsis.log` | `npx @tauri-apps/cli@2.12.3 build` 原始输出；00:36:23–00:38:31，exit 0，82.62 MiB |
| `docs/milestones/m17ep-fingerprint.log` | 29 项 SHA-256 构建前 00:36:18、构建后 00:38:59、安装卸载后 00:40:23；`POST_BUILD_DIFF_COUNT_VS_PRE_BUILD=0`、`FINAL_DIFF_COUNT_VS_PRE_BUILD=0`；产物/资源哈希 |
| `docs/milestones/m17ep-ui-integrity.log` | 23 个 codegen 资产解压 SHA-256、9 个当前源码命中、14 个孤立资产 `inExe=false`、release/安装副本压缩字节命中 |
| `docs/milestones/m17ep-markers-release.log` | release exe 的 M17e/M17c/M17/M16/既有标记与旧术语替换复核 |
| `docs/milestones/m17ep-markers-installed.log` | 安装副本标记（全部为预期值） |
| `docs/milestones/m17ep-self-check.log` | 安装副本 self-check 原始 stdout/stderr（`SELF_CHECK_EXIT=0`，00:39:39） |
| `docs/milestones/m17ep-install.log` | 安装清单/哈希、安装/卸载时间与退出码、产物哈希复核 |
| 源码指纹 | §1（`Get-FileHash <file> -Algorithm SHA256`） |
| 安装/卸载 | §3（`installer_exit=0`、`uninstall_exit=0`、目录已删除、产物哈希复核一致） |
| acc-12 行为证据 | `docs/milestones/m17-report.md` §8（L1/UI 日志） |
| M17c 行为/打包证据 | `docs/milestones/m17c-package-report.md`、`m17-report.md` §7 |
