# M14 安装包重建（NSIS · 指纹 · 安装/自检）

> **后续更新（M15/M14c，2026-10-18 16:42）**：M14 之后源码新增 M15（采样率可选 P1）与 M14c（恢复收尾纳入 `keep_segments` 清理），已从当前最终源码再次重建；**本报告的 15:02 安装包已被覆盖**。最新产物、指纹与安装/自检结果见 `docs/milestones/m15-package-report.md`。

- 日期：2026-10-18
- 范围：M12-G（2026-10-18 00:19 包，SHA-256 `A99BE175…05D29`）之后源码新增
  **M14**（成品成功后自动清理原段 + `keep_segments` 设置）与 **M14b**（纯录音纳入同一
  语义）——证据 `docs/milestones/m14-report.md`（含 §7 M14b 节）→ 从当前最终源码重建
  NSIS 并复验安装/自检。
- 流程：沿用 `docs/milestones/m11-c-report.md` / `m12-e-report.md` / `m12-f-report.md` /
  `m12-g-report.md`（`npx @tauri-apps/cli@2.12.3 build`，resources 映射
  ffmpeg/ffprobe/licenses）。
- 约束遵守：只做打包与报告；未改源码/测试/`specs/`/`docs/spikes/`；未执行任何 `git`
  写操作（未提交/未推送；本仓库当前尚无提交）；**未跑长测**（电池/sync/soak 由编排者
  统一复跑）。

## 0. 结论

| 项 | 状态 | 关键证据 |
|---|---|---|
| 源码指纹 | **已记录（构建前后一致）** | §1：任务清单共 16 项 SHA-256；构建前 14:59 记录，构建后重算完全相同 |
| NSIS 重建 | **完成，已覆盖 M12-G 00:19 旧包** | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe`；2026-10-18 15:02:01；86,520,827 B（82.51 MiB）；SHA-256 `0AEFC206…F5916` |
| M14/M14b 代码进包 | **已验证** | release 日志重编 `sr-pipeline`/`screen-rec`；exe 含 `keep_segments`；本次构建生成的新 UI 资产（brotli 解压）含 `keep_segments` 与「保留原始分段」 |
| 静默安装 + 自检 | **exit 0** | `%TEMP%\sr_m14p_install`；ffmpeg/ffprobe 来源 `exe-adjacent`；版本 `n8.1.3-18-g46d899d7a0-20261015` |
| 卸载清理 | **完成** | `uninstall.exe /S` exit 0；安装目录已删除 |
| 产物哈希复核 | **一致** | 卸载后安装包 SHA-256 仍为 `0AEFC206…F5916` |
| ffmpeg source offer | **保持精确 commit** | 未改动；SHA-256 `FF1A3F3E…55808`；安装目录内同名文件哈希一致；commit `46d899d7a02bc63237472cc4897258d5ddf274ac` |
| 长测 | **未运行（按约束）** | 仅打包/安装/自检 |

**本报告描述的 15:02 安装包取代 M12-G（00:19）安装包**；M12-G 报告顶部已加指向本报告的说明。

## 1. 源码指纹（构建前记录，构建后复核一致）

记录时间：2026-10-18 14:59（本机）。最晚源码 mtime = **2026-10-18 14:56:02**
（`crates/sr-pipeline/src/route.rs`）；构建开始 14:59:27，新包 mtime = **2026-10-18 15:02:01**。
清单共 **16 项**（本轮较 M12-G 指纹清单新增 `src-tauri/src/settings.rs`、`ui/api.js`）。

| 文件 | SHA-256 | mtime |
|---|---|---|
| `Cargo.lock` | `5268A66731DC03F07653A0A3072B0E1704055742BCAB35724B3617BC66966AAF` | 2026-10-17 19:32:45 |
| `src-tauri/tauri.conf.json` | `95080F11734FA35F3BF28B3B51B01A4D8EA28A1AF742B4FD306FDDE040F819FC` | 2026-10-18 14:40:47 |
| `src-tauri/src/region.rs` | `A9A04F7C82E5E54894BA3ABAB0092BAA619D954DE741A6DE296C5D4310B96D24` | 2026-10-17 21:01:09 |
| `src-tauri/src/lib.rs` | `DEA816D9A960741086F2A4C0B12C16B5EFCE4DDD44E6E3C09B6DC7F928432EB6` | 2026-10-17 19:33:27 |
| `src-tauri/src/commands.rs` | `3006E2AE68E550D38C9B5C7DE141A7A6FF17B2AF6BD0568039BFAE91DCFC318F` | 2026-10-18 14:52:38 |
| `src-tauri/src/settings.rs` | `C6A043AC8147288AF0059CCA2D3E1C83611B5E48FC42C8C7DC1C29C52FFCB2D3` | 2026-10-18 14:33:31 |
| `crates/sr-pipeline/src/route.rs` | `BD56D18E4B9D7DAF41A82F9CFF89379D26D6FF5796019EE1ABC1CED6530AE320` | 2026-10-18 14:56:02 |
| `crates/sr-pipeline/src/naming.rs` | `5BCA89EE4D00C28AF50CB941D276DDD22ADB178B95C082A6C58D85D3C243EB4B` | 2026-10-17 21:00:19 |
| `crates/sr-audio/src/resample.rs` | `4D39004B75D9F891256F99B2A9EE230F95E4B3E99D43A48FEEE7E345021B55C4` | 2026-10-17 21:00:19 |
| `crates/sr-audio/src/wasapi.rs` | `89E9593B22E35B98CF054B85E345645ECB1E4B7B53DC189E4FBC3A9C718E5152` | 2026-10-17 23:42:02 |
| `crates/sr-audio/src/slots.rs` | `44DB9DB8D0985F78196B0550451C1DD89E97667C5247F70DF5445CBBCEFCAC98` | 2026-10-17 23:42:55 |
| `crates/sr-capture/src/wgc.rs` | `90B244810A5BED028E5233CC12B2E3C7A6504C6D543A5BD6ECF1BCC83BA12928` | 2026-10-17 22:28:49 |
| `ui/region.js` | `AAA0CD9AD104E521BDBD2B5DDACB714DB5AB25ED481BE951B739A2D40001D9A4` | 2026-10-17 21:01:13 |
| `ui/app.js` | `7E7B7EC35285A55432797035E74BFCB86FA3447E8CC3326FCE3CD89B90B76D49` | 2026-10-18 14:33:48 |
| `ui/api.js` | `584B845C95881FDD32A9DD7B32CEB15FCF67288AB5070F5610BE3DCB3B18FF5B` | 2026-10-18 14:41:57 |
| `crates/sr-audio/tests/stall_pad_policy.rs` | `60553CAE593D3B80E4074A32647EFF70B6CC90D9C667F5DD7449DDE9F3CE0733` | 2026-10-17 23:42:13 |

说明：

- 构建后对上述 16 项重算 SHA-256，**逐项与构建前相同**。
- 相对 M12-G 指纹，内容变化：`commands.rs`（`E460B758…` → `3006E2AE…`）、
  `route.rs`（`8CEDF4E6…` → `BD56D18E…`）、`ui/app.js`（`4289C4EB…` → `7E7B7EC3…`）；
  新增项 `settings.rs`、`ui/api.js`；`tauri.conf.json` 内容未变（`95080F…`，mtime 14:40:47
  为 M14 真实应用层验证后恢复所致，同 `m14-report.md` §5）。
- `crates/sr-audio/tests/stall_pad_policy.rs` 为 L1 测试文件，不随安装包
  （bundle resources 仍仅 ffmpeg/ffprobe/licenses），但按任务要求计入指纹。
- 本轮源码增量（M14/M14b）：`route.rs` 新增
  `start_routes_with_options` / `start_audio_only_with_options` /
  `cleanup_session_after_finalize`；`settings.rs` 新增 `keep_segments`（默认 false）；
  `commands.rs` 两条启动路径透传；`ui/app.js`/`ui/api.js` 设置项与 mock；见
  `docs/milestones/m14-report.md`。

打包资源（本轮未改）：

| 文件 | SHA-256 | 说明 |
|---|---|---|
| `src-tauri/resources/licenses/ffmpeg-source-offer.txt` | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` | M12-C 修正版：精确 commit + 四种获取方式；3411 B |
| `src-tauri/resources/licenses/FFmpeg-LICENSE.txt` | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | 与构建包 LICENSE.txt 一致 |

## 2. 构建

工具链：Node v24.18.0 / npm 11.16.0；rustc 1.99.0 / cargo 1.99.0；`tauri-cli 2.12.3`（npx）。

```
npx --yes @tauri-apps/cli@2.12.3 build
→ Compiling sr-pipeline v0.1.0 (...crates\sr-pipeline)
→ Compiling screen-rec v0.1.0 (...src-tauri)
→ Finished `release` profile [optimized] target(s) in 31.43s
→ Built application at: ...\target\release\screen-rec.exe
→ Info Patching ...\screen-rec.exe with bundle type information: nsis
→ Running makensis to produce ...\target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe
→ Finished 1 bundle at: ...\Screen_rec_0.1.0_x64-setup.exe (82.51 MiB)
→ TAURI_BUILD_EXIT=0
```

- 构建开始：2026-10-18 14:59:27；结束：**15:02:01**（`m14p-build-nsis.log`）。
- 日志中的 PowerShell `NativeCommandError` 块是 npx 向 stderr 输出一行 Info 时由
  PowerShell 5.1 包装产生，**实际 `TAURI_BUILD_EXIT=0`**（与 M12-G 日志同形）。

完整日志：`docs/milestones/m14p-build-nsis.log`。

产物（覆盖旧包）：

| 项 | 旧包（M12-G，被覆盖） | 新包（M14，本轮） |
|---|---|---|
| 路径 | `target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe` | 同左 |
| 构建时间（mtime） | 2026-10-18 00:19:04 | **2026-10-18 15:02:01** |
| 大小 | 86,582,036 B（82.57 MiB） | **86,520,827 B（82.51 MiB）** |
| SHA-256 | `A99BE175BE4DFEA12AC09473B05CC41786DFA1C495661AD47A6148197F105D29` | **`0AEFC206E8D28808FF443AA7A0A2DA0F8AC8831165142D8AB031166FC32F5916`** |

- release 主程序：`target\release\screen-rec.exe`，14,721,024 B，mtime 15:02:01，
  SHA-256 `CB7C1876687563ABC063094F372CFB12D726CEF5DD8EC2745E99B37456579F4D`。
- 二进制标记（release exe 实测存在）：`SR_DIAG_AV`（门控诊断仍编入）、
  `take_region_screenshot`、`MoveFileExW`、`keep_segments`（M14 设置字段）。
- **UI 进包证据**：本次构建在 14:59:39 生成了新的 `tauri-codegen-assets`：
  `166e50a0…ab04.js`（brotli 解压后 46,266 字符）含 `keep_segments` 与
  「保留原始分段（调试/排查用）」；`71399472…f8c1.js`（解压后 19,307 字符）含
  `keep_segments`。对应 `ui/app.js` 与 `ui/api.js` 的本轮修改，证明 15:02 包内 UI
  不是 M12-G 旧资产。

## 3. 静默安装 + 安装目录自检

```
安装：Screen_rec_0.1.0_x64-setup.exe /S /D=C:\Users\elvis\AppData\Local\Temp\sr_m14p_install
→ installer_exit=0；install_dir_exists=True
```

安装目录清单（实测）：

| 文件 | 大小 | mtime |
|---|---|---|
| `screen-rec.exe` | 14,721,024 B | 2026-10-18 15:00:00 |
| `uninstall.exe` | 79,390 B | 2026-10-18 15:02:18 |
| `ffmpeg\ffmpeg.exe` | 134,092,288 B | 2026-10-15 12:43:48 |
| `ffmpeg\ffprobe.exe` | 133,887,488 B | 2026-10-15 12:43:48 |
| `licenses\FFmpeg-LICENSE.txt` | 7,651 B | 2026-10-15 12:43:48 |
| `licenses\ffmpeg-source-offer.txt` | 3,411 B | 2026-10-17 20:19:36 |

安装后哈希：

| 文件 | SHA-256 | 对照 |
|---|---|---|
| 安装 `screen-rec.exe` | `01B13C160946138C5BD2771178453EC4CDAB4DD7B09AC365DCD202C2FA20062E` | 与 release exe `CB7C18…` 不同（Tauri 打包会改写 exe；M12-G 同现象：`447C60…` vs `673A4C…`）。安装副本大小 14,721,024 B 与 release 相同，且不同于 M12-G 的 14,717,440 B |
| 安装 `ffmpeg\ffmpeg.exe` | `3B57117B384DF98F2B8D01876DF0A596A0B6EBF45F756B30F15D6F65AB8251C5` | = 锁定构建包 |
| 安装 `ffmpeg\ffprobe.exe` | `5B9724EE7A115D5A8458D41BD2E0CE6206847E2B76105E822061A4421322CDE9` | = 锁定构建包 |
| 安装 `licenses\FFmpeg-LICENSE.txt` | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | = 构建包 LICENSE.txt |
| 安装 `licenses\ffmpeg-source-offer.txt` | `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808` | = 源文件（精确 commit 说明随包） |

`--self-check`（安装目录运行；完整输出 `docs/milestones/m14p-self-check.log`，2026-10-18 15:02:43）：

```
[self-check] 搜索根（按优先级）
  env  SCREENREC_FFMPEG_DIR: (未设置)
  tauri resource_dir: (未设置)
  exe 同目录: C:\Users\elvis\AppData\Local\Temp\sr_m14p_install
  开发回退: ...\Screen_rec\crates\sr-core\../../tools/ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin
[self-check] ffmpeg: ...\sr_m14p_install\ffmpeg\ffmpeg.exe（来源：exe-adjacent）
[self-check]   -version: ffmpeg version n8.1.3-18-g46d899d7a0-20261015 Copyright (c) 2000-2026 the FFmpeg developers
[self-check] ffprobe: ...\sr_m14p_install\ffmpeg\ffprobe.exe（来源：exe-adjacent）
[self-check]   -version: ffprobe version n8.1.3-18-g46d899d7a0-20261015 Copyright (c) 2007-2026 the FFmpeg developers
SELF_CHECK_EXIT=0
```

卸载与产物复核：

```
uninstall.exe /S → uninstall_exit=0；dir_exists_after_uninstall=False
安装包 SHA-256（卸载后）→ 0AEFC206E8D28808FF443AA7A0A2DA0F8AC8831165142D8AB031166FC32F5916（与构建时一致）
```

## 4. ffmpeg source offer（保持 M12-C 精确说明）

- 文件未改动：SHA-256 `FF1A3F3E26DB58F339F6D8A9FDBD5747E774B94D799B778D3293E26719455808`，3411 B。
- 精确版本：`FFmpeg n8.1.3-18-g46d899d7a0` = tag `n8.1.3` + 18 commits，commit
  `46d899d7a02bc63237472cc4897258d5ddf274ac`（文件第 6/7/29/34/49 行均保留）。
- 四种获取方式保留：git checkout / GitHub commit 归档 / 上游 tag 对照（注明少 18 commits）/
  BtbN 构建脚本与日期。
- 随包验证：安装目录 `licenses/ffmpeg-source-offer.txt` 哈希 = 源文件哈希（§3）；
  `--self-check` 打印的 ffmpeg/ffprobe 版本与该说明一致。

## 5. 变更文件清单

| 文件 | 变更 |
|---|---|
| `docs/milestones/m14-package-report.md` | 本报告（新增） |
| `docs/milestones/m14p-build-nsis.log` | NSIS 构建完整日志（`TAURI_BUILD_EXIT=0`，新增） |
| `docs/milestones/m14p-self-check.log` | 安装目录 `--self-check` 原始输出（`SELF_CHECK_EXIT=0`，新增） |
| `docs/milestones/m12-g-report.md` | 顶部加一行指向本报告（其 00:19 包已被覆盖） |

未改任何源码/测试/`specs/`/`docs/spikes/`；§1 构建前后指纹一致可证。

> 说明：构建期间工作区另有 `specs/002-recording-modes.md` 于 2026-10-18 14:58:31
> （构建开始前）被改动，**非本任务**（本任务未打开该文件，属另一条线的规格）。`specs/`
> 不参与 bundle 资源映射（resources 仅 ffmpeg/ffprobe/licenses），不影响安装包内容。

## 6. 未决 / 风险（回交编排者）

1. **包内 M14/M14b 行为未做运行时功能复验**：本任务只做“构建 + 安装 + `--self-check`”；
   默认清理/`keep_segments` 的录制语义在 `m14-report.md` §5 由 debug/命令层验证，未在
   *release 安装包* 上重跑录制。若验收要求安装包级证据，建议在安装目录补一次
   keep=false/true 的短录制（约 15s）。
2. **未签名**：NSIS 安装包仍未签名（同 M11-C/M12-G），生产分发需证书与签名步骤。
3. **`resource_dir` 分支未做 GUI 上下文端到端**：`--self-check` 先于 Tauri 启动；本次
   安装实测命中 `exe-adjacent`。既有单测覆盖优先级，GUI 注册见 `src-tauri/src/lib.rs:105–108`。
4. **未在干净机安装**：本机已有 WebView2；安装器对 WebView2 前置的引导未在无运行时
   机器审计（同 M11-C #4 / M12-G #4）。
5. **BtbN 构建脚本未按 commit 锁定**：FFmpeg 源码已精确到 commit；构建脚本仅锁定仓库 +
   构建日期，二进制由构建包 SHA-256 固定（同 M12-G #5）。
6. **长测未运行**：按约束未跑电池/sync/soak；本报告不据此宣称长稳。
7. **安装后 exe 哈希与 release exe 不同**：构建日志显示 Tauri 会向 release 主程序写入
   bundle 类型信息（“Patching … with bundle type information: nsis”）后再打包；安装副本
   与构建目录副本因此不必逐字节相同。安装副本大小 14,721,024 B 与 release 相同、且不同于
   M12-G 的 14,717,440 B；M12-G 亦有同样哈希差异。不是构建污染，安装包哈希与自检结果为准。

## 7. 证据索引（本机实际）

| 证据 | 内容 |
|---|---|
| `docs/milestones/m14p-build-nsis.log` | `npx @tauri-apps/cli@2.12.3 build` 全量输出，15:02:01，exit 0，82.51 MiB |
| `docs/milestones/m14p-self-check.log` | 安装目录 self-check 输出，`SELF_CHECK_EXIT=0`，15:02:43，版本 `n8.1.3-18-g46d899d7a0-20261015` |
| 源码指纹 | §1（构建前 14:59 记录，构建后重算一致；`Get-FileHash <file> -Algorithm SHA256`） |
| UI 进包 | §2（`target\release\build\screen-rec-ee450db…\out\tauri-codegen-assets`，14:59:39 生成；brotli 解压含 `keep_segments`/「保留原始分段」） |
| 安装/卸载 | §3（`installer_exit=0`、`uninstall_exit=0`、目录已删除、产物哈希复核一致） |
| M14/M14b 语义证据 | `docs/milestones/m14-report.md`（§2 实现、§4 回归、§5 真实应用层、§7 M14b） |
