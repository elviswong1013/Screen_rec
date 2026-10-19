# FFmpeg LGPL 整改（chromaprint/FFTW）

- 日期：2026-10-19
- 范围：移除随包 FFmpeg 的 `chromaprint`/FFTW（GPLv2+），使二进制回到本项目声明的 LGPLv3 路径。
- 基线：`35c0c08`（M18）；本整改未改录制/编码业务代码。

## 1. 问题

随包 `ffmpeg.exe`/`ffprobe.exe` 的 BtbN `win64-lgpl-8.1` 配置包含
`--enable-chromaprint`。BtbN 的 `50-chromaprint.sh` 依赖 `fftw3`，`25-fftw3.sh`
以 `--disable-shared --enable-static` 构建 FFTW；FFTW `COPYING` 为 GPLv2+。
因此旧二进制并非纯 LGPLv3，旧 source offer 只附 LGPL 文本，属于发行阻断项。

## 2. 修复

- 使用 BtbN 构建镜像
  `ghcr.io/btbn/ffmpeg-builds/win64-lgpl-8.1@sha256:a0ac6b9b3b89cf1f3f0b6cb3556dceb6b6acec22946b4d8f79af516099b920fa`
  与同一 FFmpeg commit `46d899d7a02bc63237472cc4897258d5ddf274ac`。
- 仅从镜像的 configure 参数中移除 `--enable-chromaprint`；保留
  `--enable-version3`、BtbN LGPL 依赖、`libopenh264`、`liblcevc-dec`；
  无 `--enable-gpl`/`--enable-nonfree`；未修改 FFmpeg 源码。
- 复现脚本：`src-tauri/resources/licenses/ffmpeg-build-script.sh`；
  完整 configure 行：`src-tauri/resources/licenses/ffmpeg-build-info.txt`。
- 版本串：`n8.1.3-18-g46d899d7a0-20261019`。

## 3. 产物与哈希

| 项 | 值 |
|---|---|
| 定制构建包 | `ffmpeg-n8.1.3-18-g46d899d7a0-20261019-win64-lgpl.zip`（167,108,447 B） |
| 构建包 SHA-256 | `015EB37E45AFB52901DFF77562819A524DEEED07DDEFADDE766ABB14339BC502` |
| ffmpeg.exe SHA-256 | `3091264B90D26398E0366B0E422E16FD76D2ECF85868E4F072209721E7C793F3` |
| ffprobe.exe SHA-256 | `CB47EFF47532A4F5CE8F3A0FB76FDED41BD401D168C61C7DD187EBBC318B8CAD` |
| LGPLv3 文本 SHA-256 | `DA7EABB7BAFDF7D3AE5E9F223AA5BDC1EECE45AC569DC21B3B037520B4464768` |
| GPLv3 文本 SHA-256 | `8CEB4B9EE5ADEDDE47B31E975C1D90C73AD27B6B165A1DCD80C7C545EB65B903` |

## 4. 验证（工程）

- `ffmpeg -buildconf`：无 `chromaprint`、无 `fftw`；仍有 `--enable-version3`，
  无 `--enable-gpl`/`--enable-nonfree`。
- `ffmpeg -muxers` / `ffprobe -muxers`：无 `chromaprint`。
- 冒烟：新二进制用 `h264_mf` + AAC 编码 1 秒 640×360 H.264/AAC MKV，
  ffprobe 报告 H.264 + AAC 48k 双声道，退出码 0。
- `-encoders` 仍有 `h264_nvenc`/`h264_qsv`/`h264_amf`/`h264_mf`/`aac`；
  `-muxers` 仍有 matroska/mp4。
- 代码审计新增 `forbidden_muxer_markers = ["chromaprint"]`，防止后续构建回退：
  `cargo test -p sr-pipeline --test license_and_build_audit -- --nocapture` → 1 passed。
- `cargo test --workspace` → 全绿（含 53 项 screen-rec L1）。
- L2 `container` 14/14（含四档率 MP4、多轨 MP4、取消/回滚/恢复）。
- L2 `region_video` 10/10（含 48k/96k 区域、MP4、恢复、并发、合并、音轨矩阵）。
- L2 `sync_dual` 90s 抽查 3/3：
  - 48k：max 70.6 / 55.2 ms；
  - 44.1k：max 51.8 / 62.7 ms；
  - 96k：max 58.3 / 53.4 ms。
- 未重跑 610s 长测；本次只替换 FFmpeg 构建配置，编码器/封装参数未变，90s 抽查用于回归。

## 5. 未完成/待办

- 本整改只消除 GPL 冲突；`libopenh264`、`liblcevc-dec` 等静态组件的逐项通知/
  源码清单仍未补齐（正式发行前需处理）。
- Rust 第三方 notices、EULA/隐私政策、H.264/AAC 专利适用性仍未清权。
- 新安装包尚未重建；重建后需做包—源码一致性、安装/自检。
- 本报告是工程证据，不是法律意见；最终合规由律师/许可方确认。
