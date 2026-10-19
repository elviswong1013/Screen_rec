# 随包 FFmpeg/ffprobe 组件、许可证与源码状态清单（部分清单）

> **状态：工程草案 / 部分清单（partial）。**
> 本文件只整理当前随包 FFmpeg/ffprobe 二进制中**可本地核验**的事实、逐组件线索与材料缺口；
> 不是法律意见，不构成合规结论，也不表示可以正式发行。
> 日期：2026-10-19；代码基线：仓库 `HEAD 24575f2`；对象：`tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin/ffmpeg.exe` 与 `ffprobe.exe`（以及 `src-tauri/tauri.conf.json` 实际打包的两个同名文件）。

## 0. 结论摘要

1. 当前二进制版本为 **`n8.1.3-18-g46d899d7a0-20261019`**，对应 FFmpeg commit **`46d899d7a02bc63237472cc4897258d5ddf274ac`**。按构建脚本与 source offer 的记载，定制构建只从 BtbN LGPL 配置移除了 `--enable-chromaprint`（及其 FFTW 依赖）；当前 `-buildconf` 中 `chromaprint`、`fftw`、`--enable-gpl`、`--enable-nonfree` 均为 0 命中，`--enable-version3` 存在。
2. 本清单按 `-buildconf` 列出 **63 项直接组件/构建特性**（不含 `--enable-version3`，含 `--enable-pthreads` 与 `--extra-libs=-lgomp`），并另列 **11 项已知传递依赖示例**。BtbN 静态依赖闭包**尚未逐项解析**，清单不完整。
3. **许可证本地已确认的只有 FFmpeg 本体 1 项**（`FFmpeg-LICENSE.txt` + `-L` + `--enable-version3`）。其余 **74 项组件行的许可证均为 `unknown`/`NOASSERTION`**；表中 `L3[E]` / `L3[?]` 只是 Lane3 研究报告引用的上游线索，不是本地许可证原文，不能当作已确认结论。
4. 高风险项：`libopenh264`（BSD-2 线索、无专利授权）、`liblcevc-dec`（Clear BSD 线索、明示不授予专利）、若干 LGPL 静态组件的重链接/组合作品问题、以及 BtbN 精确构建脚本/patches/依赖源码缺失。正式发行前仍需律师与上游/BtbN 材料。
5. 许可证文本层面，随包只有 FFmpeg LGPLv3 文本与 GPLv3 文本；**没有**逐组件 LICENSE/COPYING/NOTICE、没有依赖源码归档、没有 BtbN `scripts.d` 快照、没有 patches 清单、没有第三方 notices。
6. `ffmpeg-build-info.txt` 记录的基础镜像是可变的 `:latest`，而 `ffmpeg-source-offer.txt` 记录的是 digest `sha256:a0ac6b9b3b89cf1f3f0b6cb3556dceb6b6acec22946b4d8f79af516099b920fa`；两者不一致，SBOM 同时记录该 digest 并标注此项为文档记载、未在本机验证。

## 1. 二进制身份与构建事实

| 项 | 值 | 证据等级/来源 |
|---|---|---|
| `ffmpeg.exe` 版本 | `n8.1.3-18-g46d899d7a0-20261019` | 本机 `-version` |
| `ffprobe.exe` 版本 | `n8.1.3-18-g46d899d7a0-20261019` | 本机 `-version` |
| FFmpeg commit | `46d899d7a02bc63237472cc4897258d5ddf274ac` | `ffmpeg-source-offer.txt`、构建脚本、`git describe` 预期 |
| 构建日期 / `--extra-version` | `2026-10-19` / `20261019` | 本机 `-version`、`-buildconf` |
| `ffmpeg.exe` SHA-256 | `3091264B90D26398E0366B0E422E16FD76D2ECF85868E4F072209721E7C793F3` | 本机 `Get-FileHash` |
| `ffprobe.exe` SHA-256 | `CB47EFF47532A4F5CE8F3A0FB76FDED41BD401D168C61C7DD187EBBC318B8CAD` | 本机 `Get-FileHash` |
| 定制构建包 SHA-256 | `015EB37E45AFB52901DFF77562819A524DEEED07DDEFADDE766ABB14339BC502` | `ffmpeg-lgpl-remediation.md` |
| 安装包 SHA-256 | `F3A42C4E304FAECCFBF2410A58A4ACB6D486DB3443E510C4E8B252FEF3B14F2F` | `ffmpeg-lgpl-package-report.md` |
| 构建基础镜像（文档记载） | `ghcr.io/btbn/ffmpeg-builds/win64-lgpl-8.1@sha256:a0ac6b9b3b89cf1f3f0b6cb3556dceb6b6acec22946b4d8f79af516099b920fa` | `ffmpeg-source-offer.txt`；`ffmpeg-build-info.txt` 仅记 `:latest` |
| BtbN 构建脚本参考提交 | `63ac3265b0789ea16c7b1f8351cd2ad195a28722` | `ffmpeg-source-offer.txt`；**不是**该镜像构建输入的证明 |
| 编译器 | `gcc 16.2.0 (crosstool-NG 1.29.0.7_b1a94f6)` | 本机 `-version` |
| LGPLv3 文本 | `FFmpeg-LICENSE.txt`，SHA-256 `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` | 本机哈希；与构建包 `LICENSE.txt` 同哈希 |
| GPLv3 文本 | `GPLv3.txt`，SHA-256 `8CEB4B9EE5ADEDDE47B31E975C1D90C73AD27B6B165A1DCD80C7C545EB65B903` | 本机哈希 |
| 源码获取说明 | `ffmpeg-source-offer.txt`，SHA-256 `EE4B61C2D2CD5AAE7D5CFB7D85D5510E8A8E0F964CE14E98E7D13D61B0689CA8` | 本机哈希 |
| 构建信息 | `ffmpeg-build-info.txt`，SHA-256 `07C48A3155C755216028C7C1E5A7FA604348606F960DDF54D2C00EA16193BD16` | 本机哈希 |
| 构建脚本 | `ffmpeg-build-script.sh`，SHA-256 `DB5CFF6A8FDF29572E0C19F707D431B17BA885F16B5F0443A605448280611273` | 本机哈希 |

补充事实：

- 构建脚本只做 `CONFIGURE_FLAGS="${FF_CONFIGURE/--enable-chromaprint/}"` 的替换；本地脚本未发现/未应用任何 FFmpeg patch。**但**未取得镜像内 BtbN `scripts.d` 原文与 patch 清单，不能仅凭本地脚本证明“与镜像原始配置逐字只差这一项”或“BtbN 未打 patch”。
- `ffmpeg-build-info.txt` 的 `Base image` 行是 `ghcr.io/btbn/ffmpeg-builds/win64-lgpl-8.1:latest`；`ffmpeg-source-offer.txt` 则写 digest。两者只能作为文档记载，需 BtbN 确认 digest 对应的脚本/依赖版本。
- 当前 NSIS 安装包只收 `ffmpeg/ffmpeg.exe`、`ffmpeg/ffprobe.exe` 与 `licenses/` 下 5 个文本；构建 zip 里的 `ffplay.exe` 不在安装包内。

## 2. 本机自检：构建配置与已移除项

命令（在仓库根执行）：

```powershell
tools\ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin\ffmpeg.exe -hide_banner -version
tools\ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin\ffmpeg.exe -hide_banner -buildconf
tools\ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin\ffmpeg.exe -hide_banner -muxers
tools\ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin\ffmpeg.exe -hide_banner -L
tools\ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin\ffmpeg.exe -hide_banner -encoders
tools\ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin\ffmpeg.exe -hide_banner -decoders
tools\ffmpeg\ffmpeg-n8.1-latest-win64-lgpl-8.1\bin\ffmpeg.exe -hide_banner -filters
```

本机观察结果：

| 检查 | 结果 |
|---|---|
| `-buildconf` 中 `chromaprint` | **0** 命中 |
| `-buildconf` 中 `fftw` | **0** 命中 |
| `-buildconf` 中 `--enable-gpl` | **0** 命中 |
| `-buildconf` 中 `--enable-nonfree` | **0** 命中 |
| `-buildconf` 中 `--enable-version3` | **1** 命中 |
| `-buildconf` 中 `--enable-shared` | **0** 命中 |
| `-buildconf` 中 `--pkg-config-flags=--static` | 存在 |
| `-muxers` 中 `chromaprint` | **0** 命中 |
| `-L` 输出 | LGPL “version 3 … or later” |
| `--enable-*` 总数 | **63**（含 `--enable-version3`；组件/特性相关 62 项） |
| `--disable-libx264` / `libx265` / `libxavs2` / `libxvid` | 均存在禁用开关；`-encoders` 中对应 GPL 编码器 0 命中 |
| `-encoders` 中 `libopenh264` | 存在（另有 `libopenh264` decoder） |
| `-filters` 中 `lcevc` / `libplacebo` / `libvmaf` | 均存在 |
| `-protocols` 中 `srt` / `rist` / `zmq` | 均存在 |

## 3. 已移除、不应再出现的组件：chromaprint / FFTW

### 3.1 事实

- 旧 BtbN `win64-lgpl-8.1` 构建含 `--enable-chromaprint`；BtbN 的 `50-chromaprint.sh` 依赖 FFTW3，FFTW `COPYING` 为 GPLv2+（见 Lane2/Lane3 报告与 `ffmpeg-lgpl-remediation.md`）。
- 当前定制构建只移除 `--enable-chromaprint`；FFTW 不再由 chromaprint 拉入。
- 当前二进制 `-buildconf` 中 `chromaprint` 与 `fftw` 均 0 命中；`-muxers` 中 `chromaprint` 0 命中。这是**本机观察**，不是对二进制目标代码的符号级扫描。
- **结论：chromaprint 与 FFTW 已移除，不应再出现在当前构建、安装包或 SBOM 的组件清单中。** 若未来构建恢复 `--enable-chromaprint`，必须重新生成清单、SBOM 与合规评估。

### 3.2 回归风险

- 若后续重新使用未改动的 BtbN `win64-lgpl-8.1` 镜像或恢复 `--enable-chromaprint`，GPLv2+ 的 FFTW 可能重新进入静态链接。
- 仓库已有审计：`crates/sr-pipeline/src/sidecar.rs` 的 `forbidden_muxer_markers = ["chromaprint"]`，以及 `crates/sr-pipeline/tests/license_and_build_audit.rs` 对 `-buildconf`/`-muxers` 的断言。该断言覆盖当前回归点，但不替代逐组件许可证审计。
- 本清单与 SBOM **不把 chromaprint/FFTW 列为当前组件**；若未来构建重新引入，必须重新生成清单并重新评估。

## 4. 组件清单（以实际 `-buildconf` 为准）

### 4.1 阅读说明

- **静态进入**：`是（静态配置）` 的依据是 `--pkg-config-flags=--static`、无 `--enable-shared`、BtbN static 安装、安装包内无第三方 DLL；这是**配置层推断**，不是逐符号/逐目标文件核验。`待核` 表示该组件可能是头文件、运行时加载或静态链接，未在本机确认。
- **许可证（本地证据）**：只有本地随包/仓库中的许可证原文才算“已确认”。`L3[E]` = Lane3 报告引用的上游许可证原文（2026-10-19 抓取）；`L3[?]` = Lane3 报告标注未取证/预期；两者都**不是**本地许可证原文，因此许可证列一律以 `unknown` 开头。
- **通知/源码状态（需上游）**：`L` = LICENSE/COPYING/NOTICE/版权声明；`V` = 精确版本/commit；`S` = 源码归档 + SHA-256；`P` = patches/build scripts/build log。`—` 表示该项不适用（如 OS 组件）。
- **已确认**：本地许可证/通知文本已确认。**存在性**由 `-buildconf`/`-version` 另行确认，不混用。
- **待上游**：至少缺 L/V/S/P 之一，需 BtbN 或组件上游提供。
- **待法律**：本清单识别出的具体法律问题代码：`L` = LGPL 静态链接/重链接/Combined Work 适用性；`P` = 专利/编解码许可；`D` = 双许可分支选择；`M` = MPL 覆盖文件源码；`E` = GCC Runtime Library Exception；`O` = 其他通知/offer/安装信息。`—` 表示本轮未识别出独立法律问题，不等于已清权。
- **Screen_rec 调用**：基于当前 `crates/` 静态检索与生产路径 argv；`否` 表示未发现直接调用，不表示 FFmpeg 内部绝不会走到该组件。

### 4.1.1 证据范围与限制

- 证据口径：Lane2/Lane3 报告基于旧构建（Lane3 记录版本为 `20261015`）；本清单的组件集合与版本以当前 `20261019` 二进制的 `-buildconf` 为准。Lane3 的 chromaprint/FFTW 阻断结论针对旧构建，当前已移除；其余许可证线索仅作待上游核实线索。
- 本清单的直接组件集合来自当前二进制的 `-buildconf`，不是从 BtbN 仓库推断。
- `-buildconf` 只列 FFmpeg 直接开关，**不列传递依赖**；Lane3 已证实 `librsvg → glib/cairo/pango`、`libbluray → libudfread`、`librist → mbedTLS`、`libass → libunibreak`、`libjxl → brotli/highway/skcms` 等示例，但完整闭包仍未解析。
- `Screen_rec` 生产路径的编码器只有 `h264_nvenc → h264_qsv → h264_amf → h264_mf` 与原生 `aac`；容器为 Matroska 分段、收尾 remux 到 Matroska/MP4、合并为 Matroska；`ffprobe` 只读 `format=duration`、`format=format_name,duration`、`stream=codec_type` 与 `packet=stream_index,pts_time` 等元数据。未发现 `libopenh264`、`lcevc`、`libplacebo`、`libaom`、`libvpx`、`libsvtav1`、`libdav1d`、`libmp3lame`、`libbluray` 等被生产代码直接调用。`h264_mf` 走 Windows Media Foundation，不是上述 buildconf 外部组件；其专利/OS 责任在第 7 节单独列出。
- 源码位置：`crates/sr-pipeline/src/sidecar.rs:82`（`-c:v`）、`:92`（`-c:a aac`）、`:96-109`（segment/matroska）、`:134`（D4 链）；`crates/sr-pipeline/src/probe.rs:93`（D4 链）；`crates/sr-pipeline/src/finalize.rs:400-431`（Matroska/MP4 流拷贝）、`:123-131`/`:167-175`（ffprobe）；`crates/sr-merge/src/merge.rs:289`（合并为 Matroska）、`:297-305`（ffprobe）。

## 4.2 FFmpeg 本体

| # | 组件（buildconf 开关） | 用途 | 静态进入 | 许可证（本地证据） | 通知/源码状态（需上游） | Screen_rec 调用 | 已确认 | 待上游 | 待法律 | 备注 |
|---:|---|---|---|---|---|---|:---:|:---:|:---:|---|
| 1 | FFmpeg 本体（libav*）<br>`FFmpeg core` | 解复用、解码、编码、封装、滤镜、协议核心 | 本体（构建为静态 exe） | LGPL-3.0-or-later（本地：FFmpeg-LICENSE.txt + `-L` + `--enable-version3`） | L 已有 LGPLv3 文本；V 已确认；S 仅 commit URL、未归档；P BtbN 脚本/patches 未钉 | 是（录制/收尾/合并/探测） | Y | Y | L,O | GPLv3 文本随包；源码归档、书面 offer、BtbN 精确材料仍缺 |

## 4.3 高风险/编解码器组件

| # | 组件（buildconf 开关） | 用途 | 静态进入 | 许可证（本地证据） | 通知/源码状态（需上游） | Screen_rec 调用 | 已确认 | 待上游 | 待法律 | 备注 |
|---:|---|---|---|---|---|---|:---:|:---:|:---:|---|
| 2 | libopenh264<br>`--enable-libopenh264` | H.264 编码/解码（OpenH264） | 是（静态配置；编码器和解码器均在 -encoders/-decoders 出现） | unknown（L3[E]: BSD-2；无专利授权） | L,V,S,P；另需 Cisco 专利/royalty 适用性说明 | 否（应用未调用 libopenh264） | — | Y | P | 高风险：BSD-2 文本/通知/源码均未随包；自编译二进制的专利安排待法律 |
| 3 | liblcevc-dec<br>`--enable-liblcevc-dec` | LCEVC 解码/滤镜（`lcevc` filter） | 是（静态配置；`lcevc` filter 存在） | unknown（L3[E]: Clear BSD，明示不授予专利） | L,V,S,P；另需 V-Nova/专利池说明 | 否（应用未调用 LCEVC） | — | Y | P | 高风险：Clear BSD 文本/通知/源码均未随包；专利池未评估 |
| 4 | libmp3lame<br>`--enable-libmp3lame` | MP3 编码 | 是（静态配置） | unknown（L3[E]: LGPL-2.0 / Library GPL v2） | L,V,S,P | 否 | — | Y | L | LGPL 静态组件；重链接材料未提供 |
| 5 | libbluray<br>`--enable-libbluray` | Blu-ray 解复用 | 是（静态配置） | unknown（L3[E]: LGPL-2.1） | L,V,S,P；另需 libudfread | 否 | — | Y | L | 传递依赖 libudfread；重链接材料未提供 |
| 6 | libplacebo<br>`--enable-libplacebo` | GPU 视频滤镜（`libplacebo` filter） | 是（静态配置） | unknown（L3[E]: LGPL-2.1） | L,V,S,P | 否 | — | Y | L | 依赖 Vulkan/shaderc/lcms2；重链接材料未提供 |
| 7 | libaom<br>`--enable-libaom` | AV1 编码/解码 | 是（静态配置） | unknown（L3[E]: BSD-2 + AOM 专利授权） | L,V,S,P；LICENSE/NOTICE（若有） | 否 | — | Y | P | AV1 专利授权范围待律师 |
| 8 | libvpx<br>`--enable-libvpx` | VP8/VP9 编码/解码 | 是（静态配置） | unknown（L3[E]: BSD-3） | L,V,S,P | 否 | — | Y | P | VP8/VP9 专利授权范围待律师 |
| 9 | libsvtav1<br>`--enable-libsvtav1` | AV1 编码 | 是（静态配置） | unknown（L3[E]: BSD-3 Clear） | L,V,S,P | 否 | — | Y | P | AV1 专利授权范围待律师 |
| 10 | libdav1d<br>`--enable-libdav1d` | AV1 解码 | 是（静态配置；`libdav1d` decoder 存在） | unknown（L3[E]: BSD-2） | L,V,S,P | 否（ffprobe 默认不解码） | — | Y | P | AV1 专利授权范围待律师 |
| 11 | librav1e<br>`--enable-librav1e` | AV1 编码 | 是（静态配置；Rust 构建） | unknown（L3[E]: BSD-2） | L,V,S,P；另需 Rust crate 闭包 | 否 | — | Y | P | Rust crate 依赖未展开；AV1 专利待律师 |
| 12 | libvvenc<br>`--enable-libvvenc` | VVC/H.266 编码 | 是（静态配置） | unknown（L3[?]: 预期 BSD-3，未取原文） | L,V,S,P | 否 | — | Y | P | VVC 专利池未评估；许可证原文未取 |
| 13 | libkvazaar<br>`--enable-libkvazaar` | HEVC/H.265 编码 | 是（静态配置） | unknown（L3[E]: BSD-3） | L,V,S,P | 否 | — | Y | P | HEVC 专利池未评估 |
| 14 | libuavs3d<br>`--enable-libuavs3d` | AVS3 解码 | 是（静态配置；`libuavs3d` decoder 存在） | unknown（L3[E]: BSD-3 类） | L,V,S,P | 否 | — | Y | P | AVS3 专利/标准许可未评估 |
| 15 | liboapv<br>`--enable-liboapv` | APV 编码/解码 | 是（静态配置；`apv` decoder 存在） | unknown（L3[E]: BSD-3 类） | L,V,S,P | 否 | — | Y | P | APV 专利/标准许可未评估 |
| 16 | libjxl<br>`--enable-libjxl` | JPEG XL 编码/解码 | 是（静态配置） | unknown（L3[E]: BSD-3；传递 brotli MIT；highway/skcms 未取证） | L,V,S,P；另需 brotli/highway/skcms | 否 | — | Y | — | 传递依赖未逐项取证 |
| 17 | libvmaf<br>`--enable-libvmaf` | 视频质量评估（VMAF） | 是（静态配置；`libvmaf` filter 存在） | unknown（L3[E]: BSD-2-Clause-Patent） | L,V,S,P；另需 VMAF 模型文件及许可 | 否 | — | Y | — | 模型文件是否随构建/分发未确认 |

**高风险单列：`libopenh264` 与 `liblcevc-dec`**

- 两者均**不在** Screen_rec 当前生产调用路径中，但都**静态进入**随包 `ffmpeg.exe`（配置层证据：`--pkg-config-flags=--static`、无 DLL；`-encoders`/`-filters` 可观察到 `libopenh264` 与 `lcevc`）。
- `libopenh264`：Lane3 报告引用的上游 `LICENSE` 为 BSD-2；该许可证**不包含专利许可**。Cisco 的 royalty-free 安排通常针对 Cisco 分发的二进制；BtbN 从 `cisco/openh264.git` 源码自编译的二进制是否覆盖 MPEG-LA/H.264 专利费，**需法律确认**。当前随包没有 BSD-2 文本、版权通知或源码。
- `liblcevc-dec`：Lane3 报告引用的 `LCEVCdec/LICENSE.md` 为 Clear BSD，并**明示不授予任何专利许可**；LCEVC 有专利池。当前随包没有 Clear BSD 文本、版权通知或源码。
- 可选降低风险的产品/工程方向（需产品与律师决定，不是本清单的结论）：改用 `--disable-libopenh264`/`--disable-liblcevc-dec` 的构建，或取得相应专利/再分发意见并补齐通知与源码。两者均未被应用调用，禁用不会改变当前录制链路，但仍需回归测试。

## 4.4 其余 buildconf 直接组件/特性

| # | 组件（buildconf 开关） | 用途 | 静态进入 | 许可证（本地证据） | 通知/源码状态（需上游） | Screen_rec 调用 | 已确认 | 待上游 | 待法律 | 备注 |
|---:|---|---|---|---|---|---|:---:|:---:|:---:|---|
| 18 | amf<br>`--enable-amf` | AMD AMF 硬件编码接口 | 否（头文件 + 运行时加载 AMF 运行时） | unknown（L3[?]: 预期 MIT，未取原文） | L,V,P | 间接（h264_amf） | — | Y | — | AMF 运行时由 AMD 驱动提供 |
| 19 | cuda-llvm<br>`--enable-cuda-llvm` | CUDA 编译支持（构建期） | 否（编译期） | unknown（L3[?]: NVIDIA EULA） | L,V,P | 否（仅构建） | — | Y | — | 构建工具链 EULA/再分发需单独审查 |
| 20 | ffnvcodec<br>`--enable-ffnvcodec` | NVENC/NVDEC 头文件 | 否（头文件） | unknown（L3[?]: 预期 MIT，未取原文） | L,V,P | 间接（h264_nvenc） | — | Y | — | NVENC 运行时由 NVIDIA 驱动提供 |
| 21 | fontconfig<br>`--enable-fontconfig` | 字体配置 | 是（静态配置） | unknown（L3[E]: MIT 类） | L,V,S,P | 否 | — | Y | — | 无字幕/字体渲染调用 |
| 22 | gmp<br>`--enable-gmp` | 大数运算 | 是（静态配置） | unknown（L3[E]: GPLv3/LGPLv3 双许可；构建选项未记录） | L,V,S,P；另需记录所选分支 | 否 | — | Y | D,L | 若选 GPLv3 分支，组合作品义务与 LGPL 路径不同 |
| 23 | libiconv<br>`--enable-iconv` | 字符集转换 | 是（静态配置） | unknown（L3[?]: LGPL-2.1+ 线索，未取原文） | L,V,S,P | 否（FFmpeg 内部路径） | — | Y | L | 若为 LGPL，静态重链接材料未提供 |
| 24 | lcms2<br>`--enable-lcms2` | 色彩管理 | 是（静态配置） | unknown（L3[E]: MIT） | L,V,S,P | 否 | — | Y | — | 无直接调用 |
| 25 | libaribb24<br>`--enable-libaribb24` | ARIB 字幕解码 | 是（静态配置；decoder 存在） | unknown（L3[?]: 预期 LGPL，未取证） | L,V,S,P | 否 | — | Y | L | 许可证原文未取 |
| 26 | libaribcaption<br>`--enable-libaribcaption` | ARIB 字幕解码 | 是（静态配置；decoder 存在） | unknown（L3[?]: 预期 MIT，未取证） | L,V,S,P | 否 | — | Y | — | 许可证原文未取 |
| 27 | libass<br>`--enable-libass` | ASS/SSA 字幕渲染 | 是（静态配置；`ass` filter 存在） | unknown（L3[E]: ISC） | L,V,S,P；另需 libunibreak | 否（无字幕渲染） | — | Y | — | 传递 libunibreak |
| 28 | libfreetype<br>`--enable-libfreetype` | 字体渲染 | 是（静态配置） | unknown（L3[E]: FTL 或 GPLv2 双许可；构建选项未记录） | L,V,S,P；另需记录所选分支 | 否 | — | Y | D | 若选 GPLv2 分支，义务与 FTL 不同 |
| 29 | libfribidi<br>`--enable-libfribidi` | 双向文本（BiDi） | 是（静态配置） | unknown（L3[E]: LGPL-2.1） | L,V,S,P | 否 | — | Y | L | LGPL 静态组件；重链接材料未提供 |
| 30 | libgme<br>`--enable-libgme` | 游戏音乐解复用 | 是（静态配置；demuxer 存在） | unknown（L3[?]: 预期 LGPL，未取证） | L,V,S,P | 否 | — | Y | L | 许可证原文未取 |
| 31 | libharfbuzz<br>`--enable-libharfbuzz` | 文本整形 | 是（静态配置） | unknown（L3[E]: Old MIT） | L,V,S,P | 否 | — | Y | — | 无直接调用 |
| 32 | libopencore-amrnb<br>`--enable-libopencore-amrnb` | AMR-NB 编码/解码 | 是（静态配置） | unknown（L3[E]: Apache-2.0） | L,V,S,P；NOTICE（若有） | 否 | — | Y | — | Apache-2.0 NOTICE 未随包 |
| 33 | libopencore-amrwb<br>`--enable-libopencore-amrwb` | AMR-WB 编码/解码 | 是（静态配置） | unknown（L3[E]: Apache-2.0） | L,V,S,P；NOTICE（若有） | 否 | — | Y | — | Apache-2.0 NOTICE 未随包 |
| 34 | libopenjpeg<br>`--enable-libopenjpeg` | JPEG 2000 编码/解码 | 是（静态配置） | unknown（L3[E]: BSD-2） | L,V,S,P | 否 | — | Y | — | 无直接调用 |
| 35 | libopenmpt<br>`--enable-libopenmpt` | Tracker 音乐解复用 | 是（静态配置；demuxer 存在） | unknown（L3[E]: BSD-3） | L,V,S,P | 否 | — | Y | — | 无直接调用 |
| 36 | libopus<br>`--enable-libopus` | Opus 编码/解码 | 是（静态配置） | unknown（L3[E]: BSD-3） | L,V,S,P | 否 | — | Y | — | 无直接调用 |
| 37 | librist<br>`--enable-librist` | RIST 协议 | 是（静态配置；`rist` protocol 存在） | unknown（L3[?]: librist 未取证；传递 mbedTLS Apache-2.0 或 GPL-2.0+） | L,V,S,P；另需 mbedTLS 分支记录 | 否 | — | Y | D | mbedTLS 双许可分支未记录 |
| 38 | librsvg<br>`--enable-librsvg` | SVG 解码 | 是（静态配置） | unknown（L3[E]: LGPL-2.1+；传递 glib/cairo/pango） | L,V,S,P；另需 glib/cairo/pango + Rust crate 闭包 | 否 | — | Y | L | Rust crate 闭包与 glib/cairo/pango 未逐项 |
| 39 | libshaderc<br>`--enable-libshaderc` | GLSL→SPIR-V 编译 | 是（静态配置） | unknown（L3[E]: Apache-2.0） | L,V,S,P；NOTICE（若有） | 否 | — | Y | — | Apache-2.0 NOTICE 未随包 |
| 40 | libsnappy<br>`--enable-libsnappy` | Snappy 压缩（HAP 等） | 是（静态配置） | unknown（L3[E]: BSD-3） | L,V,S,P | 否 | — | Y | — | 无直接调用 |
| 41 | libsoxr<br>`--enable-libsoxr` | 重采样 | 是（静态配置） | unknown（L3[E]: LGPL-2.1） | L,V,S,P | 否 | — | Y | L | LGPL 静态组件；重链接材料未提供 |
| 42 | libsrt<br>`--enable-libsrt` | SRT 协议 | 是（静态配置；`srt` protocol 存在） | unknown（L3[E]: MPL-2.0） | L,V,S,P；MPL 覆盖文件源码 | 否 | — | Y | M | MPL 文件级源码获取说明未提供 |
| 43 | libssh<br>`--enable-libssh` | SSH/SFTP 协议 | 是（静态配置） | unknown（L3 未覆盖该项） | L,V,S,P | 否 | — | Y | — | Lane3 未给出许可证线索；先补上游材料 |
| 44 | libtheora<br>`--enable-libtheora` | Theora 编码/解码 | 是（静态配置） | unknown（L3[E]: BSD-3） | L,V,S,P | 否 | — | Y | — | 无直接调用 |
| 45 | libtwolame<br>`--enable-libtwolame` | MP2 编码 | 是（静态配置） | unknown（L3[E]: LGPL-2.1） | L,V,S,P | 否 | — | Y | L | LGPL 静态组件；重链接材料未提供 |
| 46 | libvorbis<br>`--enable-libvorbis` | Vorbis 编码/解码 | 是（静态配置） | unknown（L3[E]: BSD-3） | L,V,S,P | 否 | — | Y | — | 无直接调用 |
| 47 | libvpl<br>`--enable-libvpl` | Intel oneVPL / QSV 接口 | 待核（dispatcher 运行时/静态未确认） | unknown（L3[E]: MIT） | L,V,S,P | 间接（h264_qsv） | — | Y | — | dispatcher 分发形态待核 |
| 48 | libwebp<br>`--enable-libwebp` | WebP 编码/解码 | 是（静态配置） | unknown（L3[E]: BSD-3） | L,V,S,P | 否 | — | Y | — | 无直接调用 |
| 49 | libxml2<br>`--enable-libxml2` | XML 解析 | 是（静态配置） | unknown（L3[E]: MIT） | L,V,S,P | 否 | — | Y | — | 无直接调用 |
| 50 | libzimg<br>`--enable-libzimg` | 缩放/色彩空间转换（zscale） | 是（静态配置；`zscale` filter 存在） | unknown（L3[E]: WTFPL-2） | L,V,S,P | 否 | — | Y | — | 无直接调用 |
| 51 | libzmq<br>`--enable-libzmq` | ZeroMQ 协议 | 是（静态配置；`zmq` protocol 存在） | unknown（L3[E]: MPL-2.0） | L,V,S,P；MPL 覆盖文件源码 | 否 | — | Y | M | MPL 文件级源码获取说明未提供 |
| 52 | libzvbi<br>`--enable-libzvbi` | 图文电视（teletext）解码 | 是（静态配置；decoder 存在） | unknown（L3[?]: 预期 LGPL，未取证） | L,V,S,P | 否 | — | Y | L | 许可证原文未取 |
| 53 | lv2<br>`--enable-lv2` | LV2 音频插件 | 是（静态配置；`lv2` filter 存在） | unknown（L3[E]: ISC 类） | L,V,S,P | 否 | — | Y | — | 无直接调用 |
| 54 | lzma<br>`--enable-lzma` | LZMA 压缩 | 是（静态配置） | unknown（L3[?]: 公有领域线索，未取原文） | L,V,S,P | 否（FFmpeg 内部路径） | — | Y | — | 许可证原文未取 |
| 55 | openal<br>`--enable-openal` | OpenAL 音频设备 | 是（静态配置；`openal` device 存在） | unknown（L3[E]: LGPL-2.0 / Library GPL v2） | L,V,S,P | 否 | — | Y | L | LGPL 静态组件；重链接材料未提供 |
| 56 | opencl<br>`--enable-opencl` | OpenCL ICD / 滤镜 | 待核（ICD loader/运行时） | unknown（L3[E]: Apache-2.0） | L,V,S,P；NOTICE（若有） | 否 | — | Y | — | 无直接调用 |
| 57 | schannel<br>`--enable-schannel` | Windows TLS | 否（OS API） | unknown（Windows OS 组件，不随包） | —（OS 组件） | 否 | — | — | — | 不随安装包分发 |
| 58 | sdl2<br>`--enable-sdl2` | ffplay 输出（当前安装包不含 ffplay） | 仅 ffplay（不随安装包） | unknown（L3[E]: zlib） | L,V,S,P（仅当分发 ffplay） | 否（ffplay 未随包） | — | Y | — | 构建 zip 含 ffplay，但 NSIS 安装包只收 ffmpeg/ffprobe |
| 59 | vaapi<br>`--enable-vaapi` | VA-API 硬件加速 | 待核（运行时/链接形态未确认） | unknown（L3[?]: 预期 MIT，未取原文） | L,V,S,P | 否 | — | Y | — | 无直接调用 |
| 60 | vapoursynth<br>`--enable-vapoursynth` | VapourSynth 帧服务 | 待核（运行时加载） | unknown（L3[?]: 未取证） | L,V,S,P | 否 | — | Y | — | 无直接调用 |
| 61 | vulkan<br>`--enable-vulkan` | Vulkan 视频/滤镜 | 待核（loader/运行时） | unknown（L3[E]: Apache-2.0 为主） | L,V,S,P；NOTICE（若有） | 否 | — | Y | — | loader 分发形态未核 |
| 62 | zlib<br>`--enable-zlib` | zlib 压缩 | 是（静态配置） | unknown（L3[E]: zlib） | L,V,S,P | 否（FFmpeg 内部路径） | — | Y | — | 无直接调用 |
| 63 | winpthreads / MinGW-w64 runtime<br>`--enable-pthreads` | MinGW-w64 线程/运行时 | 是（推断：MinGW-w64 静态运行时） | unknown（工具链运行时；Lane3 未覆盖） | L,V,S,P | 是（运行时隐式） | — | Y | — | winpthreads/libgcc/libstdc++ 具体闭包未列 |
| 64 | libgomp（GCC OpenMP runtime）<br>`--extra-libs=-lgomp` | OpenMP 运行时 | 是（推断：`-lgomp` 静态链接） | unknown（GCC Runtime Library Exception 线索） | L,V,S,P；GCC 例外文本 | 是（运行时隐式） | — | Y | E | GCC Runtime Library Exception 文本未随包 |

## 4.5 已知传递依赖示例（不完整）

> 以下 11 项来自 Lane3 报告与 BtbN `scripts.d` 的依赖关系；**不是**完整传递闭包。`-buildconf` 不显示它们；本机未做符号级确认。librsvg 的 Rust crate 闭包、VMAF 模型文件、工具链运行时、glib 的更多依赖等仍未逐项列出。

| # | 组件（buildconf 开关） | 用途 | 静态进入 | 许可证（本地证据） | 通知/源码状态（需上游） | Screen_rec 调用 | 已确认 | 待上游 | 待法律 | 备注 |
|---:|---|---|---|---|---|---|:---:|:---:|:---:|---|
| 65 | glib<br>`transitive: librsvg -> glib` | librsvg 依赖（GLib） | 是（经 librsvg，推断） | unknown（L3[E]: LGPL-2.1+） | L,V,S,P | 否 | — | Y | L | 另带 pcre2/libffi/zlib 等；未逐项 |
| 66 | cairo<br>`transitive: librsvg -> cairo` | librsvg 依赖（2D 绘图） | 是（经 librsvg，推断） | unknown（L3[?]: 未取证） | L,V,S,P | 否 | — | Y | — | 许可未知；先补上游材料 |
| 67 | pango<br>`transitive: librsvg -> pango` | librsvg 依赖（文本布局） | 是（经 librsvg，推断） | unknown（L3[E]: Library GPL v2） | L,V,S,P | 否 | — | Y | L | LGPL 静态组件；重链接材料未提供 |
| 68 | libudfread<br>`transitive: libbluray -> libudfread` | libbluray 依赖（UDF 读取） | 是（经 libbluray，推断） | unknown（L3[?]: 未取证） | L,V,S,P | 否 | — | Y | — | 许可未知；先补上游材料 |
| 69 | mbedTLS<br>`transitive: librist -> mbedTLS` | librist 依赖（TLS/crypto） | 是（经 librist，推断） | unknown（L3[E]: Apache-2.0 或 GPL-2.0+） | L,V,S,P；另需记录所选分支 | 否 | — | Y | D | 双许可分支未记录 |
| 70 | libunibreak<br>`transitive: libass -> libunibreak` | libass 依赖（换行/断词） | 是（经 libass，推断） | unknown（L3[?]: 未取证） | L,V,S,P | 否 | — | Y | — | 许可未知；先补上游材料 |
| 71 | brotli<br>`transitive: libjxl -> brotli` | libjxl 依赖（Brotli 压缩） | 是（经 libjxl，推断） | unknown（L3[E]: MIT） | L,V,S,P | 否 | — | Y | — | 许可原文未随包 |
| 72 | highway<br>`transitive: libjxl -> highway` | libjxl 依赖（SIMD） | 是（经 libjxl，推断） | unknown（L3[?]: 未取证） | L,V,S,P | 否 | — | Y | — | 许可未知；先补上游材料 |
| 73 | skcms<br>`transitive: libjxl -> skcms` | libjxl 依赖（色彩管理） | 是（经 libjxl，推断） | unknown（L3[?]: 未取证） | L,V,S,P | 否 | — | Y | — | 许可未知；先补上游材料 |
| 74 | pcre2<br>`transitive: glib -> pcre2` | glib 依赖（正则） | 是（经 glib，推断） | unknown（L3[?]: 未取证） | L,V,S,P | 否 | — | Y | — | 许可未知；先补上游材料 |
| 75 | libffi<br>`transitive: glib -> libffi` | glib 依赖（FFI） | 是（经 glib，推断） | unknown（L3[?]: 未取证） | L,V,S,P | 否 | — | Y | — | 许可未知；先补上游材料 |

## 5. 本地许可/通知/源码材料现状

| 本地文件 | 覆盖范围 | 不覆盖/缺口 |
|---|---|---|
| `src-tauri/resources/licenses/FFmpeg-LICENSE.txt` | LGPLv3 正文（7,651 B） | 无 FFmpeg 版权声明；无逐组件文本；不是 GPLv3 正文 |
| `src-tauri/resources/licenses/GPLv3.txt` | GPLv3 正文（35,147 B） | 仅作为 LGPLv3 引用的 GPLv3 文本；是否必须随包由律师判断 |
| `src-tauri/resources/licenses/ffmpeg-source-offer.txt` | FFmpeg commit 的获取方式、构建镜像 digest、二进制哈希、sidecar 声明 | 未列第三方组件；未钉 BtbN 构建 commit/patches；无 3 年期书面 offer 的形式要件；未提供本地源码归档 |
| `src-tauri/resources/licenses/ffmpeg-build-info.txt` | FFmpeg commit、构建日期、configure flags、移除项 | 基础镜像只写 `:latest`；无依赖版本/源码/patches |
| `src-tauri/resources/licenses/ffmpeg-build-script.sh` | 容器内复现脚本；检出固定 FFmpeg commit；只移除 chromaprint | 未钉 BtbN 仓库 commit；依赖镜像内 `scripts.d`；未保存 build log |
| 构建包内 `LICENSE.txt` / `BUILD-INFO.txt` | 与上述 LGPLv3 / build-info 同哈希 | 同缺口；构建包无 `COPYING.GPLv3`、无 `LICENSE.md`、无组件文本 |
| 构建包内 `bin/ffplay.exe` | 构建产物 | 不在 NSIS 安装包；若分发整个 zip，SDL2/ffplay 进入分发面 |

结论：**现有随包材料只覆盖 FFmpeg 本体的 LGPLv3 文本与有限构建信息**；没有逐组件许可证/通知/源码，也没有 BtbN 精确构建脚本与 patches 清单。`ffmpeg-source-offer.txt` 的“本应用不静态/动态链接 FFmpeg”只描述宿主应用与 `ffmpeg.exe` 的关系，不能解释 `ffmpeg.exe` 内部静态链接的第三方库。

## 6. 仍需向上游/BtbN 索取的材料

### 6.1 构建级（BtbN / FFmpeg 上游）

| # | 对象 | 需要的材料 | 用途 |
|---:|---|---|---|
| 1 | BtbN/FFmpeg-Builds | 生成该镜像/构建的**精确仓库 commit 或不可变 tag**；`63ac...` 只是参考，不是证明 | 固定 `scripts.d` 与构建逻辑 |
| 2 | BtbN | GitHub Actions workflow run ID、build log、镜像 manifest；确认 `:latest` 与 digest `sha256:a0ac6b9b3b89cf1f3f0b6cb3556dceb6b6acec22946b4d8f79af516099b920fa` 的对应关系 | 证明构建输入与构建环境 |
| 3 | BtbN | `scripts.d/` 完整快照、所有 patches、`config.mak`、完整 `buildconf`、build log | Corresponding Source 的 build scripts/patches |
| 4 | BtbN | 每个依赖的 `SCRIPT_REPO`/`SCRIPT_COMMIT`/版本、源码 URL、源码归档 SHA-256、LICENSE/COPYING/NOTICE 路径 | 逐组件 SBOM 与通知/源码 |
| 5 | BtbN | 工具链：gcc 16.2.0、crosstool-NG 1.29.0.7_b1a94f6、binutils、MinGW-w64 runtime、libgomp 的版本/源码/许可证/例外文本 | 工具链运行时合规 |
| 6 | BtbN | 该产物的签名/attestation（若有）、可复现性声明 | provenance；减少对自述哈希的依赖 |
| 7 | BtbN | 若希望其作为第三方源码提供者：明确的 source offer/承诺/联系方式 | 支持第三方源码获取路径（需律师确认） |
| 8 | FFmpeg 上游 | `46d899d7a02bc63237472cc4897258d5ddf274ac` 的源码归档 + `LICENSE.md` + `COPYING.GPLv3` + `COPYING.LGPLv3`；确认该 commit 无本地 patch | 对应源码与完整许可文本 |

### 6.2 逐组件材料矩阵（L/V/S/P）

> 下表与第 4 节同一组件集合；`待索取` 表示本机没有该材料。OS 组件不随包，标 `—`。

| # | 组件 | LICENSE/COPYING/NOTICE | 版本/commit | 源码 + SHA-256 | patches/build scripts | 特殊项 |
|---:|---|---|---|---|---|---|
| 1 | FFmpeg 本体（libav*） | 已有 LGPLv3；GPLv3 已有 | 已确认 | 待归档/待 SHA-256 | 待 BtbN 精确脚本/patches | GPLv3 文本随包；源码归档、书面 offer、BtbN 精确材料仍缺 |
| 2 | libopenh264 | 待索取 | 待索取 | 待索取 | 待索取 | 高风险：BSD-2 文本/通知/源码均未随包；自编译二进制的专利安排待法律 |
| 3 | liblcevc-dec | 待索取 | 待索取 | 待索取 | 待索取 | 高风险：Clear BSD 文本/通知/源码均未随包；专利池未评估 |
| 4 | libmp3lame | 待索取 | 待索取 | 待索取 | 待索取 | LGPL 静态组件；重链接材料未提供 |
| 5 | libbluray | 待索取 | 待索取 | 待索取 | 待索取 | 传递依赖 libudfread；重链接材料未提供 |
| 6 | libplacebo | 待索取 | 待索取 | 待索取 | 待索取 | 依赖 Vulkan/shaderc/lcms2；重链接材料未提供 |
| 7 | libaom | 待索取 | 待索取 | 待索取 | 待索取 | AV1 专利授权范围待律师 |
| 8 | libvpx | 待索取 | 待索取 | 待索取 | 待索取 | VP8/VP9 专利授权范围待律师 |
| 9 | libsvtav1 | 待索取 | 待索取 | 待索取 | 待索取 | AV1 专利授权范围待律师 |
| 10 | libdav1d | 待索取 | 待索取 | 待索取 | 待索取 | AV1 专利授权范围待律师 |
| 11 | librav1e | 待索取 | 待索取 | 待索取 | 待索取 | Rust crate 依赖未展开；AV1 专利待律师 |
| 12 | libvvenc | 待索取 | 待索取 | 待索取 | 待索取 | VVC 专利池未评估；许可证原文未取 |
| 13 | libkvazaar | 待索取 | 待索取 | 待索取 | 待索取 | HEVC 专利池未评估 |
| 14 | libuavs3d | 待索取 | 待索取 | 待索取 | 待索取 | AVS3 专利/标准许可未评估 |
| 15 | liboapv | 待索取 | 待索取 | 待索取 | 待索取 | APV 专利/标准许可未评估 |
| 16 | libjxl | 待索取 | 待索取 | 待索取 | 待索取 | 传递依赖未逐项取证 |
| 17 | libvmaf | 待索取 | 待索取 | 待索取 | 待索取 | 模型文件是否随构建/分发未确认 |
| 18 | amf | 待索取 | 待索取 | 待索取 | 待索取 | AMF 运行时由 AMD 驱动提供 |
| 19 | cuda-llvm | 待索取 | 待索取 | 待索取 | 待索取 | 构建工具链 EULA/再分发需单独审查 |
| 20 | ffnvcodec | 待索取 | 待索取 | 待索取 | 待索取 | NVENC 运行时由 NVIDIA 驱动提供 |
| 21 | fontconfig | 待索取 | 待索取 | 待索取 | 待索取 | 无字幕/字体渲染调用 |
| 22 | gmp | 待索取 | 待索取 | 待索取 | 待索取 | 若选 GPLv3 分支，组合作品义务与 LGPL 路径不同 |
| 23 | libiconv | 待索取 | 待索取 | 待索取 | 待索取 | 若为 LGPL，静态重链接材料未提供 |
| 24 | lcms2 | 待索取 | 待索取 | 待索取 | 待索取 | 无直接调用 |
| 25 | libaribb24 | 待索取 | 待索取 | 待索取 | 待索取 | 许可证原文未取 |
| 26 | libaribcaption | 待索取 | 待索取 | 待索取 | 待索取 | 许可证原文未取 |
| 27 | libass | 待索取 | 待索取 | 待索取 | 待索取 | 传递 libunibreak |
| 28 | libfreetype | 待索取 | 待索取 | 待索取 | 待索取 | 若选 GPLv2 分支，义务与 FTL 不同 |
| 29 | libfribidi | 待索取 | 待索取 | 待索取 | 待索取 | LGPL 静态组件；重链接材料未提供 |
| 30 | libgme | 待索取 | 待索取 | 待索取 | 待索取 | 许可证原文未取 |
| 31 | libharfbuzz | 待索取 | 待索取 | 待索取 | 待索取 | 无直接调用 |
| 32 | libopencore-amrnb | 待索取 | 待索取 | 待索取 | 待索取 | Apache-2.0 NOTICE 未随包 |
| 33 | libopencore-amrwb | 待索取 | 待索取 | 待索取 | 待索取 | Apache-2.0 NOTICE 未随包 |
| 34 | libopenjpeg | 待索取 | 待索取 | 待索取 | 待索取 | 无直接调用 |
| 35 | libopenmpt | 待索取 | 待索取 | 待索取 | 待索取 | 无直接调用 |
| 36 | libopus | 待索取 | 待索取 | 待索取 | 待索取 | 无直接调用 |
| 37 | librist | 待索取 | 待索取 | 待索取 | 待索取 | mbedTLS 双许可分支未记录 |
| 38 | librsvg | 待索取 | 待索取 | 待索取 | 待索取 | Rust crate 闭包与 glib/cairo/pango 未逐项 |
| 39 | libshaderc | 待索取 | 待索取 | 待索取 | 待索取 | Apache-2.0 NOTICE 未随包 |
| 40 | libsnappy | 待索取 | 待索取 | 待索取 | 待索取 | 无直接调用 |
| 41 | libsoxr | 待索取 | 待索取 | 待索取 | 待索取 | LGPL 静态组件；重链接材料未提供 |
| 42 | libsrt | 待索取 | 待索取 | 待索取 | 待索取 | MPL 文件级源码获取说明未提供 |
| 43 | libssh | 待索取 | 待索取 | 待索取 | 待索取 | Lane3 未给出许可证线索；先补上游材料 |
| 44 | libtheora | 待索取 | 待索取 | 待索取 | 待索取 | 无直接调用 |
| 45 | libtwolame | 待索取 | 待索取 | 待索取 | 待索取 | LGPL 静态组件；重链接材料未提供 |
| 46 | libvorbis | 待索取 | 待索取 | 待索取 | 待索取 | 无直接调用 |
| 47 | libvpl | 待索取 | 待索取 | 待索取 | 待索取 | dispatcher 分发形态待核 |
| 48 | libwebp | 待索取 | 待索取 | 待索取 | 待索取 | 无直接调用 |
| 49 | libxml2 | 待索取 | 待索取 | 待索取 | 待索取 | 无直接调用 |
| 50 | libzimg | 待索取 | 待索取 | 待索取 | 待索取 | 无直接调用 |
| 51 | libzmq | 待索取 | 待索取 | 待索取 | 待索取 | MPL 文件级源码获取说明未提供 |
| 52 | libzvbi | 待索取 | 待索取 | 待索取 | 待索取 | 许可证原文未取 |
| 53 | lv2 | 待索取 | 待索取 | 待索取 | 待索取 | 无直接调用 |
| 54 | lzma | 待索取 | 待索取 | 待索取 | 待索取 | 许可证原文未取 |
| 55 | openal | 待索取 | 待索取 | 待索取 | 待索取 | LGPL 静态组件；重链接材料未提供 |
| 56 | opencl | 待索取 | 待索取 | 待索取 | 待索取 | 无直接调用 |
| 57 | schannel | —（OS 组件） | —（OS 组件） | —（OS 组件） | —（OS 组件） | 不随安装包分发 |
| 58 | sdl2 | 待索取（若分发 ffplay） | 待索取（若分发 ffplay） | 待索取（若分发 ffplay） | 待索取（若分发 ffplay） | 构建 zip 含 ffplay，但 NSIS 安装包只收 ffmpeg/ffprobe |
| 59 | vaapi | 待索取 | 待索取 | 待索取 | 待索取 | 无直接调用 |
| 60 | vapoursynth | 待索取 | 待索取 | 待索取 | 待索取 | 无直接调用 |
| 61 | vulkan | 待索取 | 待索取 | 待索取 | 待索取 | loader 分发形态未核 |
| 62 | zlib | 待索取 | 待索取 | 待索取 | 待索取 | 无直接调用 |
| 63 | winpthreads / MinGW-w64 runtime | 待索取 | 待索取 | 待索取 | 待索取 | winpthreads/libgcc/libstdc++ 具体闭包未列 |
| 64 | libgomp（GCC OpenMP runtime） | 待索取 | 待索取 | 待索取 | 待索取 | GCC Runtime Library Exception 文本未随包 |
| 65 | glib | 待索取 | 待索取 | 待索取 | 待索取 | 另带 pcre2/libffi/zlib 等；未逐项 |
| 66 | cairo | 待索取 | 待索取 | 待索取 | 待索取 | 许可未知；先补上游材料 |
| 67 | pango | 待索取 | 待索取 | 待索取 | 待索取 | LGPL 静态组件；重链接材料未提供 |
| 68 | libudfread | 待索取 | 待索取 | 待索取 | 待索取 | 许可未知；先补上游材料 |
| 69 | mbedTLS | 待索取 | 待索取 | 待索取 | 待索取 | 双许可分支未记录 |
| 70 | libunibreak | 待索取 | 待索取 | 待索取 | 待索取 | 许可未知；先补上游材料 |
| 71 | brotli | 待索取 | 待索取 | 待索取 | 待索取 | 许可原文未随包 |
| 72 | highway | 待索取 | 待索取 | 待索取 | 待索取 | 许可未知；先补上游材料 |
| 73 | skcms | 待索取 | 待索取 | 待索取 | 待索取 | 许可未知；先补上游材料 |
| 74 | pcre2 | 待索取 | 待索取 | 待索取 | 待索取 | 许可未知；先补上游材料 |
| 75 | libffi | 待索取 | 待索取 | 待索取 | 待索取 | 许可未知；先补上游材料 |

### 6.3 特殊材料/确认项

| 组件/组 | 额外需要 | 原因 |
|---|---|---|
| `libopenh264` | Cisco 对“自编译源码再分发”的专利/royalty 书面说明；BSD-2 文本与版权通知 | 许可证不含专利授权 |
| `liblcevc-dec` | V-Nova/专利池的许可说明；Clear BSD 文本与版权通知 | 上游明示不授予专利 |
| `gmp` | 构建时选择 GPLv3 还是 LGPLv3 的记录；对应许可证文本 | 双许可分支改变义务 |
| `libfreetype` | 构建时选择 FTL 还是 GPLv2 的记录；对应许可证文本 | 双许可分支改变义务 |
| `librist` / `mbedTLS` | mbedTLS 选择 Apache-2.0 还是 GPL-2.0+ 的记录；librist 自身 LICENSE | 双许可分支改变义务 |
| `librsvg` / `librav1e` | Rust `Cargo.lock`/`cargo vendor` 清单；每个 crate 的 LICENSE/COPYING | Rust 闭包未展开 |
| `libvmaf` | VMAF 模型文件清单、模型许可/来源 | 模型文件可能单独分发 |
| `libzmq` / `libsrt` | MPL-2.0 覆盖文件的源码获取说明 | MPL 文件级源码义务 |
| `libaom` / `libopencore-amr*` / `libshaderc` / `vulkan` / `opencl` | Apache/BSD NOTICE（若有）与版权页 | 许可文本/NOTICE 未随包 |
| `winpthreads` / `libgomp` | MinGW-w64 runtime、libgcc/libstdc++、libgomp 的许可证与 GCC Runtime Library Exception 文本 | 工具链运行时未列 |
| `ffplay` / `sdl2` | 若分发整个构建 zip：SDL2 文本与 ffplay 相关通知 | 当前安装包不含 ffplay，但 zip 含 |

## 7. 正式发行前仍需律师确认的问题

> 以下问题只用于把工程缺口转成法务议题，不是法律结论；回答前不应对外宣称任何合规状态。

1. **独立进程是否构成 Combined Work**：Screen_rec 通过 `Command::new` 启动 `ffmpeg.exe`/`ffprobe.exe`，不链接 `libav*`；这是否使 Screen_rec 不落入 LGPLv3 §4 的 Combined Work？若落入，§4(a)-(e) 对应用本身意味着什么？
2. **GPLv3 正文是否必须随包**：LGPLv3 引用 GPLv3；当前已随包 `GPLv3.txt`，但 `LICENSE.txt` 本身不含 GPLv3 正文。GPLv3 §4(b)/§6 是否要求 GPLv3 全文随二进制分发？
3. **Corresponding Source 的范围**：仅提供 FFmpeg 上游 commit URL 是否足够？是否必须同时提供 BtbN `scripts.d`、patches、依赖源码、工具链源码/构建脚本？本地归档、书面 offer、同址网络访问三条路径应选哪条？
4. **书面 offer 的形式要件**：GPLv3 §6(b)/(c) 要求至少 3 年、可联系、提供源码/媒体；当前 source offer 只写“媒介费用由索取方承担”“长期有效”，没有 offeror 名称/地址/联系方式/3 年期限。需要怎样的模板？BtbN 作为第三方源码提供者是否需要明确同意/承诺？
5. **静态 LGPL 依赖的重链接义务**：`libiconv`、`libfribidi`、`librsvg`（含 glib/pango）、`libbluray`、`libplacebo`、`libmp3lame`、`libsoxr`、`libtwolame`、`libgme`、`libaribb24`、`libzvbi`、`openal`、`gmp`（若 LGPL 分支）等以静态库进入 `ffmpeg.exe`；LGPLv3 §4(d)(0) 是否要求提供 object files/relink kit？当前 `--pkg-config-flags=--static` 是否使 §4(d)(1) 共享库路径不可用？
6. **OpenH264 专利/royalty**：Cisco 的 royalty-free 安排是否覆盖 BtbN 从源码自编译、再由 Screen_rec 分发的 `libopenh264`？若保留该组件，Screen_rec 是否需要向 MPEG-LA 取得 H.264 专利许可？
7. **LCEVC 专利池**：Clear BSD 明示不授予专利；若保留 `liblcevc-dec`，是否需要 V-Nova/专利池许可？若应用不调用，是否应直接禁用？
8. **其他编解码器专利**：HEVC（`libkvazaar`）、VVC（`libvvenc`）、AVS3（`libuavs3d`）、APV（`liboapv`）、AV1（`libaom`/`libvpx`/`libsvtav1`/`librav1e`/`libdav1d`）、JPEG XL（`libjxl`）以及产品实际使用的 H.264 硬件编码（NVENC/QSV/AMF/Media Foundation）与 AAC，是否存在需要单独取得的专利许可？
9. **通知的显著性**：LGPLv3 §4(a) 要求“显著通知”；当前只有安装目录 `licenses/` 文件，`tauri.conf.json` 没有 `licenseFile`；Lane2 报告称未发现 About/license 界面（本清单未复核 UI）。是否必须进入 UI/安装器/首次启动？仅 `licenses/` 是否足够？
10. **Installation Information**：GPLv3 §6 的 Installation Information 是否适用于本 Windows 桌面产品/NSIS 安装器？若适用，需要提供什么？
11. **双许可分支选择**：`gmp`（GPLv3/LGPLv3）、`libfreetype`（FTL/GPLv2）、`mbedTLS`（Apache-2.0/GPL-2.0+）在 BtbN 构建中选择了哪一支？选择结果如何改变通知、源码、重链接义务？
12. **分发边界**：当前 NSIS 只收 `ffmpeg.exe`/`ffprobe.exe`；若对外分发完整 BtbN 构建 zip（含 `ffplay.exe`、doc、presets），SDL2、ffplay 文档/预设的许可义务是否进入范围？是否应避免分发完整 zip？
13. **专利与出口/市场**：目标市场（中国/其他）是否对 H.264/AAC/AV1/HEVC/VVC 专利用不同规则？硬件编码器（NVENC/QSV/AMF/MF）的专利责任是否由驱动/OS 供应商承担，还是由 Screen_rec 承担？

## 8. 本清单的自检与边界

### 8.1 已执行的自检（本机、只读）

| 检查 | 命令/方法 | 结果 |
|---|---|---|
| 版本 | `ffmpeg.exe -version` / `ffprobe.exe -version` | 均为 `n8.1.3-18-g46d899d7a0-20261019`；退出码 0 |
| 构建配置 | `ffmpeg.exe -buildconf` | `chromaprint=0`、`fftw=0`、`--enable-gpl=0`、`--enable-nonfree=0`、`--enable-version3=1` |
| muxers | `ffmpeg.exe -muxers` | `chromaprint=0` |
| 许可输出 | `ffmpeg.exe -L` | LGPLv3-or-later 文本；无逐组件清单 |
| 哈希 | `Get-FileHash -Algorithm SHA256` | `ffmpeg.exe`/`ffprobe.exe` 与整改报告一致 |
| JSON 语法 | `python -m json.tool docs/compliance/sbom-ffmpeg.json` | 退出码 0；JSON 可解析 |
| SPDX 结构 | 自写 Python 检查：`SPDXID` 唯一、relationship 引用存在、必需字段存在 | `VALIDATION=OK`；packages=78、relationships=89 |
| 组件计数（外部） | 自写 Python 检查：直接/传递组件包 | components=74（direct=63、transitive=11；不含 FFmpeg 本体） |
| 许可证计数 | 自写 Python 检查：`licenseConcluded` | FFmpeg 2 个包为 `LGPL-3.0-or-later`；其余 76 个包为 `NOASSERTION` |
| 清单与 buildconf 一致性 | 自写 Python 检查：SBOM 注释中的 `--enable-*` 集合 vs `-buildconf` | buildconf 63 个 `--enable-*`；扣除 `--enable-version3` 后 62 个；SBOM 映射 62 个；missing=0、extra=0 |
| chromaprint/FFTW 不在 SBOM 组件中 | 自写 Python 检查包名/`SPDXID` | 0 个 chromaprint/FFTW 包；仅文档注释说明“已移除” |

### 8.2 未做/不能由本清单证明

- 未做逐符号/逐目标文件的静态链接证明；“静态进入”是配置层推断。
- 未联网核对 BtbN 官方资产、签名、镜像 manifest 或 GitHub workflow。
- 未取得 BtbN `scripts.d`、patches、依赖版本/源码、构建日志。
- 未对 74 项组件逐项取得 LICENSE/COPYING/NOTICE 原文；许可证列除 FFmpeg 外均为 `unknown`/`NOASSERTION`。
- 未做专利检索、未做法律审查、未做安装包重新构建或媒体回归。
- 未证明“只移除 chromaprint”与镜像原始 configure 逐字一致；也未证明 BtbN 未对 FFmpeg 打 patch。
- 未把 `ffplay.exe` 纳入当前安装包；但构建 zip 内含 `ffplay.exe`，若直接分发 zip 需重新评估。

### 8.3 结论边界

本清单是**工程侧部分清单与缺口台账**，不是法律意见，也不是发行许可。它列出当前二进制可核验的组件与构建事实，并明确标注哪些许可证/通知/源码仍为 `unknown`/`NOASSERTION`。在 BtbN/上游材料补齐、律师完成上述问题审查、且 release checklist 的发行门禁通过之前，不应把本清单当作任何市场正式发行的依据。
