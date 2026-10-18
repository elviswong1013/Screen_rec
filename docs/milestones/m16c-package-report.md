# M16c 安装包重建（NSIS · 指纹 · 安装/自检）

- 日期：2026-10-18
- 范围：M16 包（2026-10-18 21:05，SHA-256 `879399CE…34FD3`）之后源码完成
  **acc-9 两项 P1 修复**——`ui/app.js` 终态单调护栏（`registerRoute`/`mergePending`）与
  Ambiguous 移除 `firstActive`、`ui/api.js` ambiguous mock 夹具、
  `tools/ui/audio-sources-builder.mjs` 新增 10 项回归——证据
  `docs/milestones/m16-report.md` §7。从当前最终源码重建 NSIS 并复验安装/自检。
- 流程：沿用 `docs/milestones/m11-c-report.md` / `m12-g-report.md` /
  `m14-package-report.md` / `m15-package-report.md` / `m15c-package-report.md` /
  `m16-package-report.md`（`npx --yes @tauri-apps/cli@2.12.3 build`，
  resources 映射 ffmpeg/ffprobe/licenses）。
- 约束遵守：只做打包与报告；未改源码/测试/`specs/`；未执行任何 `git` 写操作；
  **未跑长测/未跑测试**（L1/L2/UI 结果由编排者按 `m16-report.md` §7 既有证据使用）。
  构建前检查无 `screen-rec.exe`/`ffmpeg` 进程驻留，未出现 exe 锁定失败。

## 0. 结论

| 项 | 状态 | 关键证据 |
|---|---|---|
| 源码指纹 | **已记录（构建前/构建后/安装卸载后三次一致）** | §1：与 M16 包同清单 26 项；构建前 21:20:43 记录，21:26:58 与 21:28:04 两次重算全部相同（`FINGERPRINT_DIFF_COUNT=3` 均为 acc-9 三项，`FINAL_DIFF_COUNT_VS_PRE_BUILD=0`） |
| NSIS 重建 | **完成，已覆盖 M16 21:05 旧包** | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe`；2026-10-18 21:26:39；86,634,086 B（82.62 MiB）；SHA-256 `1118B8DD…290D` |
| M16c 代码进包 | **已验证** | 本轮重编 `screen-rec`；更新后的 `ui/app.js`/`ui/api.js` 解压 SHA-256 等于当前源码，且压缩字节在 release exe 与安装副本中原样存在；旧缓存资产不在 exe 中；M16 错误串与继承标记均为 True |
| 静默安装 + 自检 | **exit 0** | `%TEMP%\sr_m16cp_install`；ffmpeg/ffprobe 来源 `exe-adjacent`；版本 `n8.1.3-18-g46d899d7a0-20261015` |
| 卸载清理 | **完成** | `uninstall.exe /S` exit 0；安装目录已删除；无 `screen-rec`/`ffmpeg` 残留进程 |
| 产物哈希复核 | **一致** | 卸载后安装包 SHA-256 仍为 `1118B8DD…290D` |
| ffmpeg source offer | **保持精确 commit** | 未改动；SHA-256 `FF1A3F3E…55808`；安装目录内同名文件哈希一致；commit `46d899d7a02bc63237472cc4897258d5ddf274ac` |
| 长测 | **未运行（按约束）** | 仅打包/安装/自检 |

**本报告描述的 21:26 安装包取代 M16（21:05）安装包**；`m16-package-report.md`
顶部已加指向本报告的说明。

## 1. 源码指纹（构建前记录，构建后/安装后复核一致）

记录时间：2026-10-18 21:20:43（本机）。最晚源码 mtime = **2026-10-18 21:16:35**
（`ui/app.js`；`tools/ui/audio-sources-builder.mjs` 21:14:47、`ui/api.js` 21:14:37
紧随其后）；最终构建开始 21:24:07，新包 mtime = **2026-10-18 21:26:39**。
清单与 `m16-package-report.md` §1 完全一致，共 **26 项**（含本轮改动的
`ui/app.js`、`ui/api.js`、`tools/ui/audio-sources-builder.mjs`）。

| 文件 | SHA-256 | 大小 | mtime |
|---|---|---|---|
| `Cargo.lock` | `5268A66731DC03F07653A0A3072B0E1704055742BCAB35724B3617BC66966AAF` | 122,645 | 2026-10-17 19:32:45 |
| `src-tauri/tauri.conf.json` | `95080F11734FA35F3BF28B3B51B01A4D8EA28A1AF742B4FD306FDDE040F819FC` | 899 | 2026-10-18 14:40:47 |
| `src-tauri/src/region.rs` | `A9A04F7C82E5E54894BA3ABAB0092BAA619D954DE741A6DE296C5D4310B96D24` | 13,610 | 2026-10-17 21:01:09 |
| `src-tauri/src/lib.rs` | `46882CE1257AB7775099217BE14B7CF7750352396E0124CD8230F2C6C7108CD0` | 12,040 | 2026-10-18 19:41:18 |
| `src-tauri/src/commands.rs` | `230059F6637334CFC0493EA41DFD403DA374979AA94152375D3D29A0163AA487` | 81,616 | 2026-10-18 20:53:17 |
| `src-tauri/src/settings.rs` | `92018FD53541187DAA6A31B730305231B329C0CFA20A05ACDB5D35E7E18741C0` | 22,131 | 2026-10-18 15:37:16 |
| `crates/sr-pipeline/src/route.rs` | `34E7272AC7A371DCFB0E8F7FA3A1DD250B686FD456FFFDD956DF12A058EF4EB1` | 107,578 | 2026-10-18 20:50:58 |
| `crates/sr-pipeline/src/naming.rs` | `5BCA89EE4D00C28AF50CB941D276DDD22ADB178B95C082A6C58D85D3C243EB4B` | 7,957 | 2026-10-17 21:00:19 |
| `crates/sr-pipeline/src/recovery.rs` | `A9BBC09167C2DD9066F4BD570BF807EB5949175F2224334309B12F8B1D5142D8` | 41,634 | 2026-10-18 16:51:48 |
| `crates/sr-pipeline/src/sidecar.rs` | `44FBDE79404689F6319FBCF1CAEEE95EC56AEC9DC393CF06F630ED3C5F00832B` | 17,343 | 2026-10-18 15:36:42 |
| `crates/sr-pipeline/src/finalize.rs` | `FF1809833AEAB57CEC4C620FAA424F610BB61B530FE2C6E45777561BCBCDE8BA` | 10,981 | 2026-10-18 16:51:48 |
| `crates/sr-audio/src/resample.rs` | `66711E62512F189968A39ABBD40765AB6D7A9CA1C81EDD16E17CA49E5B235041` | 19,510 | 2026-10-18 15:31:02 |
| `crates/sr-audio/src/wasapi.rs` | `A45C972895CE5DA75E4B377B44EBF96C7A7749A1396BA4706A2C8C5A8F378842` | 58,508 | 2026-10-18 20:51:45 |
| `crates/sr-audio/src/slots.rs` | `19FD8BBE37599801C6895A62C5E13ED2B6E0F6E4D9349B4EDCC7D0010925DA12` | 9,218 | 2026-10-18 19:58:01 |
| `crates/sr-capture/src/wgc.rs` | `90B244810A5BED028E5233CC12B2E3C7A6504C6D543A5BD6ECF1BCC83BA12928` | 10,824 | 2026-10-17 22:28:49 |
| `crates/sr-enum/src/monitors.rs` | `DF2BB30A00148C7678436B90EF2D342717F2EF4C17420590C85D2AC57A28CD13` | 12,655 | 2026-10-18 20:01:11 |
| `crates/sr-enum/src/render_endpoints.rs` | `CC63C4E7DCC8D5DA1583DFC4CDE17EA5B80710E6D5EDC5E300F9B92B996E7244` | 11,091 | 2026-10-18 20:01:11 |
| `crates/sr-core/src/events.rs` | `3E0086BFE8C793DDB8E87222C5FA31E267AD62B9F6AA1EC5A0D4FAF5B95B3CF6` | 12,383 | 2026-10-18 20:53:04 |
| `ui/region.js` | `AAA0CD9AD104E521BDBD2B5DDACB714DB5AB25ED481BE951B739A2D40001D9A4` | 5,769 | 2026-10-17 21:01:13 |
| `ui/app.js` | `07F0FD8223AAB8FB28DCC2FBE8F27CE7D0F4F062970FDA95475758746A8D6E15` | 65,099 | 2026-10-18 21:16:35 |
| `ui/api.js` | `97CCDA96B1237ACC1599BDEE037506CBF41B11FD42C2A9339D2BB7F469A47C1D` | 26,828 | 2026-10-18 21:14:37 |
| `ui/endpoint-match.js` | `EF81FB3BF3F3726B2BB656975DD1652350CCD15C1A63CA8E3A22F05F7A54BC74` | 8,202 | 2026-10-18 19:35:07 |
| `ui/route-state.js` | `D93A441AB7EB8A826E89C7DA9CBAD4CC70CE5CFCC2514861CD5FD611D6349694` | 4,202 | 2026-10-18 19:46:03 |
| `crates/sr-audio/tests/stall_pad_policy.rs` | `60553CAE593D3B80E4074A32647EFF70B6CC90D9C667F5DD7449DDE9F3CE0733` | 6,000 | 2026-10-17 23:42:13 |
| `tools/ui/settings-sample-rate.mjs` | `1CC1AF04D9F7233C55D45C9F0129ED785557F0E0920DFA6B2BD7FDD5CCCEE186` | 15,488 | 2026-10-18 16:11:49 |
| `tools/ui/audio-sources-builder.mjs` | `D71D37BA06B59AEE21E15169B98B03EFAF93750AAE757E590EEEFDEBB7217747` | 30,456 | 2026-10-18 21:14:47 |

说明：

- 构建后（21:26:58）与安装/卸载后（21:28:04）对上述 26 项重算 SHA-256，**逐项与
  构建前相同**（`m16cp-fingerprint.log`：`FINAL_DIFF_COUNT_VS_PRE_BUILD=0`、
  `FINAL_COMPARED_ITEMS=26`）。
- 相对 M16 包指纹（同 26 项）**内容变化 3 项，正是 acc-9 修复文件**：
  - `ui/app.js`：`A6ADEAA1…A6412`（63,849 B）→
    `07F0FD82…8D6E15`（65,099 B，+1,250 B）；
  - `ui/api.js`：`3159B97C…DBCE`（26,134 B）→
    `97CCDA96…A47C1D`（26,828 B，+694 B）；
  - `tools/ui/audio-sources-builder.mjs`：`14595AD9…D68E3`（24,234 B）→
    `D71D37BA…217747`（30,456 B，+6,222 B）；
  - 其余 23 项内容不变（含全部 Rust 源码、`Cargo.lock`、`tauri.conf.json`、
    `ui/endpoint-match.js`、`ui/route-state.js`）。
  与 `m16-report.md` §7.5「本次修复仅 UI 层与测试工具」的清单一致。
- 任务清单外的同期改动（未计入 26 项指纹）：`specs/002-recording-modes.md`
  （21:01:17）、`ui/index.html`（19:35:18）、`crates/sr-enum/{Cargo.toml,src/lib.rs}`、
  新增测试 `crates/sr-enum/tests/*`、`tests/sr-itests/tests/*` 等；均为 M16 既有工作树
  内容，本任务只打包，未改这些文件。`ui/index.html` 虽不在 26 项清单内，仍单独核验进包（§2）。
- `crates/sr-audio/tests/stall_pad_policy.rs`、`tools/ui/*.mjs` 与 `tests/sr-itests/*`
  不随安装包（bundle resources 仍仅 ffmpeg/ffprobe/licenses），按任务要求计入指纹。

打包资源（本轮未改）：

| 文件 | SHA-256 | 说明 |
|---|---|---|
| `src-tauri/resources/licenses/ffmpeg-source-offer.txt` | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` | M12-C 修正版：精确 commit + 四种获取方式；3,411 B |
| `src-tauri/resources/licenses/FFmpeg-LICENSE.txt` | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | 与构建包 LICENSE.txt 一致 |

## 2. 构建

工具链：Node v24.18.0 / npm 11.16.0；rustc 1.99.0 / cargo 1.99.0；
`tauri-cli 2.12.3`（npx）。

```
npx --yes @tauri-apps/cli@2.12.3 build
→ Compiling screen-rec v0.1.0
→ Finished `release` profile [optimized] target(s) in 22.67s
→ Built application at: ...\target\release\screen-rec.exe
→ Info Patching ...\screen-rec.exe with bundle type information: nsis
→ Running makensis to produce ...\target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe
→ Finished 1 bundle at: ...\Screen_rec_0.1.0_x64-setup.exe (82.62 MiB)
→ TAURI_BUILD_EXIT=0
```

- 最终构建开始：2026-10-18 **21:24:07**；结束：**21:26:39**（152.2 s；
  `m16cp-build-nsis.log`）。
- 本轮源码相较 M16 包仅 UI/测试工具变化，Rust 源码未变，因此只重编
  `screen-rec`（22.67 s，含 UI codegen 资产），随后 makensis 打包。
- 构建前 `screen-rec.exe` 进程数 0、`ffmpeg` 进程数 0，exe 未被锁定，未触发重试。
- 说明：本轮先于 21:20:51–21:23:27 执行过一次构建并成功产出中间包；因构建日志
  需规范化为 UTF-8，于 21:24:07–21:26:39 重新执行最终构建，中间包被覆盖且从未安装。
  以下时间、大小与哈希均以**最终构建**为准。

产物（覆盖旧包）：

| 项 | 旧包（M16，被覆盖） | 新包（M16c，本轮） |
|---|---|---|
| 路径 | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe` | 同左 |
| 构建时间（mtime） | 2026-10-18 21:05:49 | **2026-10-18 21:26:39** |
| 大小 | 86,597,996 B（82.59 MiB） | **86,634,086 B（82.62 MiB，+36,090 B）** |
| SHA-256 | `879399CE2EF2DA9E3F3B1941C943B49E93639C27250098B9C3EA3DAE69134FD3` | **`1118B8DD5FFC82ECFA2BFCE7560EB83AA4342391C039F6CD9A3864BDADAA290D`** |

- release 主程序：`target\release\screen-rec.exe`，15,006,720 B（较 M16 的
  15,006,208 B 增加 512 B），mtime 21:26:39，
  SHA-256 `2B875F8687C66618AD7F93B4CAFE439B994F6835AA23616F81135AD5F159560B`。
- **UI 进包证据**：本轮 Tauri 重新生成 `tauri-codegen-assets`
  （首次中间构建 21:20:55 生成；最终构建因输入未变复用同一内容，
  目录 `target\release\build\screen-rec-ee450db19d850100\out\tauri-codegen-assets`）；
  解压后 SHA-256 与当前源码逐项相等：

  | 源码 | 资产文件（brotli） | 压缩 B | 解压 B | 解压 SHA-256 | 对照 |
  |---|---|---:|---:|---|---|
  | `ui/app.js` | `854b8101…351ce.js` | 17,314 | 65,099 | `07F0FD82…8D6E15` | = 当前 `ui/app.js`（本轮更新） |
  | `ui/api.js` | `36760c3e…f6841.js` | 8,664 | 26,828 | `97CCDA96…A47C1D` | = 当前 `ui/api.js`（本轮更新） |
  | `ui/endpoint-match.js` | `e8565f57…852a1.js` | 2,750 | 8,202 | `EF81FB3B…BC74` | = 当前 `ui/endpoint-match.js`（未变） |
  | `ui/route-state.js` | `23061bfb…fd8db.js` | 1,479 | 4,202 | `D93A441A…9694` | = 当前 `ui/route-state.js`（未变） |
  | `ui/index.html` | `9bc926e8…176e.html` | 2,245 | 7,863 | `43C8C424…EBFA` | = 当前 `ui/index.html`（未变） |
  | `ui/region.js` | `a15c6368…8dd3b.js` | 2,246 | 5,769 | `AAA0CD9A…D9A4` | = 当前 `ui/region.js`（未变） |

  进一步核验：
  - 上述 6 个资产的 brotli 压缩字节在 `target\release\screen-rec.exe` 中**原样逐字节存在**
    （`indexOf` 命中），且 `ui/region.html`、`ui/styles.css`、`ui/region-geometry.js`
    三个未变资产同样解压哈希等于源码并被嵌入；
  - 构建目录中仍残留 M16 旧缓存资产（旧 `app.js` `a0060ed2…e962.js`、旧 `api.js`
    `7fed8c0b…f017.js`），但它们在 release exe 中**均未命中**（`inExe=false`），
    证明包内 UI 不是旧缓存。旧缓存仅存在于 `target\release\build` 工作目录，不随包分发。
- 二进制标记（安装副本实测，UTF-8 字节匹配）：M16 新增
  `render_endpoint 音源仅适用于显示器目标`=True、`显示器目标没有任何音源`=True、
  `纯录音不接受 render_endpoint 音源`=True、`音频采集端点失效`=True；继承
  `E_INVALID_AUDIO_PLAN`/`stop_notice`/`keep_segments`/`audio_sample_rate`/
  `SR_DIAG_AV` 全部 True（§3）。
- 新包比 M16 大 36,090 B：release 主程序 +512 B，UI 资产压缩字节变化
  （app +400 B、api +167 B）与 NSIS 压缩/元数据差异共同导致；安装清单完整（§3），
  不是缺文件。

完整日志：`docs/milestones/m16cp-build-nsis.log`（UTF-8；`TAURI_BUILD_EXIT=0`）。

## 3. 静默安装 + 安装目录自检

```
安装：Screen_rec_0.1.0_x64-setup.exe /S /D=C:\Users\elvis\AppData\Local\Temp\sr_m16cp_install
→ installer_exit=0；install_dir_exists=True（安装 2026-10-18 21:27:05–21:27:13，8.3 s）
```

安装目录清单（实测）：

| 文件 | 大小 | mtime | SHA-256 |
|---|---:|---|---|
| `screen-rec.exe` | 15,006,720 B | 2026-10-18 21:24:32 | `3C374079C5CB6F98C863A31E8CAF6D52F4F0F8BDF337EF3E5F52357DF94A3E9B` |
| `uninstall.exe` | 79,390 B | 2026-10-18 21:27:12 | `B8712ADAAC20514E4751D05CF4152537EA2E3BAA2B079256F386EBE0A4C6B387` |
| `ffmpeg\ffmpeg.exe` | 134,092,288 B | 2026-10-15 12:43:48 | `3B57117B384DF98F2B8D01876DF0A596A0B6EBF45F756B30F15D6F65AB8251C5` |
| `ffmpeg\ffprobe.exe` | 133,887,488 B | 2026-10-15 12:43:48 | `5B9724EE7A115D5A8458D41BD2E0CE6206847E2B76105E822061A4421322CDE9` |
| `licenses\FFmpeg-LICENSE.txt` | 7,651 B | 2026-10-15 12:43:48 | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` |
| `licenses\ffmpeg-source-offer.txt` | 3,411 B | 2026-10-17 20:19:36 | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` |

- 安装副本 `screen-rec.exe` 与 release exe 大小相同（15,006,720 B），哈希不同
  （`3C3740…3E9B` vs `2B875F…560B`）：Tauri 向 release 主程序写入 bundle 类型信息后
  再打包（M12-G/M14/M15/M15c/M16 同现象）。
- 安装副本 `screen-rec.exe` 二进制标记实测：4 条 M16 错误串全部 True，
  `E_INVALID_AUDIO_PLAN`/`stop_notice`/`keep_segments`/`audio_sample_rate`/
  `SR_DIAG_AV` 全部 True；更新后的 `app.js`/`api.js` brotli 压缩字节在安装副本中
  同样原样存在（`IN_INSTALLED_EXE=True`，`m16cp-self-check.log`）。
- ffmpeg/ffprobe/licenses 哈希与 M16 包逐一相同；source offer 未变。

`--self-check`（安装目录运行；完整输出 `docs/milestones/m16cp-self-check.log`，
2026-10-18 21:27:20–21:27:21）：

```
[self-check] 搜索根（按优先级）
  env  SCREENREC_FFMPEG_DIR: (未设置)
  tauri resource_dir: (未设置)
  exe 同目录: C:\Users\elvis\AppData\Local\Temp\sr_m16cp_install
  开发回退: ...\Screen_rec\crates\sr-core\../../tools/ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin
[self-check] ffmpeg: ...\sr_m16cp_install\ffmpeg\ffmpeg.exe（来源：exe-adjacent）
[self-check]   -version: ffmpeg version n8.1.3-18-g46d899d7a0-20261015 Copyright (c) 2000-2026 the FFmpeg developers
[self-check] ffprobe: ...\sr_m16cp_install\ffmpeg\ffprobe.exe（来源：exe-adjacent）
[self-check]   -version: ffprobe version n8.1.3-18-g46d899d7a0-20261015 Copyright (c) 2007-2026 the FFmpeg developers
SELF_CHECK_EXIT=0
```

- `screen-rec.exe` 为 GUI 子系统：使用 `Start-Process -Wait
  -RedirectStandardOutput/-RedirectStandardError` 取得完整输出与 `SELF_CHECK_EXIT=0`。
  重定向输出文件为 UTF-8；控制台按本地代码页显示中文时可能乱码，不影响日志文件内容。

卸载与产物复核：

```
uninstall.exe /S → uninstall_exit=0；dir_exists_after_uninstall=False
残留进程数：screen-rec=0，ffmpeg=0
安装包 SHA-256（卸载后）→ 1118B8DD5FFC82ECFA2BFCE7560EB83AA4342391C039F6CD9A3864BDADAA290D（与构建时一致）
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
| `docs/milestones/m16c-package-report.md` | 本报告（新增） |
| `docs/milestones/m16cp-build-nsis.log` | NSIS 最终构建完整日志（`TAURI_BUILD_EXIT=0`，新增） |
| `docs/milestones/m16cp-fingerprint.log` | 26 项源码指纹三次记录与产物哈希（新增） |
| `docs/milestones/m16cp-self-check.log` | 安装清单/哈希、安装 exe 标记、self-check 原始输出、卸载与产物复核（新增） |
| `docs/milestones/m16-package-report.md` | 顶部加一段指向本报告（其 21:05 旧包已被覆盖） |

未改任何源码/测试/`specs/`；§1 三次指纹一致可证。

> 说明：`docs/milestones/*.log` 被 `.gitignore` 的 `*.log` 忽略（与 M11–M16 报告相同），
> 日志已落盘但默认不进入 git；如需提交日志，请编排者显式 `git add -f`。

## 6. 未决 / 风险（回交编排者）

1. **包内 acc-9 UI 行为未做运行时点击复验**：本任务只做“构建 + 安装 + `--self-check`”；
   acc-9 的红→绿证据（终态后重注册、注册前 failed + 旧状态、Ambiguous 不取首项、
   start DTO 绑定候选）在 `m16-report.md` §7.3（4 FAIL → 全通过）。安装包级证据为
   `app.js`/`api.js` 解压哈希等于源码、压缩字节存在于 exe、UI runner 日志
   `m16c-ui-green.log`。若验收要求安装包内真实点击回归，需 GUI 运行（不在本任务范围）。
2. **未签名**：NSIS 安装包仍未签名（同 M11-C/M12-G/M14/M15/M15c/M16），生产分发需证书与签名步骤。
3. **未在干净机安装**：本机已有 WebView2；安装器对 WebView2 前置的引导未在无运行时
   机器审计（同前序报告）。
4. **本轮构建执行两次**：第一次 21:20:51–21:23:27 为中间构建，因日志编码规范化需要
   重新构建，最终包为 21:24:07–21:26:39；中间包从未安装、已被覆盖。最终产物哈希见 §2。
5. **`list_render_endpoints` 字面量判据**：延续 `m16-package-report.md` §6 #2 的说明
   （命令名未形成连续 ASCII 字面量），本轮进包判据仍为 M16 错误串 + UI 资产 + 重编日志，
   未重新调查该字符串布局。
6. **长测未运行**：按约束未跑电池/sync/soak；本报告不据此宣称长稳。
7. **安装后 exe 哈希与 release exe 不同**：Tauri 写入 bundle 类型信息后再打包；
   安装副本与构建目录副本因此不必逐字节相同（同 M15c/M16）。
8. **M16/acc-9 测试与规格改动未计入 26 项指纹**：`specs/002-recording-modes.md`、
   `ui/index.html`、`crates/sr-enum/{Cargo.toml,src/lib.rs}`、`tests/sr-itests/*` 等
   为 M16/acc-9 既有工作树内容；本包不包含测试文件（bundle resources 不变）。

## 7. 证据索引（本机实际）

| 证据 | 内容 |
|---|---|
| `docs/milestones/m16cp-build-nsis.log` | `npx @tauri-apps/cli@2.12.3 build` 最终构建全量输出；21:24:07–21:26:39，exit 0，82.62 MiB |
| `docs/milestones/m16cp-fingerprint.log` | 26 项 SHA-256 构建前 21:20:43、构建后 21:26:58、安装卸载后 21:28:04；`FINGERPRINT_DIFF_COUNT=3`（均为 acc-9 文件）、`FINAL_DIFF_COUNT_VS_PRE_BUILD=0`；产物哈希 |
| `docs/milestones/m16cp-self-check.log` | 安装清单/哈希、安装 exe 标记、self-check 输出（`SELF_CHECK_EXIT=0`，21:27:21）、卸载与产物复核 |
| 源码指纹 | §1（`Get-FileHash <file> -Algorithm SHA256`） |
| UI 进包 | §2（`tauri-codegen-assets` 解压 SHA-256 = 当前源码；压缩字节在 release exe 与安装副本中原样存在；旧缓存资产 `inExe=false`） |
| 安装/卸载 | §3（`installer_exit=0`、`uninstall_exit=0`、目录已删除、产物哈希复核一致） |
| acc-9 行为证据 | `docs/milestones/m16-report.md` §7（`m16c-ui-red.log` 4 FAIL → `m16c-ui-green.log` 全通过；`m16c-ui-settings-regression.log`） |
| M16 行为证据 | `docs/milestones/m16-report.md` §4（L1、L2、UI runner）与 `m16-l2-*.log`、`m16-ui-*.log` |
| M15/M15c 行为证据 | `docs/milestones/m15-report.md`、`m15c-package-report.md` |
