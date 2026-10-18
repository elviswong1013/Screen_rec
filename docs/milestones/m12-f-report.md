# M12-F 最终源码重建安装包（NSIS · 指纹 · 安装/自检）

> **后续更新（M12-G，2026-10-18 00:19）**：M12-F 之后新增 WASAPI 停滞/续流 P1 修复（`slots.rs`/`wasapi.rs` + `stall_pad_policy.rs`）。已从最终源码再次重建；**本报告的 23:06 安装包已被覆盖**。最新产物、指纹与安装/自检结果见 `docs/milestones/m12-g-report.md`。

- 日期：2026-10-17
- 范围：M12-E（21:08 包）之后源码新增 `SR_DIAG_AV=1` 门控诊断（`crates/sr-audio/src/wasapi.rs`、`crates/sr-capture/src/wgc.rs`、`crates/sr-pipeline/src/route.rs`；默认关闭、行为中性，见 `docs/milestones/m12-h-report.md`）→ 从当前最终源码重建 NSIS 安装包并复验安装/自检。
- 流程：沿用 `docs/milestones/m11-c-report.md` / `docs/milestones/m12-e-report.md`（`npx @tauri-apps/cli@2.12.3 build`，resources 映射 ffmpeg/ffprobe/licenses）。
- 约束遵守：只做打包与报告；未改源码/测试/`specs/`/`docs/spikes/`；未 `git`；**未跑长测**（电池/sync/soak 由编排者统一复跑）。

## 0. 结论

| 项 | 状态 | 关键证据 |
|---|---|---|
| 源码指纹 | **已记录（构建前后一致）** | §1：12 个文件 SHA-256；构建后重算完全相同 |
| NSIS 重建 | **完成，已覆盖 21:08 旧包** | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe`；23:06:11；86,570,680 B；SHA-256 `EEAF6EC6…41DC0` |
| 静默安装 + 自检 | **exit 0** | `%TEMP%\sr_m12f_install`；ffmpeg/ffprobe 来源 `exe-adjacent`；版本 `n8.1.3-18-g46d899d7a0-20261015` |
| 卸载清理 | **完成** | `uninstall.exe /S` exit 0；安装目录已删除 |
| 产物哈希复核 | **一致** | 卸载后安装包 SHA-256 仍为 `EEAF6EC6…41DC0` |
| ffmpeg source offer | **保持精确 commit** | 未改动；SHA-256 `FF1A3F3E…55808`；安装目录内同名文件哈希一致 |
| 长测 | **未运行（按约束）** | 仅打包/安装/自检 |

## 1. 源码指纹（构建前记录，构建后复核一致）

记录时间：2026-10-17 23:03（本机）。最晚源码 mtime = **22:29:18**（`crates/sr-pipeline/src/route.rs`），新包 mtime = **23:06:11**。

| 文件 | SHA-256 | mtime |
|---|---|---|
| `Cargo.lock` | `5268A66731DC03F07653A0A3072B0E1704055742BCAB35724B3617BC66966AAF` | 2026-10-17 19:32:45 |
| `src-tauri/tauri.conf.json` | `95080F11734FA35F3BF28B3B51B01A4D8EA28A1AF742B4FD306FDDE040F819FC` | 2026-10-17 15:11:35 |
| `src-tauri/src/region.rs` | `A9A04F7C82E5E54894BA3ABAB0092BAA619D954DE741A6DE296C5D4310B96D24` | 2026-10-17 21:01:09 |
| `src-tauri/src/lib.rs` | `DEA816D9A960741086F2A4C0B12C16B5EFCE4DDD44E6E3C09B6DC7F928432EB6` | 2026-10-17 19:33:27 |
| `src-tauri/src/commands.rs` | `E460B7582BB20A22441AC7B6713BAC30B7D45E3DAD69FBDA82C330C883DD3BC2` | 2026-10-17 21:00:19 |
| `crates/sr-pipeline/src/route.rs` | `8CEDF4E6A0C1980E52C2661209344BFEFA40ECF8CE751BBD2BC9864BCDFE4B0C` | 2026-10-17 22:29:18 |
| `crates/sr-pipeline/src/naming.rs` | `5BCA89EE4D00C28AF50CB941D276DDD22ADB178B95C082A6C58D85D3C243EB4B` | 2026-10-17 21:00:19 |
| `crates/sr-audio/src/resample.rs` | `4D39004B75D9F891256F99B2A9EE230F95E4B3E99D43A48FEEE7E345021B55C4` | 2026-10-17 21:00:19 |
| `crates/sr-audio/src/wasapi.rs` | `94BA11D3C1352E0C9D12C7EEC9588D0DF25536FFFEE4275182BF977077885805` | 2026-10-17 22:04:16 |
| `crates/sr-capture/src/wgc.rs` | `90B244810A5BED028E5233CC12B2E3C7A6504C6D543A5BD6ECF1BCC83BA12928` | 2026-10-17 22:28:49 |
| `ui/region.js` | `AAA0CD9AD104E521BDBD2B5DDACB714DB5AB25ED481BE951B739A2D40001D9A4` | 2026-10-17 21:01:13 |
| `ui/app.js` | `4289C4EB13BC93EC60E552A73F66813BA006E9C24073A28AD6A84EAE1FD8ACE7` | 2026-10-17 14:00:53 |

说明：本轮的源码增量为 `SR_DIAG_AV` 门控诊断（`wasapi.rs:103–110`、`wgc.rs:137–149`、`route.rs:245–249` 等）；默认关闭（仅 `SR_DIAG_AV=1|true` 时输出 stderr 诊断），按 `docs/milestones/m12-h-report.md` 为行为中性。

打包资源（本轮未改）：

| 文件 | SHA-256 | 说明 |
|---|---|---|
| `src-tauri/resources/licenses/ffmpeg-source-offer.txt` | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` | M12-C 修正版：精确 commit + 四种获取方式 |
| `src-tauri/resources/licenses/FFmpeg-LICENSE.txt` | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | 与构建包 LICENSE.txt 一致 |

## 2. 构建

工具链：Node v24.18.0 / npm 11.16.0；rustc 1.99.0 / cargo 1.99.0；`tauri-cli 2.12.3`（npx）。

```
npx --yes @tauri-apps/cli@2.12.3 build
→ Finished `release` profile [optimized] target(s) in 27.21s
→ Built application at: ...\target\release\screen-rec.exe
→ Info Patching ...\screen-rec.exe with bundle type information: nsis
→ Running makensis to produce ...\target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe
→ Finished 1 bundle at: ...\Screen_rec_0.1.0_x64-setup.exe (82.56 MiB)
→ TAURI_BUILD_EXIT=0
```

完整日志：`docs/milestones/m12f-build-nsis.log`。

产物（覆盖旧包）：

| 项 | 旧包（M12-E，被覆盖） | 新包（M12-F，本轮） |
|---|---|---|
| 路径 | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe` | 同左 |
| 构建时间（mtime） | 2026-10-17 21:08:11 | **2026-10-17 23:06:11** |
| 大小 | 86,565,902 B | **86,570,680 B（82.56 MiB）** |
| SHA-256 | `E5A8B762AF7336A88E171F63D913AD1159D65B956FD7BE2460802CAF9D37325A` | **`EEAF6EC6311051BE68C31C652BD39529CCD64588EAF2B0700BB5D52809B41DC0`** |

- release 主程序：`target\release\screen-rec.exe`，14,716,416 B，mtime 23:06:11，SHA-256 `A8783D476809E25C49F5A4D0C1FAE0054A960B1232FB4684A44ED9B086ABC4C9`。
- 二进制标记（release exe 实测存在）：`SR_DIAG_AV`（本轮诊断已编入）、`take_region_screenshot`、`MoveFileExW`。

## 3. 静默安装 + 安装目录自检

```
安装：Screen_rec_0.1.0_x64-setup.exe /S /D=C:\Users\elvis\AppData\Local\Temp\sr_m12f_install
→ installer_exit=0；install_dir_exists=True
```

安装目录清单（实测）：

| 文件 | 大小 | mtime |
|---|---|---|
| `screen-rec.exe` | 14,716,416 B | 23:04:04 |
| `uninstall.exe` | 79,390 B | 23:06:28 |
| `ffmpeg\ffmpeg.exe` | 134,092,288 B | 2026-10-15 12:43:48 |
| `ffmpeg\ffprobe.exe` | 133,887,488 B | 2026-10-15 12:43:48 |
| `licenses\FFmpeg-LICENSE.txt` | 7,651 B | 2026-10-15 12:43:48 |
| `licenses\ffmpeg-source-offer.txt` | 3,411 B | 2026-10-17 20:19:36 |

安装后哈希：

| 文件 | SHA-256 | 对照 |
|---|---|---|
| 安装 `screen-rec.exe` | `877E0FA4EED8F72743843FCF029E26EE656133BFDBF83840A7C17BF82AD64602` | 含 `SR_DIAG_AV` / `take_region_screenshot` 标记 |
| 安装 `ffmpeg\ffmpeg.exe` | `3B57117B384DF98F2B8D01876DF0A596A0B6EBF45F756B30F15D6F65AB8251C5` | = 锁定构建包 |
| 安装 `ffmpeg\ffprobe.exe` | `5B9724EE7A115D5A8458D41BD2E0CE6206847E2B76105E822061A4421322CDE9` | = 锁定构建包 |
| 安装 `licenses\FFmpeg-LICENSE.txt` | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | = 构建包 LICENSE.txt |
| 安装 `licenses\ffmpeg-source-offer.txt` | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` | = 源文件（精确 commit 说明随包） |

`--self-check`（安装目录运行；完整输出 `docs/milestones/m12f-self-check.log`）：

```
[self-check] 搜索根（按优先级）
  env  SCREENREC_FFMPEG_DIR: (未设置)
  tauri resource_dir: (未设置)
  exe 同目录: C:\Users\elvis\AppData\Local\Temp\sr_m12f_install
  开发回退: ...\tools\ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1/bin
[self-check] ffmpeg: ...\sr_m12f_install\ffmpeg\ffmpeg.exe（来源：exe-adjacent）
[self-check]   -version: ffmpeg version n8.1.3-18-g46d899d7a0-20261015 ...
[self-check] ffprobe: ...\sr_m12f_install\ffmpeg\ffprobe.exe（来源：exe-adjacent）
[self-check]   -version: ffprobe version n8.1.3-18-g46d899d7a0-20261015 ...
SELF_CHECK_EXIT=0
```

卸载与产物复核：

```
uninstall.exe /S → uninstall_exit=0；dir_exists_after_uninstall=False
安装包 SHA-256（卸载后）→ EEAF6EC6311051BE68C31C652BD39529CCD64588EAF2B0700BB5D52809B41DC0（与构建时一致）
```

## 4. ffmpeg source offer（保持 M12-C 精确说明）

- 文件未改动：SHA-256 `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808`，3411 B。
- 精确版本：`FFmpeg n8.1.3-18-g46d899d7a0` = tag `n8.1.3` + 18 commits，commit `46d899d7a02bc63237472cc4897258d5ddf274ac`。
- 四种获取方式保留：git checkout / GitHub commit 归档 / 上游 tag 对照（注明少 18 commits）/ BtbN 构建脚本与日期。
- 随包验证：安装目录 `licenses/ffmpeg-source-offer.txt` 哈希 = 源文件哈希（§3）；`--self-check` 打印的 ffmpeg/ffprobe 版本与该说明一致。

## 5. 变更文件清单

| 文件 | 变更 |
|---|---|
| `docs/milestones/m12-f-report.md` | 本报告 |
| `docs/milestones/m12f-build-nsis.log` | NSIS 构建完整日志（`TAURI_BUILD_EXIT=0`） |
| `docs/milestones/m12f-self-check.log` | 安装目录 `--self-check` 原始输出（`SELF_CHECK_EXIT=0`） |
| `docs/milestones/m12-e-report.md` | 顶部加一行指向本报告（其 21:08 包已被覆盖） |

未改任何源码/测试/`specs/`/`docs/spikes/`；§1 构建前后指纹一致可证。

## 6. 未决 / 风险（回交编排者）

1. **未签名**：NSIS 安装包仍未签名（同 M11-C），生产分发需证书与签名步骤。
2. **`resource_dir` 分支未做 GUI 上下文端到端**：`--self-check` 先于 Tauri 启动；本次安装实测命中 `exe-adjacent`。既有单测覆盖优先级，GUI 注册见 `src-tauri/src/lib.rs:105–108`。
3. **未在干净机安装**：本机已有 WebView2；安装器对 WebView2 前置的引导未在无运行时机器审计（同 M11-C #4）。
4. **BtbN 构建脚本未按 commit 锁定**：FFmpeg 源码已精确到 commit；构建脚本仅锁定仓库 + 构建日期，二进制由构建包 SHA-256 固定。
5. **诊断代码的运行影响未在本任务复验**：`SR_DIAG_AV` 默认关闭（仅 env=1/true 时输出 stderr），按 `m12-h-report.md` 为行为中性；本任务只验证打包与安装/自检。
6. **长测未运行**：按约束未跑电池/sync/soak；本报告不据此宣称长稳，待编排者统一串行复跑。
7. **开发回退路径编入二进制**：`--self-check` 会打印仓库 `tools/ffmpeg/...` 路径（最后兜底，实际未命中）；安装仍不依赖源码树。

## 7. 证据索引（本机实际）

| 证据 | 内容 |
|---|---|
| `docs/milestones/m12f-build-nsis.log` | `npx @tauri-apps/cli@2.12.3 build` 全量输出，exit 0，82.56 MiB |
| `docs/milestones/m12f-self-check.log` | 安装目录 self-check 输出，`SELF_CHECK_EXIT=0`，版本 `n8.1.3-18-g46d899d7a0-20261015` |
| 源码指纹 | §1（构建前记录，构建后重算一致；`Get-FileHash <file> -Algorithm SHA256`） |
| 安装/卸载 | §3（`installer_exit=0`、`uninstall_exit=0`、目录已删除、产物哈希复核一致） |
| 诊断证据 | `docs/milestones/m12-h-report.md`（`SR_DIAG_AV=1` 门控、默认关闭） |
