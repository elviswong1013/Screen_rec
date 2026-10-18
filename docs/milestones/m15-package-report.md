# M15 安装包重建（NSIS · 指纹 · 安装/自检）

> **后续更新（M15c/acc-7，2026-10-18 17:08）**：M15 之后源码新增 acc-7 阻断修复
> （`finalize.rs` 成品校验 + 恢复清理前复核 + 回归/规格状态），已从当前源码再次重建；
> **本报告的 16:42 安装包已被覆盖**。最新产物、指纹与安装/自检结果见
> `docs/milestones/m15c-package-report.md`。

- 日期：2026-10-18
- 范围：M14 包（2026-10-18 15:02，SHA-256 `0AEFC206…F5916`）之后源码新增
  **M15**（采样率可选 P1，REQ-1..8）与 **M14c**（恢复收尾纳入 `keep_segments` 清理）
  ——证据 `docs/milestones/m15-report.md`（含 §7 M15b）与
  `docs/milestones/m14-report.md` §8 → 从当前最终源码重建 NSIS 并复验安装/自检。
- 流程：沿用 `docs/milestones/m11-c-report.md` / `m12-g-report.md` /
  `m14-package-report.md`（`npx --yes @tauri-apps/cli@2.12.3 build`，resources 映射
  ffmpeg/ffprobe/licenses）。
- 约束遵守：只做打包与报告；未改源码/测试/`specs/`/`docs/spikes/`；未执行任何 `git`
  写操作（仅只读 `git status`/`git diff`）；**未跑长测/未跑测试**（电池/sync/soak 与 L1/L2 均由
  编排者按既有结果使用）。

## 0. 结论

| 项 | 状态 | 关键证据 |
|---|---|---|
| 源码指纹 | **已记录（构建前/构建后/安装后三次一致）** | §1：任务清单共 19 项 SHA-256；16:38:54 记录，16:42:37 与 16:43:59 两次重算全部相同 |
| NSIS 重建 | **完成，已覆盖 M14 15:02 旧包** | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe`；2026-10-18 16:42:31；86,555,459 B（82.55 MiB）；SHA-256 `E500CAD8…87B0D` |
| M15/M14c 代码进包 | **已验证** | 两次尝试合计重编 `sr-audio`/`sr-pipeline`/`screen-rec`（第 2 次仅 `screen-rec`）；exe 含 `audio_sample_rate`/`get_endpoint_mix_rate`/`keep_segments`；本次构建生成的 UI 资产（brotli 解压）SHA-256 分别等于当前 `ui/app.js` 与 `ui/api.js`，且含四档设置与查询接线 |
| 静默安装 + 自检 | **exit 0** | `%TEMP%\sr_m15p_install`；ffmpeg/ffprobe 来源 `exe-adjacent`；版本 `n8.1.3-18-g46d899d7a0-20261015` |
| 卸载清理 | **完成** | `uninstall.exe /S` exit 0；安装目录已删除 |
| 产物哈希复核 | **一致** | 卸载后安装包 SHA-256 仍为 `E500CAD8…87B0D` |
| ffmpeg source offer | **保持精确 commit** | 未改动；SHA-256 `FF1A3F3E…55808`；安装目录内同名文件哈希一致；commit `46d899d7a02bc63237472cc4897258d5ddf274ac` |
| 长测 | **未运行（按约束）** | 仅打包/安装/自检 |

**本报告描述的 16:42 安装包取代 M14（15:02）安装包**；`m14-package-report.md` 顶部已加指向本报告的说明。

## 1. 源码指纹（构建前记录，构建后/安装后复核一致）

记录时间：2026-10-18 16:38:54（本机）。最晚源码 mtime = **2026-10-18 16:31:01**
（`crates/sr-pipeline/src/recovery.rs`，M14c）；构建（第 2 次尝试）开始 16:40:12，
新包 mtime = **2026-10-18 16:42:31**。清单共 **19 项**（较 M14 指纹新增
`crates/sr-pipeline/src/recovery.rs`、`crates/sr-pipeline/src/sidecar.rs`、
`tools/ui/settings-sample-rate.mjs`）。

| 文件 | SHA-256 | mtime |
|---|---|---|
| `Cargo.lock` | `5268A66731DC03F07653A0A3072B0E1704055742BCAB35724B3617BC66966AAF` | 2026-10-17 19:32:45 |
| `src-tauri/tauri.conf.json` | `95080F11734FA35F3BF28B3B51B01A4D8EA28A1AF742B4FD306FDDE040F819FC` | 2026-10-18 14:40:47 |
| `src-tauri/src/region.rs` | `A9A04F7C82E5E54894BA3ABAB0092BAA619D954DE741A6DE296C5D4310B96D24` | 2026-10-17 21:01:09 |
| `src-tauri/src/lib.rs` | `B8108CA2A50DD5533846EDE9E28F7DDD535CD46ABDFE506AC96B1FB06334B7C7` | 2026-10-18 15:37:16 |
| `src-tauri/src/commands.rs` | `311834D40BB84BFE8B222ABBFAFEF9C3D310065F5339957C425036BD3103D5ED` | 2026-10-18 16:29:42 |
| `src-tauri/src/settings.rs` | `92018FD53541187DAA6A31B730305231B329C0CFA20A05ACDB5D35E7E18741C0` | 2026-10-18 15:37:16 |
| `crates/sr-pipeline/src/route.rs` | `DE181A8D6EECF609DADA8FE208FC9BB8E47D6A1D36F54127A9AD1406E2468E74` | 2026-10-18 15:37:16 |
| `crates/sr-pipeline/src/naming.rs` | `5BCA89EE4D00C28AF50CB941D276DDD22ADB178B95C082A6C58D85D3C243EB4B` | 2026-10-17 21:00:19 |
| `crates/sr-pipeline/src/recovery.rs` | `AED56B486D7492C810D5A2830A99E9D3A1C9CA14A16588D2A52A66659AC1439F` | 2026-10-18 16:31:01 |
| `crates/sr-pipeline/src/sidecar.rs` | `44FBDE79404689F6319FBCF1CAEEE95EC56AEC9DC393CF06F630ED3C5F00832B` | 2026-10-18 15:36:42 |
| `crates/sr-audio/src/resample.rs` | `66711E62512F189968A39ABBD40765AB6D7A9CA1C81EDD16E17CA49E5B235041` | 2026-10-18 15:31:02 |
| `crates/sr-audio/src/wasapi.rs` | `6C26D98846814205A0F9D23B260FD50DBD690D7BFA84CED52A466FC61F7B2DFC` | 2026-10-18 15:37:16 |
| `crates/sr-audio/src/slots.rs` | `44DB9DB8D0985F78196B0550451C1DD89E97667C5247F70DF5445CBBCEFCAC98` | 2026-10-17 23:42:55 |
| `crates/sr-capture/src/wgc.rs` | `90B244810A5BED028E5233CC12B2E3C7A6504C6D543A5BD6ECF1BCC83BA12928` | 2026-10-17 22:28:49 |
| `ui/region.js` | `AAA0CD9AD104E521BDBD2B5DDACB714DB5AB25ED481BE951B739A2D40001D9A4` | 2026-10-17 21:01:13 |
| `ui/app.js` | `E6EACD03C79C94F7A558F6EC610C2284E0D33DDDC18DE4D61E8DEDB3E9C1D0C1` | 2026-10-18 15:35:40 |
| `ui/api.js` | `16E0CA19466B156CBDF15BB43B333463122154185FF83C3A5892383F0FF5C021` | 2026-10-18 15:35:22 |
| `crates/sr-audio/tests/stall_pad_policy.rs` | `60553CAE593D3B80E4074A32647EFF70B6CC90D9C667F5DD7449DDE9F3CE0733` | 2026-10-17 23:42:13 |
| `tools/ui/settings-sample-rate.mjs` | `1CC1AF04D9F7233C55D45C9F0129ED785557F0E0920DFA6B2BD7FDD5CCCEE186` | 2026-10-18 16:11:49 |

说明：

- 构建后（16:42:37）与安装/卸载后（16:43:59）对上述 19 项重算 SHA-256，**逐项与
  构建前相同**（`m15p-fingerprint.log`：`FINGERPRINT_DIFF_COUNT=0`、`FINAL_DIFF_COUNT=0`）。
- 相对 M14 指纹，内容变化：`lib.rs`（`DEA816D9…` → `B8108CA2…`）、`commands.rs`
  （`3006E2AE…` → `311834D4…`）、`settings.rs`（`C6A043AC…` → `92018FD5…`）、
  `route.rs`（`BD56D18E…` → `DE181A8D…`）、`resample.rs`（`4D39004B…` → `66711E62…`）、
  `wasapi.rs`（`89E9593B…` → `6C26D988…`）、`ui/app.js`（`7E7B7EC3…` → `E6EACD03…`）、
  `ui/api.js`（`584B845C…` → `16E0CA19…`）；新增 `recovery.rs`、`sidecar.rs`、
  `tools/ui/settings-sample-rate.mjs`。`tauri.conf.json` 内容未变（`95080F…`）。
- 本轮源码增量：M15 采样率可选（settings/query/resample/wasapi/sidecar/route/commands/
  ui + 测试与 UI runner）见 `m15-report.md`；M14c 恢复收尾接入 `keep_segments` 清理见
  `m14-report.md` §8（`finalize_session_with_options`、`cleanup_session_after_finalize`、
  `commands::finalize_session` 透传）。
- `crates/sr-audio/tests/stall_pad_policy.rs` 与 `tools/ui/settings-sample-rate.mjs`
  不随安装包（bundle resources 仍仅 ffmpeg/ffprobe/licenses），但按任务要求计入指纹。

打包资源（本轮未改）：

| 文件 | SHA-256 | 说明 |
|---|---|---|
| `src-tauri/resources/licenses/ffmpeg-source-offer.txt` | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` | M12-C 修正版：精确 commit + 四种获取方式；3411 B |
| `src-tauri/resources/licenses/FFmpeg-LICENSE.txt` | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | 与构建包 LICENSE.txt 一致 |

## 2. 构建

工具链：Node v24.18.0 / npm 11.16.0；rustc 1.99.0 / cargo 1.99.0；`tauri-cli 2.12.3`（npx）。

第 1 次尝试（16:39:01–16:39:28）在链接/替换 exe 阶段失败：

```
error: failed to remove file `...\target\release\screen-rec.exe`
Caused by: 存取被拒。 (os error 5)
TAURI_BUILD_EXIT=1
```

原因：一个 15:58:24 启动的 `target\release\screen-rec.exe`（pid 123840，父进程已退出、
无 ffmpeg 子进程、录像已于 15:58:48 封口）仍驻留并锁定 exe。确认其空闲后关闭该进程，
第 2 次尝试成功：

```
npx --yes @tauri-apps/cli@2.12.3 build
→ Compiling screen-rec v0.1.0 (...src-tauri)
→ Finished `release` profile [optimized] target(s) in 21.50s
→ Built application at: ...\target\release\screen-rec.exe
→ Info Patching ...\screen-rec.exe with bundle type information: nsis
→ Running makensis to produce ...\target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe
→ Finished 1 bundle at: ...\Screen_rec_0.1.0_x64-setup.exe (82.55 MiB)
→ TAURI_BUILD_EXIT=0
```

- 第 2 次构建开始：2026-10-18 16:40:12；结束：**16:42:31**（`m15p-build-nsis.log`）。
- 第 1 次尝试已编译 `sr-audio`/`sr-pipeline`/`screen-rec`；源码在两次尝试之间未变
  （§1 三次指纹一致），第 2 次仅重编 `screen-rec` 并链接。

产物（覆盖旧包）：

| 项 | 旧包（M14，被覆盖） | 新包（M15，本轮） |
|---|---|---|
| 路径 | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe` | 同左 |
| 构建时间（mtime） | 2026-10-18 15:02:01 | **2026-10-18 16:42:31** |
| 大小 | 86,520,827 B（82.51 MiB） | **86,555,459 B（82.55 MiB）** |
| SHA-256 | `0AEFC206E8D28808FF443AA7A0A2DA0F8AC8831165142D8AB031166FC32F5916` | **`E500CAD85475F394ABCCE60E3B05E0ACAC2A451A4D46277EAC2D311B62E87B0D`** |

- release 主程序：`target\release\screen-rec.exe`，14,763,520 B，mtime 16:42:31，
  SHA-256 `23782417987A3E76CCEEC774528537CFC80309103BA42745864DE2BFC838638D`。
- 二进制标记（release exe 实测存在）：`SR_DIAG_AV`（门控诊断仍编入）、
  `take_region_screenshot`、`MoveFileExW`、`keep_segments`（M14/M14c）、
  `audio_sample_rate`、`get_endpoint_mix_rate`（M15）。
- **UI 进包证据**：第 1 次尝试（源码未变）于 16:39:07 生成 `tauri-codegen-assets`，第 2 次构建复用：
  `b2306b97…8a02.js`（压缩 14,977 B；brotli 解压 50,250 字符）解压内容 SHA-256 =
  `E6EACD03…D0C1`，**等于当前 `ui/app.js` 指纹**，含 `audio_sample_rate`、`set-sr`、
  `getEndpointMixRate`、`keep_segments`、「升采样」「保留原始分段」；
  `90679536…256d.js`（压缩 7,656 B；解压 20,541 字符）解压内容 SHA-256 =
  `16E0CA19…C021`，**等于当前 `ui/api.js` 指纹**，含 `audio_sample_rate`、
  `getEndpointMixRate`、`keep_segments`。证明 16:42 包内 UI 为 M15 当前源码，
  不是 M14 旧资产。

完整日志：`docs/milestones/m15p-build-nsis.log`。

## 3. 静默安装 + 安装目录自检

```
安装：Screen_rec_0.1.0_x64-setup.exe /S /D=C:\Users\elvis\AppData\Local\Temp\sr_m15p_install
→ installer_exit=0；install_dir_exists=True（2026-10-18 16:42:54）
```

安装目录清单（实测）：

| 文件 | 大小 | mtime |
|---|---|---|
| `screen-rec.exe` | 14,763,520 B | 2026-10-18 16:40:36 |
| `uninstall.exe` | 79,390 B | 2026-10-18 16:43:01 |
| `ffmpeg\ffmpeg.exe` | 134,092,288 B | 2026-10-15 12:43:48 |
| `ffmpeg\ffprobe.exe` | 133,887,488 B | 2026-10-15 12:43:48 |
| `licenses\FFmpeg-LICENSE.txt` | 7,651 B | 2026-10-15 12:43:48 |
| `licenses\ffmpeg-source-offer.txt` | 3,411 B | 2026-10-17 20:19:36 |

安装后哈希：

| 文件 | SHA-256 | 对照 |
|---|---|---|
| 安装 `screen-rec.exe` | `3A8D90F9C978D4A405FAFC6717AD5A176BF09313083CB67699CE4D7BD0146C45` | 与 release exe `23782417…` 不同（Tauri 打包会改写 exe；M12-G/M14 同现象）。安装副本大小 14,763,520 B = release，且不同于 M14 的 14,721,024 B |
| 安装 `ffmpeg\ffmpeg.exe` | `3B57117B384DF98F2B8D01876DF0A596A0B6EBF45F756B30F15D6F65AB8251C5` | = 锁定构建包 |
| 安装 `ffmpeg\ffprobe.exe` | `5B9724EE7A115D5A8458D41BD2E0CE6206847E2B76105E822061A4421322CDE9` | = 锁定构建包 |
| 安装 `licenses\FFmpeg-LICENSE.txt` | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | = 构建包 LICENSE.txt |
| 安装 `licenses\ffmpeg-source-offer.txt` | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` | = 源文件（精确 commit 说明随包） |

安装副本 `screen-rec.exe` 二进制标记实测：`audio_sample_rate=True`、
`get_endpoint_mix_rate=True`、`keep_segments=True`、`SR_DIAG_AV=True`。

`--self-check`（安装目录运行；完整输出 `docs/milestones/m15p-self-check.log`，
2026-10-18 16:43:29–16:43:30）：

```
[self-check] 搜索根（按优先级）
  env  SCREENREC_FFMPEG_DIR: (未设置)
  tauri resource_dir: (未设置)
  exe 同目录: C:\Users\elvis\AppData\Local\Temp\sr_m15p_install
  开发回退: ...\Screen_rec\crates\sr-core\../../tools/ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin
[self-check] ffmpeg: ...\sr_m15p_install\ffmpeg\ffmpeg.exe（来源：exe-adjacent）
[self-check]   -version: ffmpeg version n8.1.3-18-g46d899d7a0-20261015 Copyright (c) 2000-2026 the FFmpeg developers
[self-check] ffprobe: ...\sr_m15p_install\ffmpeg\ffprobe.exe（来源：exe-adjacent）
[self-check]   -version: ffprobe version n8.1.3-18-g46d899d7a0-20261015 Copyright (c) 2007-2026 the FFmpeg developers
SELF_CHECK_EXIT=0
```

卸载与产物复核：

```
uninstall.exe /S → uninstall_exit=0；dir_exists_after_uninstall=False
安装包 SHA-256（卸载后）→ E500CAD85475F394ABCCE60E3B05E0ACAC2A451A4D46277EAC2D311B62E87B0D（与构建时一致）
```

## 4. ffmpeg source offer（保持 M12-C 精确说明）

- 文件未改动：SHA-256 `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808`，3411 B。
- 精确版本：`FFmpeg n8.1.3-18-g46d899d7a0` = tag `n8.1.3` + 18 commits，commit
  `46d899d7a02bc63237472cc4897258d5ddf274ac`（文件第 7/29/34/49 行均保留）。
- 四种获取方式保留：git checkout / GitHub commit 归档 / 上游 tag 对照（注明少 18 commits）/
  BtbN 构建脚本与日期。
- 随包验证：安装目录 `licenses/ffmpeg-source-offer.txt` 哈希 = 源文件哈希（§3）；
  `--self-check` 打印的 ffmpeg/ffprobe 版本与该说明一致。

## 5. 变更文件清单

| 文件 | 变更 |
|---|---|
| `docs/milestones/m15-package-report.md` | 本报告（新增） |
| `docs/milestones/m15p-build-nsis.log` | NSIS 构建完整日志（含第 1 次失败与第 2 次 `TAURI_BUILD_EXIT=0`，新增） |
| `docs/milestones/m15p-fingerprint.log` | 19 项源码指纹三次记录与产物哈希（新增） |
| `docs/milestones/m15p-self-check.log` | 安装清单/哈希、安装目录 `--self-check` 原始输出、卸载与产物复核（新增） |
| `docs/milestones/m14-package-report.md` | 顶部加一行指向本报告（其 15:02 旧包已被覆盖） |

未改任何源码/测试/`specs/`/`docs/spikes/`；§1 三次指纹一致可证。

> 说明：`docs/milestones/*.log` 被 `.gitignore` 的 `*.log` 忽略（与 M15 报告 §6.6 相同），
> 日志已落盘但默认不进入 git；如需提交日志，请编排者显式 `git add -f`。

## 6. 未决 / 风险（回交编排者）

1. **包内 M15/M14c 行为未做运行时功能复验**：本任务只做“构建 + 安装 + `--self-check`”；
   采样率实际生效（44.1k/88.2k/96k 录制与 WAV）、设置页交互、恢复后默认清理/
   `keep_segments` 的语义证据在 `m15-report.md`（L1/L2/UI runner）与 `m14-report.md`
   §8（L1/L2），未在 *release 安装包* 上重跑录制。若验收要求安装包级证据，建议在安装
   目录补一次短录制（约 15s，默认清理与 `keep_segments=true` 各一次）。
2. **未签名**：NSIS 安装包仍未签名（同 M11-C/M12-G/M14），生产分发需证书与签名步骤。
3. **`resource_dir` 分支未做 GUI 上下文端到端**：`--self-check` 先于 Tauri 启动；本次
   安装实测命中 `exe-adjacent`。既有单测覆盖优先级，GUI 注册见 `src-tauri/src/lib.rs`。
4. **未在干净机安装**：本机已有 WebView2；安装器对 WebView2 前置的引导未在无运行时
   机器审计（同 M11-C #4 / M12-G #4 / M14 #4）。
5. **BtbN 构建脚本未按 commit 锁定**：FFmpeg 源码已精确到 commit；构建脚本仅锁定仓库 +
   构建日期，二进制由构建包 SHA-256 固定（同 M14 #5）。
6. **长测未运行**：按约束未跑电池/sync/soak；本报告不据此宣称长稳。
7. **安装后 exe 哈希与 release exe 不同**：构建日志显示 Tauri 会向 release 主程序写入
   bundle 类型信息（“Patching … with bundle type information: nsis”）后再打包；安装副本
   与构建目录副本因此不必逐字节相同。M12-G/M14 亦有同样哈希差异，不是构建污染；安装包
   哈希与自检结果为准。
8. **构建环境中的孤儿进程（环境注记，非产品缺陷）**：第 1 次构建失败源于一个 15:58:24
   启动、父进程已退出且空闲的 `target\release\screen-rec.exe`（pid 123840）锁定 exe；
   确认其无 ffmpeg 子进程、录像已于 15:58:48 封口后强制结束。其遗留的
   `%USERPROFILE%\Videos\Screen_rec\.screenrec_tmp\1792310265895_0`（M14c/L3 恢复验证
   残留）未清理、未改动。若该进程属于另一条线的现场，请编排者知悉。
9. **`m15p-build-nsis.log` 中第 1 次失败的中文错误行受控制台代码页影响显示为乱码**
   （`存取被拒`），退出码与英文错误行完整；成功构建段落未受影响。

## 7. 证据索引（本机实际）

| 证据 | 内容 |
|---|---|
| `docs/milestones/m15p-build-nsis.log` | `npx @tauri-apps/cli@2.12.3 build` 两次尝试全量输出；第 2 次 16:42:31，exit 0，82.55 MiB |
| `docs/milestones/m15p-fingerprint.log` | 19 项 SHA-256 构建前 16:38:54、构建后 16:42:37、安装后 16:43:59；`DIFF=0`；产物哈希 |
| `docs/milestones/m15p-self-check.log` | 安装清单/哈希、self-check 输出（`SELF_CHECK_EXIT=0`，16:43:29）、卸载与产物复核 |
| 源码指纹 | §1（`Get-FileHash <file> -Algorithm SHA256`） |
| UI 进包 | §2（`tauri-codegen-assets` 16:39:07；解压 SHA-256 = 当前 `ui/app.js`/`ui/api.js`） |
| 安装/卸载 | §3（`installer_exit=0`、`uninstall_exit=0`、目录已删除、产物哈希复核一致） |
| M15 行为证据 | `docs/milestones/m15-report.md`（REQ-1..8；§7 M15b） |
| M14c 行为证据 | `docs/milestones/m14-report.md` §8 |
