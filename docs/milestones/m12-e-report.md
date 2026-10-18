# M12-E 最终源码重建安装包（NSIS · 指纹 · 安装/自检）

> **后续更新（M12-F，2026-10-17 23:06）**：M12-E 之后新增 `SR_DIAG_AV` 门控诊断（`wasapi.rs`/`wgc.rs`/`route.rs`，默认关闭、行为中性），已从最终源码再次重建；**本报告的 21:08 安装包已被覆盖**。最新产物、指纹与安装/自检结果见 `docs/milestones/m12-f-report.md`。

- 日期：2026-10-17
- 范围：M12-C 之后源码又有改动（`region.rs` 的 `open_region_overlay` 改 async 以修复 WebView2 重入死锁；P2 截图失败保留覆盖层；`cargo fmt --all` 全仓库格式化，语义无变化）→ 从**当前最终源码**重建 NSIS 安装包并复验安装/自检。
- 流程：沿用 `docs/milestones/m11-c-report.md`（`npx @tauri-apps/cli@2.12.3 build`，resources 映射 ffmpeg/ffprobe/licenses）。
- 约束遵守：只做打包与报告；未改源码/测试/`specs/`/`docs/spikes/`；未 `git`；**未跑长测**（同步/soak 由编排者统一复跑）。

## 0. 结论

| 项 | 状态 | 关键证据 |
|---|---|---|
| 源码指纹 | **已记录（构建前后一致）** | §1：10 个文件 SHA-256；构建后重算完全相同 |
| NSIS 重建 | **完成，已覆盖 20:22 旧包** | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe`；21:08:11；86,565,902 B；SHA-256 `E5A8B762…7325A` |
| 静默安装 + 自检 | **exit 0** | `%TEMP%\sr_m12e_install`；ffmpeg/ffprobe 来源 `exe-adjacent`；版本 `n8.1.3-18-g46d899d7a0-20261015` |
| 安装可重复性 | **字节一致** | 两次安装 `screen-rec.exe` 均为 `1B80FBE3…15AB1`；两次卸载 exit 0，目录均删除 |
| ffmpeg source offer | **保持精确 commit** | 未改动；SHA-256 `FF1A3F3E…55808`；安装目录内同名文件哈希一致；仍为 `46d899d7a02bc63237472cc4897258d5ddf274ac` + 四种获取方式 |
| 长测 | **未运行（按约束）** | 仅打包/安装/自检 |

## 1. 源码指纹（构建前记录，构建后复核一致）

记录时间：2026-10-17 21:06（本机）。最晚源码 mtime = **21:01:13**（`ui/region.js`），新包 mtime = **21:08:11**。

| 文件 | SHA-256 | mtime |
|---|---|---|
| `Cargo.lock` | `5268A66731DC03F07653A0A3072B0E1704055742BCAB35724B3617BC66966AAF` | 2026-10-17 19:32:45 |
| `src-tauri/tauri.conf.json` | `95080F11734FA35F3BF28B3B51B01A4D8EA28A1AF742B4FD306FDDE040F819FC` | 2026-10-17 15:11:35 |
| `src-tauri/src/region.rs` | `A9A04F7C82E5E54894BA3ABAB0092BAA619D954DE741A6DE296C5D4310B96D24` | 2026-10-17 21:01:09 |
| `src-tauri/src/lib.rs` | `DEA816D9A960741086F2A4C0B12C16B5EFCE4DDD44E6E3C09B6DC7F928432EB6` | 2026-10-17 19:33:27 |
| `src-tauri/src/commands.rs` | `E460B7582BB20A22441AC7B6713BAC30B7D45E3DAD69FBDA82C330C883DD3BC2` | 2026-10-17 21:00:19 |
| `crates/sr-pipeline/src/route.rs` | `66C92458EC3940714C20BEE6C564E17E73A3C67D9C824FC1BF699BD5E3BF9B28` | 2026-10-17 21:00:19 |
| `crates/sr-pipeline/src/naming.rs` | `5BCA89EE4D00C28AF50CB941D276DDD22ADB178B95C082A6C58D85D3C243EB4B` | 2026-10-17 21:00:19 |
| `crates/sr-audio/src/resample.rs` | `4D39004B75D9F891256F99B2A9EE230F95E4B3E99D43A48FEEE7E345021B55C4` | 2026-10-17 21:00:19 |
| `ui/region.js` | `AAA0CD9AD104E521BDBD2B5DDACB714DB5AB25ED481BE951B739A2D40001D9A4` | 2026-10-17 21:01:13 |
| `ui/app.js` | `4289C4EB13BC93EC60E552A73F66813BA006E9C24073A28AD6A84EAE1FD8ACE7` | 2026-10-17 14:00:53 |

说明：`cargo fmt --all` 使多数 Rust 文件在 21:00:19 更新（仅格式化）；`region.rs`/`ui/region.js` 的改动为 async 覆盖层打开 + P2 失败保留覆盖层。本任务按任务说明以「语义无变化」为前提，未对格式化做独立语义审计。

上游证据：`docs/milestones/m12-d-report.md` 记录该版本（async 覆盖层打开 + P2 失败提示）已通过**真实 GUI** 验证（覆盖层可见、成品与覆盖前底层内容一致、失败提示可见），且 `cargo fmt --all -- --check` exit 0；本报告只复验打包/安装/自检，不重复 GUI 取证。

打包资源（本轮未改）：

| 文件 | SHA-256 | 说明 |
|---|---|---|
| `src-tauri/resources/licenses/ffmpeg-source-offer.txt` | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` | M12-C 修正版：精确 commit + 四种获取方式 |
| `src-tauri/resources/licenses/FFmpeg-LICENSE.txt` | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | 与构建包 LICENSE.txt 一致 |

## 2. 构建

工具链：Node v24.18.0 / npm 11.16.0；rustc 1.99.0 (b940084d7 2026-09-28) / cargo 1.99.0；`tauri-cli 2.12.3`（npx）。

```
npx --yes @tauri-apps/cli@2.12.3 build
→ Finished `release` profile [optimized] target(s) in 21.70s
→ Built application at: ...\target\release\screen-rec.exe
→ Info Patching ...\screen-rec.exe with bundle type information: nsis
→ Running makensis to produce ...\target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe
→ Finished 1 bundle at: ...\Screen_rec_0.1.0_x64-setup.exe (82.56 MiB)
→ TAURI_BUILD_EXIT=0
```

完整日志：`docs/milestones/m12e-build-nsis.log`。

产物（覆盖旧包）：

| 项 | 旧包（M12-C，被覆盖） | 新包（M12-E，本轮） |
|---|---|---|
| 路径 | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe` | 同左 |
| 构建时间（mtime） | 2026-10-17 20:22:12 | **2026-10-17 21:08:11** |
| 大小 | 86,572,089 B | **86,565,902 B（82.56 MiB）** |
| SHA-256 | `EECC55DF9F238BE8D5D3D04BE138B9EFB1A3DD6674829C7E51D64605FD134A6B` | **`E5A8B762AF7336A88E171F63D913AD1159D65B956FD7BE2460802CAF9D37325A`** |

- release 主程序：`target\release\screen-rec.exe`，14,712,320 B，mtime 21:08:11，SHA-256 `CCDDA90FBA4E0BD73F5486DCAEFDF9E99A8EDAD2725AF17963EEC05091946F03`。
- 二进制标记：`take_region_screenshot`、`MoveFileExW` 均在 release exe 中实测存在（`True`）。
- 前端资产：本次构建的 `ui/region.js`（含 async/P2 对应前端）以 Brotli 资产
  `target/release/build/screen-rec-ee450db19d850100/out/tauri-codegen-assets/a15c6368d9042c596a97d815c40fbc9e562465b2071177b02d7c3e3e56b8dd3b.js`
  嵌入；解压后 5,769 B，SHA-256 `AAA0CD9AD104E521BDBD2B5DDACB714DB5AB25ED481BE951B739A2D40001D9A4`，
  与 `ui/region.js` **逐字节一致**（`byte_identical=true`），含 P2 说明「失败时后端恢复覆盖层可见」与 `takeRegionScreenshot`。

## 3. 静默安装 + 安装目录自检

```
安装：Screen_rec_0.1.0_x64-setup.exe /S /D=C:\Users\elvis\AppData\Local\Temp\sr_m12e_install
→ installer_exit=0；install_dir_exists=True
```

安装目录清单（实测）：

| 文件 | 大小 | mtime |
|---|---|---|
| `screen-rec.exe` | 14,712,320 B | 21:06:10 |
| `uninstall.exe` | 79,390 B | 21:08:37 |
| `ffmpeg\ffmpeg.exe` | 134,092,288 B | 2026-10-15 12:43:48 |
| `ffmpeg\ffprobe.exe` | 133,887,488 B | 2026-10-15 12:43:48 |
| `licenses\FFmpeg-LICENSE.txt` | 7,651 B | 2026-10-15 12:43:48 |
| `licenses\ffmpeg-source-offer.txt` | 3,411 B | 2026-10-17 20:19:36 |

安装后哈希：

| 文件 | SHA-256 | 对照 |
|---|---|---|
| 安装 `screen-rec.exe` | `1B80FBE3B97187EEF147D683712DD6E5FDBE8511B4B394A6AFFC0FC48B815AB1` | 重复安装字节一致（见下）；含两项 M12 标记 |
| 安装 `ffmpeg\ffmpeg.exe` | `3B57117B384DF98F2B8D01876DF0A596A0B6EBF45F756B30F15D6F65AB8251C5` | = 锁定构建包 |
| 安装 `ffmpeg\ffprobe.exe` | `5B9724EE7A115D5A8458D41BD2E0CE6206847E2B76105E822061A4421322CDE9` | = 锁定构建包 |
| 安装 `licenses\FFmpeg-LICENSE.txt` | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | = 构建包 LICENSE.txt |
| 安装 `licenses\ffmpeg-source-offer.txt` | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` | = 源文件（精确 commit 说明随包） |

`--self-check`（安装目录运行；完整输出 `docs/milestones/m12e-self-check.log`）：

```
[self-check] 搜索根（按优先级）
  env  SCREENREC_FFMPEG_DIR: (未设置)
  tauri resource_dir: (未设置)
  exe 同目录: C:\Users\elvis\AppData\Local\Temp\sr_m12e_install
  开发回退: ...\tools\ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1/bin
[self-check] ffmpeg: ...\sr_m12e_install\ffmpeg\ffmpeg.exe（来源：exe-adjacent）
[self-check]   -version: ffmpeg version n8.1.3-18-g46d899d7a0-20261015 ...
[self-check] ffprobe: ...\sr_m12e_install\ffmpeg\ffprobe.exe（来源：exe-adjacent）
[self-check]   -version: ffprobe version n8.1.3-18-g46d899d7a0-20261015 ...
SELF_CHECK_EXIT=0
```

重复安装（第二空目录）：`installer_exit=0`，`screen-rec.exe` SHA-256 再次为 `1B80FBE3…15AB1`、大小 14,712,320 B（字节一致）。

卸载清理：两个安装目录分别 `uninstall.exe /S` → 均 `exit=0`；`dir1_exists=False`、`dir2_exists=False`。安装包保留在 `target\release\bundle\nsis\`，最终哈希复核仍为 `E5A8B762…7325A`。

## 4. ffmpeg source offer（保持 M12-C 精确说明）

- 文件未改动：SHA-256 `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808`，3411 B。
- 精确版本：`FFmpeg n8.1.3-18-g46d899d7a0` = tag `n8.1.3` + 18 commits，commit `46d899d7a02bc63237472cc4897258d5ddf274ac`。
- 四种获取方式（原文保留）：
  1. `git clone https://github.com/FFmpeg/FFmpeg.git` + `git checkout 46d899d7a02bc63237472cc4897258d5ddf274ac` + `git describe --tags --always`；
  2. GitHub commit 归档：`https://github.com/FFmpeg/FFmpeg/archive/46d899d7a02bc63237472cc4897258d5ddf274ac.tar.gz`；
  3. 上游 tag 对照：`https://github.com/FFmpeg/FFmpeg/releases/tag/n8.1.3`（注明少 18 commits，不能单独作为对应源码）；
  4. BtbN 构建脚本/日期：`https://github.com/BtbN/FFmpeg-Builds`（2026-10-15，`--extra-version=20261015`）。
- 随包验证：安装目录 `licenses/ffmpeg-source-offer.txt` 哈希 = 源文件哈希（§3）；`--self-check` 打印的 ffmpeg/ffprobe 版本与该说明一致。

## 5. 变更文件清单

| 文件 | 变更 |
|---|---|
| `docs/milestones/m12-e-report.md` | 本报告 |
| `docs/milestones/m12e-build-nsis.log` | NSIS 构建完整日志（`TAURI_BUILD_EXIT=0`） |
| `docs/milestones/m12e-self-check.log` | 安装目录 `--self-check` 原始输出（`SELF_CHECK_EXIT=0`） |

未改任何源码/测试/`specs/`/`docs/spikes/`；§1 构建前后指纹一致可证。安装包已覆盖 M12-C 的 20:22 产物。

## 6. 未决 / 风险（回交编排者）

1. **未签名**：NSIS 安装包仍未签名（同 M11-C #1），生产分发需证书与签名步骤。
2. **`resource_dir` 分支未做 GUI 上下文端到端**：`--self-check` 先于 Tauri 启动；本次安装实测命中 `exe-adjacent`。既有单测覆盖优先级，GUI 注册见 `src-tauri/src/lib.rs:105–108`。
3. **未在干净机安装**：本机已有 WebView2；安装器对 WebView2 前置的引导未在无运行时机器审计（同 M11-C #4）。
4. **BtbN 构建脚本未按 commit 锁定**：FFmpeg 源码已精确到 commit；构建脚本仅锁定仓库 + 构建日期，二进制由构建包 SHA-256 固定。
5. **格式化语义未独立审计**：`cargo fmt --all` 涉及全仓库 Rust 文件（21:00:19）；按任务说明语义无变化，本任务只做打包，未逐文件做语义 diff。
6. **长测未运行**：按约束未跑同步/soak；本报告不据此宣称长稳，待编排者统一串行复跑。
7. **开发回退路径编入二进制**：`--self-check` 会打印仓库 `tools/ffmpeg/...` 路径（最后兜底，实际未命中）；安装仍不依赖源码树。

## 7. 证据索引（本机实际）

| 证据 | 内容 |
|---|---|
| `docs/milestones/m12e-build-nsis.log` | `npx @tauri-apps/cli@2.12.3 build` 全量输出，exit 0，82.56 MiB |
| `docs/milestones/m12e-self-check.log` | 安装目录 self-check 输出，`SELF_CHECK_EXIT=0`，版本 `n8.1.3-18-g46d899d7a0-20261015` |
| 源码指纹 | §1（构建前记录，构建后重算一致；命令 `Get-FileHash <file> -Algorithm SHA256`） |
| 安装/卸载 | §3（两次 `installer_exit=0`、两次 `uninstall_exit=0`、目录均删除） |
| 嵌入前端资产 | §2（解压后与 `ui/region.js` 逐字节一致，SHA-256 `AAA0CD9A…9D4`） |
