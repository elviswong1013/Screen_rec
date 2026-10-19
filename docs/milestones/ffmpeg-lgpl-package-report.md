# FFmpeg LGPL 整改打包报告（NSIS 安装/自检）

- 日期：2026-10-19
- HEAD：`0d7ac80276b519a0e9761c41882799639706dbac`（`0d7ac80`）
- 范围：仅重建 Windows NSIS 安装包，并做静默安装/自检/卸载验证；未改源码、测试或 specs，未提交、未推送。
- 结论：**内部测试包，不是正式发行版。** 本报告是工程打包证据，不是法律意见。

## 1. 构建

- 构建命令：`npx --yes @tauri-apps/cli@2.12.3 build`
- 构建退出码：`0`
- 构建日志：`Finished 1 bundle at: ...\Screen_rec_0.1.0_x64-setup.exe (80.68 MiB)`
- 构建前 `git status --porcelain`：空；构建后仍为空（报告文件除外）。
- 构建警告：`screen-rec (lib) generated 1 warning`（`src-tauri/src/region.rs:156` 的 `is_closing` 未使用，既有代码，非本次打包引入）。

## 2. 源码指纹（构建前 / 构建后）

构建前后逐文件 SHA-256 完全一致，未发现构建过程改写源文件或 `Cargo.lock`。

| 文件 | 大小 (B) | SHA-256（before = after） |
|---|---:|---|
| `src-tauri/tauri.conf.json` | 1,129 | `09431AD8E63C13E9FE17E9902003EF4390CE05C1DE809A71860EA86B7644248C` |
| `crates/sr-pipeline/src/sidecar.rs` | 19,770 | `6A9ED97697AA2A76BDB78627B64BF89FC20263FBC8B4896A7AA46EA167B504B0` |
| `src-tauri/resources/licenses/FFmpeg-LICENSE.txt` | 7,651 | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` |
| `src-tauri/resources/licenses/GPLv3.txt` | 35,147 | `8CEB4B9EE5ADEDDE47B31E975C1D90C73AD27B6B165A1DCD80C7C545EB65B903` |
| `src-tauri/resources/licenses/ffmpeg-source-offer.txt` | 4,340 | `EE4B61C2D2CD5AAE7D5CFB7D85D5510E8A8E0F964CE14E98E7D13D61B0689CA8` |
| `src-tauri/resources/licenses/ffmpeg-build-info.txt` | 1,734 | `07C48A3155C755216028C7C1E5A7FA604348606F960DDF54D2C00EA16193BD16` |
| `src-tauri/resources/licenses/ffmpeg-build-script.sh` | 1,536 | `DB5CFF6A8FDF29572E0C19F707D431B17BA885F16B5F0443A605448280611273` |
| `Cargo.lock` | 122,672 | `734D84FFF7B4A2F1198BF6058E5CF1694413614F4F23772125183CA91ED83C0C` |

## 3. 安装包与 release 可执行文件

| 项 | 值 |
|---|---|
| 安装包路径 | `target/release/bundle/nsis/Screen_rec_0.1.0_x64-setup.exe` |
| 安装包大小 | `84,599,193` B（80.68 MiB） |
| 安装包 mtime | `2026-10-19 11:55:13` |
| 安装包 SHA-256 | `F3A42C4E304FAECCFBF2410A58A4ACB6D486DB3443E510C4E8B252FEF3B14F2F` |
| 安装包签名 | `NotSigned`（Authenticode 未签名） |
| `target/release/screen-rec.exe` 大小 | `15,326,208` B |
| `target/release/screen-rec.exe` mtime | `2026-10-19 11:55:13` |
| `target/release/screen-rec.exe` SHA-256 | `D7D75F605FF9D9E4272313256A4EA4EE60AAE7D547D34D14A05FBF2475B12F4A` |

## 4. 静默安装

- 安装命令：`Screen_rec_0.1.0_x64-setup.exe /S /D=C:\Users\elvis\AppData\Local\Temp\sr_ffmpegfix_install`
- 安装退出码：`0`（耗时约 8.3 s）
- 安装目录内容：`screen-rec.exe`、`uninstall.exe`、`ffmpeg/`、`licenses/`。

### 4.1 安装目录文件哈希

| 文件 | 大小 (B) | SHA-256 | 期望值 | 结果 |
|---|---:|---|---|---|
| `ffmpeg/ffmpeg.exe` | 130,902,528 | `3091264B90D26398E0366B0E422E16FD76D2ECF85868E4F072209721E7C793F3` | 同左 | 匹配 |
| `ffmpeg/ffprobe.exe` | 130,698,240 | `CB47EFF47532A4F5CE8F3A0FB76FDED41BD401D168C61C7DD187EBBC318B8CAD` | 同左 | 匹配 |
| `licenses/FFmpeg-LICENSE.txt` | 7,651 | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | 源文件 | 匹配 |
| `licenses/GPLv3.txt` | 35,147 | `8CEB4B9EE5ADEDDE47B31E975C1D90C73AD27B6B165A1DCD80C7C545EB65B903` | 源文件 | 匹配 |
| `licenses/ffmpeg-source-offer.txt` | 4,340 | `EE4B61C2D2CD5AAE7D5CFB7D85D5510E8A8E0F964CE14E98E7D13D61B0689CA8` | 源文件 | 匹配 |
| `licenses/ffmpeg-build-info.txt` | 1,734 | `07C48A3155C755216028C7C1E5A7FA604348606F960DDF54D2C00EA16193BD16` | 源文件 | 匹配 |
| `licenses/ffmpeg-build-script.sh` | 1,536 | `DB5CFF6A8FDF29572E0C19F707D431B17BA885F16B5F0443A605448280611273` | 源文件 | 匹配 |
| `screen-rec.exe` | 15,326,208 | `0243AE60ECAE81C3A1B1D91602BD04985409C7B85A18AB3C2B7DA640A972DE9E` | 无（安装载荷） | 记录 |

- 安装载荷 `screen-rec.exe` 与 `target/release/screen-rec.exe` 仅 3 字节不同（偏移 `12,400,034`，`0xBD33A2`）：Tauri bundle-type 标记 `UNK` → `NSS`；其余字节一致。安装/自检以安装载荷为准。

### 4.2 旧 FFmpeg 哈希检查

- 旧安装包（`docs/milestones/m17c-package-report.md`）：
  - `ffmpeg.exe`：`3B57117B384DF98F2B8D01876DF0A596A0B6EBF45F756B30F15D6F65AB8251C5`
  - `ffprobe.exe`：`5B9724EE7A115D5A8458D41BD2E0CE6206847E2B76105E822061A4421322CDE9`
- 本次安装目录实际文件哈希均不等于旧哈希（见 4.1），即安装包载荷已替换为整改后的 FFmpeg。
- 辅助检查：安装包字节流中未出现上述旧哈希字面量（`False`）。

## 5. FFmpeg 配置检查（安装目录）

### 5.1 `ffmpeg -buildconf`（退出码 0）

- `chromaprint`：**不存在**
- `fftw`：**不存在**
- `--enable-gpl`：**不存在**
- `--enable-nonfree`：**不存在**
- `--enable-version3`：**存在**
- 版本串：`ffmpeg version n8.1.3-18-g46d899d7a0-20261019`
- 配置中仍有 `--enable-libopenh264`、`--enable-liblcevc-dec`（见第 8 节剩余合规缺口）。

### 5.2 `ffmpeg -muxers`（退出码 0）

- `chromaprint`：**不存在**（匹配行数 `0`）。

## 6. 安装目录自检

命令：`C:\Users\elvis\AppData\Local\Temp\sr_ffmpegfix_install\screen-rec.exe --self-check`

关键输出（规范化显示；控制台按系统代码页显示中文时部分标签有乱码，ASCII 版本行完整）：

```text
[self-check] 搜索根（按优先级）
  env SCREENREC_FFMPEG_DIR: (未设置)
  tauri resource_dir: (未设置)
  exe 同目录: C:\Users\elvis\AppData\Local\Temp\sr_ffmpegfix_install
  开发回退: ...\tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin
[self-check] ffmpeg: C:\Users\elvis\AppData\Local\Temp\sr_ffmpegfix_install\ffmpeg\ffmpeg.exe（来源：exe-adjacent）
[self-check]   -version: ffmpeg version n8.1.3-18-g46d899d7a0-20261019 Copyright (c) 2000-2026 the FFmpeg developers
[self-check] ffprobe: C:\Users\elvis\AppData\Local\Temp\sr_ffmpegfix_install\ffmpeg\ffprobe.exe（来源：exe-adjacent）
[self-check]   -version: ffprobe version n8.1.3-18-g46d899d7a0-20261019 Copyright (c) 2007-2026 the FFmpeg developers
```

- 自检退出码：`0`
- ffmpeg/ffprobe 均命中 `exe-adjacent`（安装目录自带），版本串含 `n8.1.3-18-g46d899d7a0-20261019`。

## 7. 卸载

- 卸载命令：`uninstall.exe /S`
- 卸载退出码：`0`（耗时约 1.2 s）
- 安装目录 `C:\Users\elvis\AppData\Local\Temp\sr_ffmpegfix_install`：已删除（`INSTALL_DIR_EXISTS_AFTER=False`）。
- 卸载后残留进程检查：无 `screen-rec` / `ffmpeg` / `ffprobe` 进程。
- 本次未重跑长媒体测试；源码测试、容器/区域、90 s sync 已由既有验证覆盖。

## 8. 未签名、未审计与剩余合规缺口

- 安装包 **未签名**（Authenticode `NotSigned`）；未做安全审计、恶意代码扫描或第三方签名验证。
- Rust 第三方依赖 notices 仍未补齐。
- `libopenh264`、`liblcevc-dec` 等静态组件仍需逐项通知/源码清单/许可文本核对。
- H.264/AAC 专利许可适用性未清权。
- EULA、隐私政策未定稿。
- 本报告仅为工程打包与安装验证记录；最终合规结论需律师/许可方确认。

**再次声明：本安装包为内部测试包，不是正式发行版。**
