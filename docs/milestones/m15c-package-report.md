# M15c 安装包重建（NSIS · 指纹 · 安装/自检）

> **后续更新（M16，2026-10-18 21:05）**：M15c 之后源码新增 M16（P2 全屏音轨，
> REQ-9..15）并已从当前源码再次重建；**本报告的 17:08 安装包已被覆盖**。最新产物、
> 指纹与安装/自检结果见 `docs/milestones/m16-package-report.md`。

- 日期：2026-10-18
- 范围：M15 包（2026-10-18 16:42，SHA-256 `E500CAD8…87B0D`）之后源码新增
  **M15c/acc-7**——恢复清理前复核成品（`crates/sr-pipeline/src/finalize.rs` 新增
  `verify_final_media`；`recovery.rs` 幂等分支复用前复核并拒绝清理损坏成品）、
  对应单元/L2 回归（`tests/sr-itests/tests/{recovery.rs,sync_dual.rs}`）与
  `specs/002-recording-modes.md` 状态字段（`Draft` → `Approved`）——证据
  `docs/milestones/m14-report.md` §9、`docs/milestones/m15-report.md` §8。
  从当前最终源码重建 NSIS 并复验安装/自检。
- 流程：沿用 `docs/milestones/m11-c-report.md` / `m12-g-report.md` /
  `m14-package-report.md` / `m15-package-report.md`（`npx --yes @tauri-apps/cli@2.12.3 build`，
  resources 映射 ffmpeg/ffprobe/licenses）。
- 约束遵守：只做打包与报告；未改源码/测试/`specs/`；未执行任何 `git` 写操作；
  **未跑长测/未跑测试**（电池/sync/soak 与 L1/L2 由编排者按既有结果使用）。
  构建前检查无 `screen-rec.exe` 进程驻留，未出现 exe 锁定失败。

## 0. 结论

| 项 | 状态 | 关键证据 |
|---|---|---|
| 源码指纹 | **已记录（构建前/构建后/安装后三次一致）** | §1：任务清单共 20 项 SHA-256；17:05:58 记录，17:08:43 与 17:09:43 两次重算全部相同（`FINGERPRINT_DIFF_COUNT=0`、`FINAL_DIFF_COUNT=0`） |
| NSIS 重建 | **完成，已覆盖 M15 16:42 旧包** | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe`；2026-10-18 17:08:36；86,539,974 B（82.53 MiB）；SHA-256 `24268AC0…3FC2` |
| M15c/acc-7 代码进包 | **已验证** | 本轮重编 `sr-pipeline`/`screen-rec`；release exe 含 `成品校验失败`、`清单已记成品但校验失败`、`原段保留、拒绝清理`；UI 资产解压 SHA-256 仍等于当前 `ui/app.js`/`ui/api.js` |
| 静默安装 + 自检 | **exit 0** | `%TEMP%\sr_m15cp_install`；ffmpeg/ffprobe 来源 `exe-adjacent`；版本 `n8.1.3-18-g46d899d7a0-20261015` |
| 卸载清理 | **完成** | `uninstall.exe /S` exit 0；安装目录已删除 |
| 产物哈希复核 | **一致** | 卸载后安装包 SHA-256 仍为 `24268AC0…3FC2` |
| ffmpeg source offer | **保持精确 commit** | 未改动；SHA-256 `FF1A3F3E…55808`；安装目录内同名文件哈希一致；commit `46d899d7a02bc63237472cc4897258d5ddf274ac` |
| 长测 | **未运行（按约束）** | 仅打包/安装/自检 |

**本报告描述的 17:08 安装包取代 M15（16:42）安装包**；`m15-package-report.md` 顶部已加指向本报告的说明。

## 1. 源码指纹（构建前记录，构建后/安装后复核一致）

记录时间：2026-10-18 17:05:58（本机）。最晚源码 mtime = **2026-10-18 16:51:48**
（`crates/sr-pipeline/src/recovery.rs` 与 `finalize.rs`，M15c/acc-7）；构建开始 17:06:04，
新包 mtime = **2026-10-18 17:08:36**。清单共 **20 项**（较 M15 指纹新增
`crates/sr-pipeline/src/finalize.rs`）。

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
| `crates/sr-pipeline/src/recovery.rs` | `A9BBC09167C2DD9066F4BD570BF807EB5949175F2224334309B12F8B1D5142D8` | 2026-10-18 16:51:48 |
| `crates/sr-pipeline/src/sidecar.rs` | `44FBDE79404689F6319FBCF1CAEEE95EC56AEC9DC393CF06F630ED3C5F00832B` | 2026-10-18 15:36:42 |
| `crates/sr-pipeline/src/finalize.rs` | `FF1809833AEAB57CEC4C620FAA424F610BB61B530FE2C6E45777561BCBCDE8BA` | 2026-10-18 16:51:48 |
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

- 构建后（17:08:43）与安装/卸载后（17:09:43）对上述 20 项重算 SHA-256，**逐项与
  构建前相同**（`m15cp-fingerprint.log`：`FINGERPRINT_DIFF_COUNT=0`、`FINAL_DIFF_COUNT=0`）。
- 相对 M15 指纹：`recovery.rs` 内容变化（`AED56B48…` → `A9BBC091…`）；新增
  `finalize.rs`（`FF180983…`）；其余 18 项内容不变。`tauri.conf.json` 未变。
  这证明 M15c 源码增量仅为 `finalize.rs`/`recovery.rs` 的成品复核修复（及清单外的
  测试/规格文本）。
- 任务清单外的同期改动（未计入 20 项指纹）：`tests/sr-itests/tests/recovery.rs`、
  `tests/sr-itests/tests/sync_dual.rs`、`specs/002-recording-modes.md`（状态字段）等；
  本任务只打包，未改这些文件。
- `crates/sr-audio/tests/stall_pad_policy.rs` 与 `tools/ui/settings-sample-rate.mjs`
  不随安装包（bundle resources 仍仅 ffmpeg/ffprobe/licenses），但按任务要求计入指纹。

打包资源（本轮未改）：

| 文件 | SHA-256 | 说明 |
|---|---|---|
| `src-tauri/resources/licenses/ffmpeg-source-offer.txt` | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` | M12-C 修正版：精确 commit + 四种获取方式；3411 B |
| `src-tauri/resources/licenses/FFmpeg-LICENSE.txt` | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | 与构建包 LICENSE.txt 一致 |

## 2. 构建

工具链：Node v24.18.0 / npm 11.16.0；rustc 1.99.0 / cargo 1.99.0；`tauri-cli 2.12.3`（npx）。

```
npx --yes @tauri-apps/cli@2.12.3 build
→ Compiling sr-pipeline v0.1.0 (...\crates\sr-pipeline)
→ Compiling screen-rec v0.1.0 (...\src-tauri)
→ Finished `release` profile [optimized] target(s) in 22.19s
→ Built application at: ...\target\release\screen-rec.exe
→ Info Patching ...\screen-rec.exe with bundle type information: nsis
→ Running makensis to produce ...\target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe
→ Finished 1 bundle at: ...\Screen_rec_0.1.0_x64-setup.exe (82.53 MiB)
→ TAURI_BUILD_EXIT=0
```

- 构建开始：2026-10-18 17:06:04；结束：**17:08:36**（151.9 s；`m15cp-build-nsis.log`）。
- 仅重编 `sr-pipeline`/`screen-rec`：`sr-audio` 源文件与 M15 构建时指纹相同（§1），
  且 M15 包的 `keep_segments`/采样率功能已在 16:42 包验证；本轮 `sr-pipeline` 重编即包含
  `finalize.rs`/`recovery.rs` 的 M15c 修复。
- 第 1 次构建未失败：构建前检查无 `screen-rec.exe` 进程，exe 未被锁定。

产物（覆盖旧包）：

| 项 | 旧包（M15，被覆盖） | 新包（M15c，本轮） |
|---|---|---|
| 路径 | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe` | 同左 |
| 构建时间（mtime） | 2026-10-18 16:42:31 | **2026-10-18 17:08:36** |
| 大小 | 86,555,459 B（82.55 MiB） | **86,539,974 B（82.53 MiB）** |
| SHA-256 | `E500CAD85475F394ABCCE60E3B05E0ACAC2A451A4D46277EAC2D311B62E87B0D` | **`24268AC0B61897EE0906C093422C910CDCA6B4C7C96B37B3395E1BFCCF1C3FC2`** |

- release 主程序：`target\release\screen-rec.exe`，14,762,496 B，mtime 17:08:36，
  SHA-256 `F60F9A4A9C9B30E57ADC83793F652A182329ADC8A87FDB4BA6480203CF7858ED`。
- 二进制标记（release exe 实测存在，UTF-8 字节匹配）：`成品校验失败`、
  `清单已记成品但校验失败`、`原段保留、拒绝清理`（M15c/acc-7 错误串）、
  `keep_segments`（M14/M14c）、`audio_sample_rate`（M15）、`SR_DIAG_AV`（门控诊断）。
  release 符号表不保留 `verify_final_media` 名称，故以错误串 + `sr-pipeline` 重编 +
  §1 指纹共同证明修复进包。
- **UI 进包证据**：Tauri 复用了 16:39:07 生成的 `tauri-codegen-assets`（UI 源文件自 M15 后
  未变）：
  - `b2306b97…8a02.js`（压缩 14,977 B；brotli 解压 55,571 B）解压内容 SHA-256 =
    `E6EACD03…D0C1`，**等于当前 `ui/app.js` 指纹**；
  - `90679536…256d.js`（压缩 7,656 B；解压 22,903 B）解压内容 SHA-256 =
    `16E0CA19…C021`，**等于当前 `ui/api.js` 指纹**。
  证明包内 UI 仍为 M15 当前源码资产。
- 新包比 M15 小 15,485 B：源于 `sr-pipeline` 重编后的二进制约 1 KiB 变化及 NSIS 压缩差异；
  安装清单完整（§3），不是缺文件。

完整日志：`docs/milestones/m15cp-build-nsis.log`（已统一为 UTF-8；开头的
`node.exe : ...` 是 PowerShell 对 npx stderr 的非致命包装，构建以 `TAURI_BUILD_EXIT=0` 结束）。

## 3. 静默安装 + 安装目录自检

```
安装：Screen_rec_0.1.0_x64-setup.exe /S /D=C:\Users\elvis\AppData\Local\Temp\sr_m15cp_install
→ installer_exit=0；install_dir_exists=True（安装开始 2026-10-18 17:09:06）
```

安装目录清单（实测）：

| 文件 | 大小 | mtime |
|---|---|---|
| `screen-rec.exe` | 14,762,496 B | 2026-10-18 17:06:28 |
| `uninstall.exe` | 79,390 B | 2026-10-18 17:09:14 |
| `ffmpeg\ffmpeg.exe` | 134,092,288 B | 2026-10-15 12:43:48 |
| `ffmpeg\ffprobe.exe` | 133,887,488 B | 2026-10-15 12:43:48 |
| `licenses\FFmpeg-LICENSE.txt` | 7,651 B | 2026-10-15 12:43:48 |
| `licenses\ffmpeg-source-offer.txt` | 3,411 B | 2026-10-17 20:19:36 |

安装后哈希：

| 文件 | SHA-256 | 对照 |
|---|---|---|
| 安装 `screen-rec.exe` | `D81A7F36525E6DF914C6029EC89E21FA8B278A070C56486D07694122BF271106` | 与 release exe `F60F9A4A…` 不同（Tauri 打包会改写 exe；M12-G/M14/M15 同现象）。安装副本大小 14,762,496 B = release |
| 安装 `ffmpeg\ffmpeg.exe` | `3B57117B384DF98F2B8D01876DF0A596A0B6EBF45F756B30F15D6F65AB8251C5` | = 锁定构建包 |
| 安装 `ffmpeg\ffprobe.exe` | `5B9724EE7A115D5A8458D41BD2E0CE6206847E2B76105E822061A4421322CDE9` | = 锁定构建包 |
| 安装 `licenses\FFmpeg-LICENSE.txt` | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | = 构建包 LICENSE.txt |
| 安装 `licenses\ffmpeg-source-offer.txt` | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` | = 源文件（精确 commit 说明随包） |

安装副本 `screen-rec.exe` 二进制标记实测：`成品校验失败=True`、`keep_segments=True`、
`audio_sample_rate=True`、`SR_DIAG_AV=True`。

`--self-check`（安装目录运行；完整输出 `docs/milestones/m15cp-self-check.log`，
2026-10-18 17:09:33）：

```
[self-check] 搜索根（按优先级）
  env  SCREENREC_FFMPEG_DIR: (未设置)
  tauri resource_dir: (未设置)
  exe 同目录: C:\Users\elvis\AppData\Local\Temp\sr_m15cp_install
  开发回退: ...\Screen_rec\crates\sr-core\../../tools/ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin
[self-check] ffmpeg: ...\sr_m15cp_install\ffmpeg\ffmpeg.exe（来源：exe-adjacent）
[self-check]   -version: ffmpeg version n8.1.3-18-g46d899d7a0-20261015 Copyright (c) 2000-2026 the FFmpeg developers
[self-check] ffprobe: ...\sr_m15cp_install\ffmpeg\ffprobe.exe（来源：exe-adjacent）
[self-check]   -version: ffprobe version n8.1.3-18-g46d899d7a0-20261015 Copyright (c) 2007-2026 the FFmpeg developers
SELF_CHECK_EXIT=0
```

卸载与产物复核：

```
uninstall.exe /S → uninstall_exit=0；dir_exists_after_uninstall=False
安装包 SHA-256（卸载后）→ 24268AC0B61897EE0906C093422C910CDCA6B4C7C96B37B3395E1BFCCF1C3FC2（与构建时一致）
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
| `docs/milestones/m15c-package-report.md` | 本报告（新增） |
| `docs/milestones/m15cp-build-nsis.log` | NSIS 构建完整日志（`TAURI_BUILD_EXIT=0`，新增） |
| `docs/milestones/m15cp-fingerprint.log` | 20 项源码指纹三次记录与产物哈希（新增） |
| `docs/milestones/m15cp-self-check.log` | 安装清单/哈希、安装目录 `--self-check` 原始输出、卸载与产物复核（新增） |
| `docs/milestones/m15-package-report.md` | 顶部加一行指向本报告（其 16:42 旧包已被覆盖） |

未改任何源码/测试/`specs/`；§1 三次指纹一致可证。

> 说明：`docs/milestones/*.log` 被 `.gitignore` 的 `*.log` 忽略（与 M15 报告 §6.6 相同），
> 日志已落盘但默认不进入 git；如需提交日志，请编排者显式 `git add -f`。

## 6. 未决 / 风险（回交编排者）

1. **包内 M15c/acc-7 行为未做运行时功能复验**：本任务只做“构建 + 安装 + `--self-check`”；
   损坏成品拒绝清理/保留原段的证据在 `m14-report.md` §9 与 `m15-report.md` §8
   （单元红→绿、L2 recovery 5/5、workspace L1），未在 *release 安装包* 上重跑该场景。
   若验收要求安装包级证据，建议在安装目录构造零字节 `final_path` 会话做一次恢复拒绝清理验证。
2. **未签名**：NSIS 安装包仍未签名（同 M11-C/M12-G/M14/M15），生产分发需证书与签名步骤。
3. **`resource_dir` 分支未做 GUI 上下文端到端**：`--self-check` 先于 Tauri 启动；本次
   安装实测命中 `exe-adjacent`。既有单测覆盖优先级，GUI 注册见 `src-tauri/src/lib.rs`。
4. **未在干净机安装**：本机已有 WebView2；安装器对 WebView2 前置的引导未在无运行时
   机器审计（同 M11-C #4 / M12-G #4 / M14 #4 / M15 #4）。
5. **BtbN 构建脚本未按 commit 锁定**：FFmpeg 源码已精确到 commit；构建脚本仅锁定仓库 +
   构建日期，二进制由构建包 SHA-256 固定（同 M15 #5）。
6. **长测未运行**：按约束未跑电池/sync/soak；本报告不据此宣称长稳。
7. **安装后 exe 哈希与 release exe 不同**：Tauri 向 release 主程序写入 bundle 类型信息
   （“Patching … with bundle type information: nsis”）后再打包；安装副本与构建目录副本
   因此不必逐字节相同。安装包哈希与自检结果为准（同 M15 #7）。
8. **构建日志的 PowerShell 包装**：`m15cp-build-nsis.log` 开头 `node.exe :` 是 PowerShell
   对 npx stderr 的 `NativeCommandError` 包装（非致命）；成功构建行与
   `TAURI_BUILD_EXIT=0` 完整。日志已由混合编码统一为 UTF-8。
9. **M15c 测试/规格改动未计入 20 项指纹**：任务清单未要求；`tests/sr-itests/tests/{recovery.rs,
   sync_dual.rs}` 与 `specs/002-recording-modes.md` 的改动证据见 M15 报告 §8，本包不包含
   测试文件（bundle resources 不变）。

## 7. 证据索引（本机实际）

| 证据 | 内容 |
|---|---|
| `docs/milestones/m15cp-build-nsis.log` | `npx @tauri-apps/cli@2.12.3 build` 全量输出；17:06:04–17:08:36，exit 0，82.53 MiB |
| `docs/milestones/m15cp-fingerprint.log` | 20 项 SHA-256 构建前 17:05:58、构建后 17:08:43、安装后 17:09:43；`DIFF=0`；产物哈希 |
| `docs/milestones/m15cp-self-check.log` | 安装清单/哈希、self-check 输出（`SELF_CHECK_EXIT=0`，17:09:33）、卸载与产物复核 |
| 源码指纹 | §1（`Get-FileHash <file> -Algorithm SHA256`） |
| UI 进包 | §2（`tauri-codegen-assets` 16:39:07；解压 SHA-256 = 当前 `ui/app.js`/`ui/api.js`） |
| 安装/卸载 | §3（`installer_exit=0`、`uninstall_exit=0`、目录已删除、产物哈希复核一致） |
| M15c/acc-7 行为证据 | `docs/milestones/m14-report.md` §9（修复 + 单元红→绿）、`m15-report.md` §8（L2 recovery 5/5、route 1/1、workspace L1） |
| M15/M14c 行为证据 | `docs/milestones/m15-report.md`（REQ-1..8；§7 M15b）与 `m14-report.md` §8 |
