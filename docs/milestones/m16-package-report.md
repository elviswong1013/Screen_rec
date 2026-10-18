# M16 安装包重建（NSIS · 指纹 · 安装/自检）

> **后续更新（M16c，2026-10-18 21:26）**：M16 包之后源码完成 acc-9 两项 P1 修复（`ui/app.js`
> 终态单调护栏/Ambiguous 不取首项、`ui/api.js` ambiguous 夹具、`tools/ui/audio-sources-builder.mjs`
> 新增回归），已从当前源码再次重建；**本报告的 21:05 安装包已被覆盖**。最新产物、指纹与安装/自检
> 结果见 `docs/milestones/m16c-package-report.md`。

- 日期：2026-10-18
- 范围：M15c 包（2026-10-18 17:08，SHA-256 `24268AC0…3FC2`）之后源码新增
  **M16（P2 全屏音轨，REQ-9..15）**——渲染端点枚举/显示器匹配、端点环回捕获与失效传播、
  逐屏音源选择、UI 路由终态护栏、新测试与 UI runner——证据
  `docs/milestones/m16-report.md`。从当前最终源码重建 NSIS 并复验安装/自检。
- 流程：沿用 `docs/milestones/m11-c-report.md` / `m12-g-report.md` /
  `m14-package-report.md` / `m15-package-report.md` / `m15c-package-report.md`
  （`npx --yes @tauri-apps/cli@2.12.3 build`，resources 映射 ffmpeg/ffprobe/licenses）。
- 约束遵守：只做打包与报告；未改源码/测试/`specs/`；未执行任何 `git` 写操作；
  **未跑长测/未跑测试**（L1/L2/UI 结果由编排者按 `m16-report.md` 既有证据使用）。
  构建前检查无 `screen-rec.exe`/`ffmpeg` 进程驻留，未出现 exe 锁定失败。

## 0. 结论

| 项 | 状态 | 关键证据 |
|---|---|---|
| 源码指纹 | **已记录（构建前/构建后/安装后三次一致）** | §1：任务枚举共 26 项 SHA-256（任务文字写“21 项”，按枚举实计 26，见 §6 #1）；21:02:13 记录，21:05:54 与 21:07:28 两次重算全部相同（`FINGERPRINT_DIFF_COUNT=0`、`FINAL_DIFF_COUNT=0`） |
| NSIS 重建 | **完成，已覆盖 M15c 17:08 旧包** | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe`；2026-10-18 21:05:49；86,597,996 B（82.59 MiB）；SHA-256 `879399CE…4FD3` |
| M16 代码进包 | **已验证** | 本轮重编 `sr-core`/`sr-audio`/`sr-enum`/`sr-capture`/`sr-pipeline`/`screen-rec`；release 与安装副本 exe 均含 4 条 M16 端点错误串；5 个更新 UI 资产（app/api/endpoint-match/route-state/index.html）解压 SHA-256 等于当前源码且压缩字节原样嵌入 release exe |
| 静默安装 + 自检 | **exit 0** | `%TEMP%\sr_m16p_install`；ffmpeg/ffprobe 来源 `exe-adjacent`；版本 `n8.1.3-18-g46d899d7a0-20261015` |
| 卸载清理 | **完成** | `uninstall.exe /S` exit 0；安装目录已删除 |
| 产物哈希复核 | **一致** | 卸载后安装包 SHA-256 仍为 `879399CE…4FD3` |
| ffmpeg source offer | **保持精确 commit** | 未改动；SHA-256 `FF1A3F3E…55808`；安装目录内同名文件哈希一致；commit `46d899d7a02bc63237472cc4897258d5ddf274ac` |
| 长测 | **未运行（按约束）** | 仅打包/安装/自检 |

**本报告描述的 21:05 安装包取代 M15c（17:08）安装包**；`m15c-package-report.md`
顶部已加指向本报告的说明。

## 1. 源码指纹（构建前记录，构建后/安装后复核一致）

记录时间：2026-10-18 21:02:13（本机）。最晚源码 mtime = **2026-10-18 20:53:17**
（`src-tauri/src/commands.rs`；`ui/app.js` 20:53:13、`crates/sr-core/src/events.rs`
20:53:04 紧随其后）；构建开始 21:02:16，新包 mtime = **2026-10-18 21:05:49**。
任务文字标注“21 项”，但逐项枚举为 **26 项**（较 M15c 的 20 项新增
`render_endpoints.rs`、`monitors.rs`、`events.rs`、`endpoint-match.js`、
`route-state.js`、`audio-sources-builder.mjs`）；本轮把枚举中的 26 项全部纳入指纹。

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
| `ui/app.js` | `A6ADEAA1AB2B7CFFB9437A094302BA9F554155B5DE1B634DE1BFD9A7A98A6412` | 63,849 | 2026-10-18 20:53:13 |
| `ui/api.js` | `3159B97C99149C308DB5F7680EA8BC04DB489F0CC2EDB749C1FDE35EE8D3DBCE` | 26,134 | 2026-10-18 19:35:35 |
| `ui/endpoint-match.js` | `EF81FB3BF3F3726B2BB656975DD1652350CCD15C1A63CA8E3A22F05F7A54BC74` | 8,202 | 2026-10-18 19:35:07 |
| `ui/route-state.js` | `D93A441AB7EB8A826E89C7DA9CBAD4CC70CE5CFCC2514861CD5FD611D6349694` | 4,202 | 2026-10-18 19:46:03 |
| `crates/sr-audio/tests/stall_pad_policy.rs` | `60553CAE593D3B80E4074A32647EFF70B6CC90D9C667F5DD7449DDE9F3CE0733` | 6,000 | 2026-10-17 23:42:13 |
| `tools/ui/settings-sample-rate.mjs` | `1CC1AF04D9F7233C55D45C9F0129ED785557F0E0920DFA6B2BD7FDD5CCCEE186` | 15,488 | 2026-10-18 16:11:49 |
| `tools/ui/audio-sources-builder.mjs` | `14595AD99F139AD63495706D12C960362FD9028E071F9548413C2BF0846D68E3` | 24,234 | 2026-10-18 19:46:03 |

说明：

- 构建后（21:05:54）与安装/卸载后（21:07:28）对上述 26 项重算 SHA-256，**逐项与
  构建前相同**（`m16p-fingerprint.log`：`FINGERPRINT_DIFF_COUNT=0`、`FINAL_DIFF_COUNT=0`）。
- 相对 M15c 指纹（20 项，其中 6 项本轮任务新增）：
  - **内容变化 7 项**：`lib.rs`（`B8108C…` → `46882C…`）、`commands.rs`
    （`311834…` → `230059…`）、`route.rs`（`DE181A…` → `34E727…`）、`wasapi.rs`
    （`6C26D9…` → `A45C97…`）、`slots.rs`（`44DB9D…` → `19FD8B…`）、`ui/app.js`
    （`E6EACD…` → `A6ADEA…`）、`ui/api.js`（`16E0CA…` → `3159B9…`）；
  - **新增 6 项**：`render_endpoints.rs`、`monitors.rs`、`events.rs`、
    `endpoint-match.js`、`route-state.js`、`audio-sources-builder.mjs`；
  - 其余 13 项内容不变（含 `Cargo.lock`、`tauri.conf.json`、`recovery.rs`、
    `finalize.rs`、`resample.rs`、`wgc.rs`、`region.js`、`settings-sample-rate.mjs`）。
  这与 `m16-report.md` §6 的 M16 产品/UI 变更清单一致。
- 任务清单外的同期改动（未计入 26 项指纹）：`crates/sr-enum/src/lib.rs`、
  `crates/sr-enum/Cargo.toml`、`crates/sr-core/tests/channel_snapshot_rebind.rs`、
  `specs/002-recording-modes.md`、`ui/index.html`、新增测试
  `tests/sr-itests/tests/{render_endpoints.rs,endpoint_audio.rs}` 等；
  本任务只打包，未改这些文件。`ui/index.html` 虽不在 26 项清单内，仍单独核验进包（§2）。
- `crates/sr-audio/tests/stall_pad_policy.rs`、`tools/ui/*.mjs` 与 `tests/sr-itests/*`
  不随安装包（bundle resources 仍仅 ffmpeg/ffprobe/licenses），按任务要求计入指纹。

打包资源（本轮未改）：

| 文件 | SHA-256 | 说明 |
|---|---|---|
| `src-tauri/resources/licenses/ffmpeg-source-offer.txt` | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` | M12-C 修正版：精确 commit + 四种获取方式；3,411 B |
| `src-tauri/resources/licenses/FFmpeg-LICENSE.txt` | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | 与构建包 LICENSE.txt 一致 |

## 2. 构建

工具链：Node v24.18.0 / npm 11.16.0；rustc 1.99.0 / cargo 1.99.0；`tauri-cli 2.12.3`（npx）。

```
npx --yes @tauri-apps/cli@2.12.3 build
→ Compiling sr-core v0.1.0
→ Compiling sr-audio v0.1.0
→ Compiling sr-enum v0.1.0
→ Compiling sr-capture v0.1.0
→ Compiling sr-pipeline v0.1.0
→ Compiling screen-rec v0.1.0
→ Finished `release` profile [optimized] target(s) in 1m 21s
→ Built application at: ...\target\release\screen-rec.exe
→ Info Patching ...\screen-rec.exe with bundle type information: nsis
→ Running makensis to produce ...\target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe
→ Finished 1 bundle at: ...\Screen_rec_0.1.0_x64-setup.exe (82.59 MiB)
→ TAURI_BUILD_EXIT=0
```

- 构建开始：2026-10-18 21:02:16；结束：**21:05:49**（213.1 s；`m16p-build-nsis.log`）。
- 本轮实际重编 `sr-core`/`sr-merge`/`sr-shot`/`sr-audio`/`sr-enum`/`sr-capture`/
  `sr-pipeline`/`screen-rec`（日志可见），M16 改动全部经过 release 重编。
- 第 1 次构建即成功：构建前 `screen-rec.exe` 进程数 0、`ffmpeg` 进程数 0，exe 未被锁定。

产物（覆盖旧包）：

| 项 | 旧包（M15c，被覆盖） | 新包（M16，本轮） |
|---|---|---|
| 路径 | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe` | 同左 |
| 构建时间（mtime） | 2026-10-18 17:08:36 | **2026-10-18 21:05:49** |
| 大小 | 86,539,974 B（82.53 MiB） | **86,597,996 B（82.59 MiB）** |
| SHA-256 | `24268AC0B61897EE0906C093422C910CDCA6B4C7C96B37B3395E1BFCCF1C3FC2` | **`879399CE2EF2DA9E3F3B1941C943B49E93639C27250098B9C3EA3DAE69134FD3`** |

- release 主程序：`target\release\screen-rec.exe`，15,006,208 B，mtime 21:05:49，
  SHA-256 `1D5A57AA07AD365ADD4562F348179AE5CF2B0FE441FB4CAE646CA32D4976A35A`
  （较 M15c 14,762,496 B 增加 243,712 B）。
- 二进制标记（release exe 实测，UTF-8 字节匹配）：
  - M16 新增：`render_endpoint 音源仅适用于显示器目标`=True、
    `显示器目标没有任何音源`=True、`纯录音不接受 render_endpoint 音源`=True、
    `音频采集端点失效`=True；
  - 继承 M15/M15c：`E_INVALID_AUDIO_PLAN`=True、`stop_notice`=True、
    `keep_segments`=True、`audio_sample_rate`=True、`SR_DIAG_AV`=True。
  - 注：命令名 `list_render_endpoints` 未以连续 ASCII 字面量出现在 release 字符串表
    （该区域与相邻命令名连续排布，详见 §6 #2），因此不以该字面量作进包判据；
    命令注册见 `src-tauri/src/lib.rs`，L2 `render_endpoints` 2 passed 见
    `m16-report.md` §4.2。
- **UI 进包证据**：本轮 Tauri 在 21:03:15 重新生成 `tauri-codegen-assets`
  （`target\release\build\screen-rec-ee450db19d850100\out\tauri-codegen-assets`）；
  解压后 SHA-256 与当前源码逐项相等：

  | 源码 | 资产文件（brotli） | 压缩 B | 解压 B | 解压 SHA-256 | 对照 |
  |---|---|---:|---:|---|---|
  | `ui/app.js` | `a0060ed2…e962.js` | 16,914 | 63,849 | `A6ADEAA1…A6412` | = 当前 `ui/app.js` |
  | `ui/api.js` | `7fed8c0b…f017.js` | 8,497 | 26,134 | `3159B97C…DBCE` | = 当前 `ui/api.js` |
  | `ui/endpoint-match.js` | `e8565f57…52a1.js` | 2,750 | 8,202 | `EF81FB3B…BC74` | = 当前 `ui/endpoint-match.js` |
  | `ui/route-state.js` | `23061bfb…d8db.js` | 1,479 | 4,202 | `D93A441A…9694` | = 当前 `ui/route-state.js` |
  | `ui/index.html` | `9bc926e8…176e.html` | 2,245 | 7,863 | `43C8C424…EBFA` | = 当前 `ui/index.html` |
  | `ui/region.js` | `a15c6368…8dd3b.js` | 2,246 | 5,769 | `AAA0CD9A…D9A4` | = 当前 `ui/region.js`（未变） |

  进一步核验：上述 5 个**更新**资产的 brotli 压缩字节（app/api/endpoint-match/
  route-state/index.html）在 `target\release\screen-rec.exe` 中**原样逐字节存在**
  （`indexOf` 命中），说明包内 UI 不是旧缓存。
- 新包比 M15c 大 58,022 B：release 主程序增加 243,712 B（M16 端点/失效传播/UI 逻辑），
  经 NSIS 压缩后净增约 58 KiB；安装清单完整（§3），不是缺文件。

完整日志：`docs/milestones/m16p-build-nsis.log`（UTF-8；`TAURI_BUILD_EXIT=0`）。

## 3. 静默安装 + 安装目录自检

```
安装：Screen_rec_0.1.0_x64-setup.exe /S /D=C:\Users\elvis\AppData\Local\Temp\sr_m16p_install
→ installer_exit=0；install_dir_exists=True（安装 2026-10-18 21:06:41–21:06:50）
```

安装目录清单（实测）：

| 文件 | 大小 | mtime | SHA-256 |
|---|---:|---|---|
| `screen-rec.exe` | 15,006,208 B | 2026-10-18 21:03:40 | `0B8CE4C5B36068CF91697ADA6D265386F18CEC8CD3953684F270C110CBCC9598` |
| `uninstall.exe` | 79,390 B | 2026-10-18 21:06:49 | `B8712ADAAC20514E4751D05CF4152537EA2E3BAA2B079256F386EBE0A4C6B387` |
| `ffmpeg\ffmpeg.exe` | 134,092,288 B | 2026-10-15 12:43:48 | `3B57117B384DF98F2B8D01876DF0A596A0B6EBF45F756B30F15D6F65AB8251C5` |
| `ffmpeg\ffprobe.exe` | 133,887,488 B | 2026-10-15 12:43:48 | `5B9724EE7A115D5A8458D41BD2E0CE6206847E2B76105E822061A4421322CDE9` |
| `licenses\FFmpeg-LICENSE.txt` | 7,651 B | 2026-10-15 12:43:48 | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` |
| `licenses\ffmpeg-source-offer.txt` | 3,411 B | 2026-10-17 20:19:36 | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` |

- 安装副本 `screen-rec.exe` 与 release exe 大小相同（15,006,208 B），哈希不同
  （`0B8CE4…` vs `1D5A57…`）：Tauri 向 release 主程序写入 bundle 类型信息后再打包
  （M12-G/M14/M15/M15c 同现象）。
- 安装副本 `screen-rec.exe` 二进制标记实测：4 条 M16 错误串全部 True，
  `E_INVALID_AUDIO_PLAN`/`stop_notice`/`keep_segments`/`audio_sample_rate`/
  `SR_DIAG_AV` 全部 True（`m16p-self-check.log`）。
- ffmpeg/ffprobe/licenses 哈希与锁定构建包逐一相同；source offer 未变。

`--self-check`（安装目录运行；完整输出 `docs/milestones/m16p-self-check.log`，
2026-10-18 21:07:04）：

```
[self-check] 搜索根（按优先级）
  env  SCREENREC_FFMPEG_DIR: (未设置)
  tauri resource_dir: (未设置)
  exe 同目录: C:\Users\elvis\AppData\Local\Temp\sr_m16p_install
  开发回退: ...\Screen_rec\crates\sr-core\../../tools/ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin
[self-check] ffmpeg: ...\sr_m16p_install\ffmpeg\ffmpeg.exe（来源：exe-adjacent）
[self-check]   -version: ffmpeg version n8.1.3-18-g46d899d7a0-20261015 Copyright (c) 2000-2026 the FFmpeg developers
[self-check] ffprobe: ...\sr_m16p_install\ffmpeg\ffprobe.exe（来源：exe-adjacent）
[self-check]   -version: ffprobe version n8.1.3-18-g46d899d7a0-20261015 Copyright (c) 2007-2026 the FFmpeg developers
SELF_CHECK_EXIT=0
```

- `screen-rec.exe` 为 GUI 子系统：首轮用 PowerShell `&` 直接调用不等待且无退出码，
  随后改用 `Start-Process -Wait -RedirectStandardOutput/-RedirectStandardError`
  取得完整输出与 `SELF_CHECK_EXIT=0`（同一次安装内，非重复安装）。

卸载与产物复核：

```
uninstall.exe /S → uninstall_exit=0；dir_exists_after_uninstall=False
安装包 SHA-256（卸载后）→ 879399CE2EF2DA9E3F3B1941C943B49E93639C27250098B9C3EA3DAE69134FD3（与构建时一致）
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
| `docs/milestones/m16-package-report.md` | 本报告（新增） |
| `docs/milestones/m16p-build-nsis.log` | NSIS 构建完整日志（`TAURI_BUILD_EXIT=0`，新增） |
| `docs/milestones/m16p-fingerprint.log` | 26 项源码指纹三次记录与产物哈希（新增） |
| `docs/milestones/m16p-self-check.log` | 安装清单/哈希、安装目录 `--self-check` 原始输出、卸载与产物复核（新增） |
| `docs/milestones/m15c-package-report.md` | 顶部加一行指向本报告（其 17:08 旧包已被覆盖） |

未改任何源码/测试/`specs/`；§1 三次指纹一致可证。

> 说明：`docs/milestones/*.log` 被 `.gitignore` 的 `*.log` 忽略（与 M15/M15c 报告相同），
> 日志已落盘但默认不进入 git；如需提交日志，请编排者显式 `git add -f`。

## 6. 未决 / 风险（回交编排者）

1. **任务文字“21 项”与逐项枚举（26 项）不一致**：本报告按枚举把 26 项全部纳入指纹
   （含新增的 `render_endpoints.rs`/`monitors.rs`/`events.rs`/`endpoint-match.js`/
   `route-state.js`/`audio-sources-builder.mjs`）。若编排者的 21 项另有所指，请指正；
   当前 26 项是任务枚举的超集，三次哈希一致。
2. **`list_render_endpoints` 命令名字面量未在 release 字符串表连续出现**：该区域与相邻
   命令名（`render_endpoints`、`get_endpoint_mix_rate` 等）连续排布，完整名字未形成可
   匹配的连续字面量（可能与编译器字符串合并/优化有关，未能进一步确证）；
   `generate_handler!` 注册见 `src-tauri/src/lib.rs`，
   `ui/api.js` 的 `invoke('list_render_endpoints')` 与 L2 2 passed 见 `m16-report.md`。
   因此进包判据采用 4 条 M16 错误串 + UI 资产 + 重编日志，而非该字面量。
3. **包内 M16 行为未做运行时功能复验**：本任务只做“构建 + 安装 + `--self-check`”；
   端点 FFT/失效传播/逐屏音轨/UI runner 的证据在 `m16-report.md` §4（L1 57 二进制
   121 passed、L2 端点套件除麦克风组合 BLOCKED 外通过、UI runner 全通过），
   未在 *release 安装包* 上重跑。若验收要求安装包级证据，建议在安装目录构造端点/失效
   场景复验（本机渲染端点 3 个、采集设备 0 个）。
4. **未签名**：NSIS 安装包仍未签名（同 M11-C/M12-G/M14/M15/M15c），生产分发需证书与签名步骤。
5. **`resource_dir` 分支未做 GUI 上下文端到端**：`--self-check` 先于 Tauri 启动；本次
   安装实测命中 `exe-adjacent`。既有单测覆盖优先级，GUI 注册见 `src-tauri/src/lib.rs`。
6. **未在干净机安装**：本机已有 WebView2；安装器对 WebView2 前置的引导未在无运行时
   机器审计（同前序报告）。
7. **BtbN 构建脚本未按 commit 锁定**：FFmpeg 源码已精确到 commit；构建脚本仅锁定仓库 +
   构建日期，二进制由构建包 SHA-256 固定。
8. **长测未运行**：按约束未跑电池/sync/soak；本报告不据此宣称长稳。
9. **安装后 exe 哈希与 release exe 不同**：Tauri 写入 bundle 类型信息后再打包；
   安装副本与构建目录副本因此不必逐字节相同（同 M15c #7）。
10. **M16 测试/规格改动未计入 26 项指纹**：任务清单未要求；`specs/002-recording-modes.md`、
    `tests/sr-itests/tests/{render_endpoints.rs,endpoint_audio.rs}`、
    `crates/sr-enum/src/lib.rs`、`Cargo.toml`、`ui/index.html` 等同期改动仅作参考，
    本包不包含测试文件（bundle resources 不变）。

## 7. 证据索引（本机实际）

| 证据 | 内容 |
|---|---|
| `docs/milestones/m16p-build-nsis.log` | `npx @tauri-apps/cli@2.12.3 build` 全量输出；21:02:16–21:05:49，exit 0，82.59 MiB |
| `docs/milestones/m16p-fingerprint.log` | 26 项 SHA-256 构建前 21:02:13、构建后 21:05:54、安装后 21:07:28；`DIFF=0`；产物哈希 |
| `docs/milestones/m16p-self-check.log` | 安装清单/哈希、安装 exe 标记、self-check 输出（`SELF_CHECK_EXIT=0`，21:07:04）、卸载与产物复核 |
| 源码指纹 | §1（`Get-FileHash <file> -Algorithm SHA256`） |
| UI 进包 | §2（`tauri-codegen-assets` 21:03:15；解压 SHA-256 = 当前源码；压缩字节在 release exe 中原样存在） |
| 安装/卸载 | §3（`installer_exit=0`、`uninstall_exit=0`、目录已删除、产物哈希复核一致） |
| M16 行为证据 | `docs/milestones/m16-report.md` §4（L1、L2 端点/回归、UI runner）与 `m16-l2-*.log`、`m16-ui-*.log` |
| M15/M15c 行为证据 | `docs/milestones/m15-report.md`、`m15c-package-report.md` |
