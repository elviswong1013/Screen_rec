# M12-C 重建发行包并补证据（NSIS · 源码指纹 · 自检 · ffmpeg source offer）

> **后续更新（M12-E，2026-10-17）**：M12-C 之后源码又有改动（`region.rs` 异步覆盖层修复 + P2 失败保留覆盖层 + `cargo fmt --all`）。已从最终源码再次重建，**本报告的 20:22 安装包已被覆盖**；最新产物、指纹与安装/自检结果见 `docs/milestones/m12-e-report.md`。

- 日期：2026-10-17
- 角色：executor-backup（编排者/主体执行器的后备执行通道）
- 范围：从 **M12 最终源码**重建 NSIS 安装包；记录源码指纹；静默安装 + 安装目录 `--self-check`；修正 `ffmpeg-source-offer.txt` 精确对应 `n8.1.3-18-g46d899d7a0` 并随包验证。
- 约束遵守：除打包资源/说明外未改源码；未 `git` 提交/暂存；未改测试、`specs/`、`docs/spikes/`；**未跑任何长测/L2**（机器留给后续短测电池）。
- 背景：acceptor 指出现有安装包（15:16:13 构建）早于 M12 整改源码（`region.rs` 19:39 / `naming.rs` 19:41 / `resample.rs` 19:45），且 source offer 仍写 FFmpeg `n8.1`，与实际锁定构建 `n8.1.3-18-g46d899d7a0` 不符。

## 0. 结论

| 项 | 状态 | 关键证据 |
|---|---|---|
| 源码指纹 | **已记录（构建前后一致）** | §1：9 个文件 SHA-256；构建后重算完全相同 |
| NSIS 重建（最终源码） | **完成** | `target/release/bundle/nsis/Screen_rec_0.1.0_x64-setup.exe`；20:22:12；86,572,089 B；SHA-256 `EECC55DF…34A6B`（旧包 15:16:13 / `E650BF31…C2006`，已被覆盖） |
| 静默安装 + `--self-check` | **exit 0** | 安装目录 `%TEMP%\sr_m12c_install`；ffmpeg/ffprobe 来源 `exe-adjacent`；版本 `n8.1.3-18-g46d899d7a0-20261015` |
| source offer 修正 | **完成并随包验证** | 精确到 commit `46d899d7a02bc63237472cc4897258d5ddf274ac`（tag `n8.1.3` + 18 commits）；安装目录文件哈希 = 源文件哈希 `FF1A3F3E…55808` |
| 安装态清理 | **完成** | `uninstall.exe /S` exit 0；安装目录已删除 |
| 长测 | **未运行（按约束）** | 仅执行打包/安装/自检；无 L2/L3 |

## 1. 源码指纹（构建前记录，构建后复核）

记录时间：2026-10-17 20:18（本机）。构建完成后再算一次，**全部相同**（证明产物来自该快照、构建期间源码未被改动）。

| 文件 | SHA-256 | mtime |
|---|---|---|
| `Cargo.lock` | `5268A66731DC03F07653A0A3072B0E1704055742BCAB35724B3617BC66966AAF` | 2026-10-17 19:32:45 |
| `src-tauri/tauri.conf.json` | `95080F11734FA35F3BF28B3B51B01A4D8EA28A1AF742B4FD306FDDE040F819FC` | 2026-10-17 15:11:35 |
| `src-tauri/src/region.rs` | `1FA47BA31D3058EBE465634573B8D49D380C9690F9850B5D0A295B03A50AC52C` | 2026-10-17 19:39:42 |
| `src-tauri/src/lib.rs` | `DEA816D9A960741086F2A4C0B12C16B5EFCE4DDD44E6E3C09B6DC7F928432EB6` | 2026-10-17 19:33:27 |
| `src-tauri/src/commands.rs` | `0FEE34504958E675E9501AF5A413DEBA106C8DCC47397176973A742BD15828D9` | 2026-10-17 19:33:04 |
| `crates/sr-pipeline/src/route.rs` | `B4F5E5B5B458942D999EB85A6E59B667306159D9FCE3E8ED159DC69AA690945C` | 2026-10-17 19:44:55 |
| `crates/sr-pipeline/src/naming.rs` | `C5C50D3A1B012750C2457B1CC0E210B674FC6A08F4D0049898397E368D45ED92` | 2026-10-17 19:41:49 |
| `crates/sr-audio/src/resample.rs` | `3E98B855BBB8D854716BBDB9CE803C68E90952320EAA4A28E4345B4AD165B1E3` | 2026-10-17 19:45:53 |
| `ui/region.js` | `767C7C556E103ED5D91F90EBFB5A0806D1BE9B2EBA361F328079A622C0DFB2D8` | 2026-10-17 19:33:35 |

关键结论：最晚源码 mtime = **19:45:53**；新安装包构建于 **20:22:12**，晚于全部 M12 整改源码（覆盖 acceptor 指出的「15:16 包早于源码」问题）。

打包资源/说明（本轮新改）：

| 文件 | SHA-256 | 说明 |
|---|---|---|
| `src-tauri/resources/licenses/ffmpeg-source-offer.txt` | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` | 本轮修正（旧 2108 B → 新 3411 B） |
| `src-tauri/resources/licenses/FFmpeg-LICENSE.txt` | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | 未改；与构建包 LICENSE.txt 一致 |

## 2. 构建（按 M11-C 流程）

工具链：Node v24.18.0 / npm 11.16.0；rustc 1.99.0 (b940084d7 2026-09-28) / cargo 1.99.0；`cargo tauri` 子命令不存在，沿用 M11-C 的 npx CLI：

```
npx --yes @tauri-apps/cli@2.12.3 build
```

实际结果（完整日志 `docs/milestones/m12c-build-nsis.log`）：

```
Finished `release` profile [optimized] target(s) in 31.24s
Built application at: ...\target\release\screen-rec.exe
Info Patching ...\screen-rec.exe with bundle type information: nsis
Info Target: x64
Running makensis to produce ...\target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe
Finished 1 bundle at: ...\Screen_rec_0.1.0_x64-setup.exe (82.56 MiB)
TAURI_BUILD_EXIT=0
```

产物：

| 项 | 旧包（被覆盖） | 新包（本轮） |
|---|---|---|
| 路径 | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe` | 同左 |
| 构建时间（mtime） | 2026-10-17 15:16:13 | **2026-10-17 20:22:12** |
| 大小 | 86,485,899 B | **86,572,089 B（82.56 MiB）** |
| SHA-256 | `E650BF31DF9E517E3ED7BB887C828BDB37F66AE664D4D209891CDB7B09CC2006` | **`EECC55DF9F238BE8D5D3D04BE138B9EFB1A3DD6674829C7E51D64605FD134A6B`** |
| 版本信息 | 0.1.0 / Screen_rec | 0.1.0 / Screen_rec |

release 主程序：`target\release\screen-rec.exe`，14,686,720 B，mtime 20:22:12，SHA-256 `2732E05744D042083BD4965D70C35426BB58B8B915BAEEFFAE8704EACF5BA3F4`。

二进制特征（证明包含 M12 改动）：
- `take_region_screenshot` 字符串存在（P1-A 命令已注册）；
- `MoveFileExW` 导入名存在（P1-B 原子提交路径已编入）。
两处均在 `target/release/screen-rec.exe` 中实测为 `True`。

前端资产同样来自最终源码（P1-A 的 `ui/region.js`）：本次 release 构建生成的 Brotli 资产
`target/release/build/screen-rec-ee450db19d850100/out/tauri-codegen-assets/8559834b…c1235.js`（mtime 20:19:58）解压后
5,644 B，SHA-256 `767C7C556E103ED5D91F90EBFB5A0806D1BE9B2EBA361F328079A622C0DFB2D8`，
与 `ui/region.js` **逐字节一致**（`byte_identical=true`），含 `takeRegionScreenshot` /
`take_region_screenshot`，不含旧 `takeScreenshot: (rect)`。该哈希与 §1 的 `ui/region.js` 指纹一致。

CLI 版本提示：`tauri-cli 2.12.3` 与 crate `tauri 2.12.4` 的 Info 级版本检查沿用 M11-C 结论，不影响构建（exit 0）。

## 3. 静默安装 + 安装目录自检

```
安装：Screen_rec_0.1.0_x64-setup.exe /S /D=C:\Users\elvis\AppData\Local\Temp\sr_m12c_install
→ installer_exit=0；install_dir_exists=True
```

安装目录清单（实测）：

| 文件 | 大小 | mtime |
|---|---|---|
| `screen-rec.exe` | 14,686,720 B | 20:20:20 |
| `uninstall.exe` | 79,390 B | 20:22:27 |
| `ffmpeg\ffmpeg.exe` | 134,092,288 B | 2026-10-15 12:43:48 |
| `ffmpeg\ffprobe.exe` | 133,887,488 B | 2026-10-15 12:43:48 |
| `licenses\FFmpeg-LICENSE.txt` | 7,651 B | 2026-10-15 12:43:48 |
| `licenses\ffmpeg-source-offer.txt` | 3,411 B | 20:19:36 |

安装后哈希核对：

| 文件 | SHA-256 | 对照 |
|---|---|---|
| 安装 `screen-rec.exe` | `306CC4670170832DA1807065915955B9BA2CBD8DE6432E7008339517D7EF79E7` | 与 release exe 不同属构建流程：构建日志记录 `Patching … with bundle type information: nsis`（推断为打包期二进制标记/还原；大小不变，重复安装字节一致，见下） |
| 安装 `ffmpeg\ffmpeg.exe` | `3B57117B384DF98F2B8D01876DF0A596A0B6EBF45F756B30F15D6F65AB8251C5` | = 锁定构建包 |
| 安装 `ffmpeg\ffprobe.exe` | `5B9724EE7A115D5A8458D41BD2E0CE6206847E2B76105E822061A4421322CDE9` | = 锁定构建包 |
| 安装 `licenses\FFmpeg-LICENSE.txt` | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | = 构建包 LICENSE.txt |
| 安装 `licenses\ffmpeg-source-offer.txt` | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` | = 本轮源文件（随包生效） |

重复性复核：第二次静默安装到另一空目录 → `installer_exit=0`，安装的 `screen-rec.exe` SHA-256 再次为 `306CC467…79E7`（大小 14,686,720 B、mtime 20:20:20 均一致），且仍含 `take_region_screenshot` / `MoveFileExW` 标记；随后 `uninstall.exe /S` → `uninstall_exit=0`，目录已删除。说明该安装包内容确定、安装/卸载可重复。

`--self-check`（从安装目录运行；完整输出 `docs/milestones/m12c-self-check.log`）：

```
[self-check] 搜索根（按优先级）
  env  SCREENREC_FFMPEG_DIR: (未设置)
  tauri resource_dir: (未设置)
  exe 同目录: C:\Users\elvis\AppData\Local\Temp\sr_m12c_install
  开发回退: ...\tools\ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1/bin
[self-check] ffmpeg: ...\sr_m12c_install\ffmpeg\ffmpeg.exe（来源：exe-adjacent）
[self-check]   -version: ffmpeg version n8.1.3-18-g46d899d7a0-20261015 ...
[self-check] ffprobe: ...\sr_m12c_install\ffmpeg\ffprobe.exe（来源：exe-adjacent）
[self-check]   -version: ffprobe version n8.1.3-18-g46d899d7a0-20261015 ...
SELF_CHECK_EXIT=0
```

说明：`--self-check` 在 Tauri 启动前执行（无 `AppHandle`），`resource_dir` 显示「未设置」属既有行为（M11-C 未决 #2）；实际命中 `exe-adjacent`，即安装目录自带的 `ffmpeg/`，与发行包独立性要求一致。

卸载（按流程清理）：

```
uninstall.exe /S → uninstall_exit=0；install_dir_exists_after_uninstall=False
```

## 4. `ffmpeg-source-offer.txt` 修正

### 4.1 问题

旧说明写「FFmpeg n8.1」并指向 `https://github.com/FFmpeg/FFmpeg/tree/n8.1` 与 `ffmpeg-8.1.tar.xz`；实际二进制版本为 **`n8.1.3-18-g46d899d7a0`**（18 commits after tag `n8.1.3`），旧指向不能精确对应。

### 4.2 精确版本与源码的核对（实测）

- 二进制 `-version` 首行：`ffmpeg version n8.1.3-18-g46d899d7a0-20261015`（安装后 self-check 实测，见 §3）；
- commit 页面/补丁头实测（GitHub `FFmpeg/FFmpeg@46d899d7a0.patch`）：
  ```
  From 46d899d7a02bc63237472cc4897258d5ddf274ac Mon Sep 17 00:00:00 2001
  Date: Sun, 11 Oct 2026 11:07:34 -0300
  Subject: [PATCH] avformat/mov: check ff_mov_read_chnl return value
  ```
- tag `n8.1.3` 存在（GitHub release 页面 `FFmpeg 8.1.3 release`）；
- 构建包 SHA-256 `337B8546…8C240`、ffmpeg/ffprobe/LICENSE 哈希均与旧说明记录一致（未更换二进制）。

### 4.3 新说明内容（`src-tauri/resources/licenses/ffmpeg-source-offer.txt`，3411 B）

- 版本：`FFmpeg n8.1.3-18-g46d899d7a0` = tag `n8.1.3` + 18 commits，commit `46d899d7a02bc63237472cc4897258d5ddf274ac`（含 `ffmpeg -version` 首行与 self-check 核对提示）；
- 获取方式（三条）：
  1. `git clone https://github.com/FFmpeg/FFmpeg.git` + `git checkout 46d899d7a02bc63237472cc4897258d5ddf274ac` + `git describe --tags --always`（预期 `n8.1.3-18-g46d899d7a0`）；
  2. GitHub 归档快照 `https://github.com/FFmpeg/FFmpeg/archive/46d899d7a02bc63237472cc4897258d5ddf274ac.tar.gz`；
  3. 上游 tag `https://github.com/FFmpeg/FFmpeg/releases/tag/n8.1.3`（明确标注「少 18 个提交，不能单独作为对应源码」）；
- 构建脚本/日期：BtbN/FFmpeg-Builds，2026-10-15（`--extra-version=20261015`）+ 构建包/二进制/许可证 SHA-256；
- 保留原 LGPL 合规要点（独立进程调用、可替换 `SCREENREC_FFMPEG_DIR`、未修改源码、构建配置审计）。

### 4.4 随包验证

- `tauri build` 按 `src-tauri/tauri.conf.json:27–32` 将 `resources/licenses/ffmpeg-source-offer.txt` 映射为安装包 `licenses/ffmpeg-source-offer.txt`；
- 安装目录内文件 SHA-256 = 源文件 SHA-256 `FF1A3F3E…55808`（§3），且内容含 `n8.1.3-18-g46d899d7a0` 与完整 commit（实测检索命中，文件行 6/7/29/34/37）。

## 5. 变更文件清单

| 文件 | 变更 |
|---|---|
| `src-tauri/resources/licenses/ffmpeg-source-offer.txt` | 修正为精确版本/commit 与获取方式（打包资源，非源码） |
| `docs/milestones/m12-c-report.md` | 本报告 |
| `docs/milestones/m12c-build-nsis.log` | NSIS 构建完整日志（exit 0） |
| `docs/milestones/m12c-self-check.log` | 安装目录 `--self-check` 原始输出（exit 0） |

未改任何 Rust/JS 源码或测试；§1 的构建前后 SHA-256 一致可证。未改 `Cargo.lock`、`tauri.conf.json`。

## 6. 未决 / 风险（回交编排者）

1. **未签名**：安装包仍为未签名 NSIS（同 M11-C #1），生产分发需证书/签名步骤。
2. **`resource_dir` 分支未做 GUI 上下文端到端**：`--self-check` 早于 Tauri 启动；本次安装实测命中 `exe-adjacent`。既有单测覆盖优先级，GUI 启动注册见 `src-tauri/src/lib.rs:105–108`。
3. **未在干净机安装**：本机已有 WebView2；Tauri 安装器对 WebView2 前置的引导未在无运行时机器审计（同 M11-C #4）。
4. **BtbN 构建脚本未按 commit 锁定**：FFmpeg 源码已精确到 commit；构建脚本仅锁定仓库 + 构建日期 2026-10-15，二进制由构建包 SHA-256 固定。
5. **开发回退路径编入二进制**：self-check 会打印仓库 `tools/ffmpeg/...` 路径（最后兜底，实际未命中）；安装目录仍不依赖源码树（本次来源 `exe-adjacent`，与 M11-C 结论一致）。
6. **长测未运行**：按任务约束，L2/L3 与短测电池留给编排者后续串行执行；本报告不据此宣称长稳。

## 7. 证据索引（本机实际）

| 证据 | 内容 |
|---|---|
| `docs/milestones/m12c-build-nsis.log` | `npx @tauri-apps/cli@2.12.3 build` 全量输出，`TAURI_BUILD_EXIT=0`，产物 82.56 MiB |
| `docs/milestones/m12c-self-check.log` | 安装目录 self-check 输出，`SELF_CHECK_EXIT=0`，版本 `n8.1.3-18-g46d899d7a0-20261015` |
| 源码指纹 | §1（构建前记录；构建后重算一致，命令即 `Get-FileHash <file> -Algorithm SHA256`） |
| 安装/卸载 | §3（`installer_exit=0`；`install_dir_exists_after_uninstall=False`） |
| 旧包对照 | §2（旧 SHA-256 `E650BF31…C2006`，15:16:13） |
