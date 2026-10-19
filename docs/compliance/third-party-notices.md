# Screen_rec Rust 第三方组件通知候选（Windows x64 运行时）

> **状态：通知候选（notification candidate），需法律审阅。**
>
> 本文件是工程侧整理的候选材料，**不是法律意见**，不构成合规、审计或清权结论，也不能替代律师对通知形式、NOTICE 适用性、MPL 源码提供方式等问题的判断。
>
> **尚未接入安装包 About 页/安装器许可页。** 当前 `src-tauri/tauri.conf.json` 的 `bundle.resources` 仍只包含 FFmpeg 相关文件；本文件与 `third-party-licenses.txt`、`sbom-rust.json` 均未被打包。

- 生成日期：2026-10-19
- 代码基线：`git HEAD 24575f2e319760932749d52f72ae6b67363ed0b9`（`24575f2`）
- 目标平台：Windows x64（`x86_64-pc-windows-msvc`）
- 范围：`screen-rec` 的 Windows x64 normal **runtime** 闭包中的第三方 crate
- 第三方 runtime crate 数：**162**
- 许可证文本来源：本机 cargo registry 中对应 crate 包内的 `LICENSE*` / `COPYING*` / `NOTICE*` / `UNLICENSE*` / `COPYRIGHT*` 文件；**未联网获取、未从上游仓库补全**
- 配套文件：[`third-party-licenses.txt`](third-party-licenses.txt)（逐包拼接的本地许可文本）、[`sbom-rust.json`](sbom-rust.json)（SPDX 2.3 候选 SBOM）

## 0. 生成口径与边界

### 0.1 为什么不是 `Cargo.lock` 全量

本文件**没有**把 `Cargo.lock` 当作发货清单。`Cargo.lock` 在当前基线有 482 个 package 条目，包含非 Windows target、build script、proc-macro、dev/test 等包；直接使用会显著过列。

本次重新生成（HEAD `24575f2`）的实际命令：

```powershell
cargo metadata --offline --locked --format-version 1

# 本次将 CARGO_TARGET_DIR 指向临时目录；unit-graph 只输出图，不编译
$env:RUSTC_BOOTSTRAP = "1"
cargo build --release --unit-graph -Z unstable-options `
  --offline --locked --target x86_64-pc-windows-msvc -p screen-rec
Remove-Item Env:RUSTC_BOOTSTRAP

cargo tree -e normal --target x86_64-pc-windows-msvc --offline --locked -p screen-rec
```

> `--unit-graph` 只输出依赖/构建图并退出，**没有执行编译**；本次未构建安装包、未安装依赖、未联网。

口径：

1. 从 `screen-rec` 的 `bin` 与 `rlib` 根单元出发，遍历 release unit graph 的依赖。
2. 只保留 `platform == x86_64-pc-windows-msvc` 的单元。
3. 排除 `custom-build`（build script）与 `proc-macro`（host 过程宏）单元。
4. 按 `pkg_id` 去重；其中 workspace 自有 crate 不计入第三方。
5. 结果：162 个第三方 runtime crate；另 8 个 workspace crate 不计入；`cargo tree -e normal` 的 244 个 target-resolved name@version 中，其余 74 个为 build-only，被排除。
6. 本次重跑得到的 162 个第三方 runtime 集合与 lane1 基线 `3bb8c36` 的 162 集合**一致**（比对结果：新增 0、删除 0）；但本文件的命令输出以 HEAD `24575f2` 为准。

> `Cargo.lock` 仅用于读取这 162 个包的 SHA-256 校验和，不作为清单来源。

### 0.2 覆盖边界

本文件只覆盖 **Rust 第三方 normal runtime**。以下不在本文件范围内，不能据此认为它们已处理：

- 前端 npm/Node 依赖；
- FFmpeg/ffprobe sidecar（有单独 LGPL 文本与源码说明，本文件不评价其充分性）；
- WebView2 Runtime、Windows 系统 DLL、MSVC/Windows SDK 运行库；
- NSIS/Tauri 安装器引擎及其自带组件；
- workspace 自有 crate 的许可文本；
- AVC/AAC 等专利许可；
- 任何法律意见或合规结论。

## 1. 统计

| 项目 | 数量 | 说明 |
|---|---:|---|
| Windows x64 runtime 第三方 crate | **162** | 本文件清单与 SBOM 的包数 |
| 其中按保守口径“随包” | **162** | 属于 runtime 编译单元闭包；不表示每个符号一定被链接（可能有 DCE） |
| 本地至少有 1 个许可/通知文件 | 159 | 157 个覆盖表达式列出的全部许可族，2 个仅覆盖部分替代文本 |
| 本地无许可/通知文件 | 3 | 见 §3.3；需向上游获取 |
| MPL-2.0 | 1 | `option-ext 0.2.0`，见 §3.1 |
| 表达式含 Apache-2.0 | 116 | 其中 Apache-only 或 Apache AND MIT 共 2 个；本地均未发现 NOTICE* 文件，见 §3.2 |
| 本地文件名匹配 `NOTICE*` | 0 | 162 个包的本地 registry 内容中均未发现；不代表上游仓库没有 NOTICE |
| 非规范 SPDX 斜杠表达式 | 5 | 见 §3.4；SBOM 中已映射为 `OR` |

### 1.1 license expression 分布（Cargo.toml 原值）

| license expression | 包数 |
|---|---:|
| `MIT OR Apache-2.0` | 88 |
| `MIT` | 20 |
| `Apache-2.0 OR MIT` | 15 |
| `Unicode-3.0` | 15 |
| `Unlicense OR MIT` | 4 |
| `BSD-3-Clause` | 2 |
| `BSD-3-Clause OR Apache-2.0` | 2 |
| `MIT OR Zlib OR Apache-2.0` | 2 |
| `Unlicense/MIT` | 2 |
| `0BSD OR Apache-2.0` | 1 |
| `0BSD OR MIT OR Apache-2.0` | 1 |
| `Apache-2.0` | 1 |
| `Apache-2.0 / MIT` | 1 |
| `Apache-2.0 AND MIT` | 1 |
| `BSD-3-Clause AND MIT` | 1 |
| `BSD-3-Clause/MIT` | 1 |
| `CC0-1.0 OR MIT-0 OR Apache-2.0` | 1 |
| `MIT OR Apache-2.0 OR Zlib` | 1 |
| `MIT/Apache-2.0` | 1 |
| `MPL-2.0` | 1 |
| `Zlib OR Apache-2.0 OR MIT` | 1 |

> 统计为文件/表达式层面的事实核对，不是法律结论；“已包含”不等于“通知义务已满足”。

## 2. 全量清单（162 个第三方 runtime crate）

列说明：

- **license expression**：`Cargo.toml` 原值，保留非规范斜杠写法；SBOM 中的规范化值见 §3.4 与 `sbom-rust.json` 的 `licenseComments`。
- **随包**：`是` = 属于 Windows x64 runtime 编译单元闭包，按通知义务保守纳入；不表示逐符号链接。
- **许可证文本包含情况**：`已包含` = 本地文件覆盖表达式列出的全部许可族（按文本标记自动核对）；`部分` = 本地有文本但未覆盖全部 OR 替代项；`缺失` = 本地 registry 包内没有许可/通知文件。
- **来源 URL**：crates.io 对应版本页面；不是上游源码仓库。

| # | package | version | license expression（Cargo.toml 原值） | 随包 | 许可证文本包含情况 | 来源 URL（crates.io） |
|---:|---|---|---|---|---|---|
| 1 | `adler2` | 2.0.1 | `0BSD OR MIT OR Apache-2.0` | 是 | 已包含（3 文件） | https://crates.io/crates/adler2/2.0.1 |
| 2 | `aho-corasick` | 1.1.5 | `Unlicense OR MIT` | 是 | 已包含（3 文件） | https://crates.io/crates/aho-corasick/1.1.5 |
| 3 | `alloc-no-stdlib` | 3.0.0 | `BSD-3-Clause` | 是 | 已包含（1 文件） | https://crates.io/crates/alloc-no-stdlib/3.0.0 |
| 4 | `alloc-stdlib` | 0.3.0 | `BSD-3-Clause` | 是 | 缺失（0 文件；见 §3.3） | https://crates.io/crates/alloc-stdlib/0.3.0 |
| 5 | `anyhow` | 1.0.104 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/anyhow/1.0.104 |
| 6 | `audio-codec-algorithms` | 0.8.1 | `0BSD OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/audio-codec-algorithms/0.8.1 |
| 7 | `audioadapter` | 5.0.0 | `MIT OR Apache-2.0` | 是 | 已包含（3 文件） | https://crates.io/crates/audioadapter/5.0.0 |
| 8 | `audioadapter-buffers` | 5.2.0 | `MIT OR Apache-2.0` | 是 | 已包含（3 文件） | https://crates.io/crates/audioadapter-buffers/5.2.0 |
| 9 | `audioadapter-sample` | 5.2.0 | `MIT OR Apache-2.0` | 是 | 已包含（3 文件） | https://crates.io/crates/audioadapter-sample/5.2.0 |
| 10 | `base64` | 0.23.1 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/base64/0.23.1 |
| 11 | `bitflags` | 2.13.2 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/bitflags/2.13.2 |
| 12 | `brotli` | 9.0.0 | `BSD-3-Clause AND MIT` | 是 | 已包含（2 文件） | https://crates.io/crates/brotli/9.0.0 |
| 13 | `brotli-decompressor` | 6.0.1 | `BSD-3-Clause/MIT` | 是 | 部分（1 文件；见 §3.3） | https://crates.io/crates/brotli-decompressor/6.0.1 |
| 14 | `bytemuck` | 1.25.2 | `Zlib OR Apache-2.0 OR MIT` | 是 | 已包含（3 文件） | https://crates.io/crates/bytemuck/1.25.2 |
| 15 | `byteorder-lite` | 0.1.0 | `Unlicense OR MIT` | 是 | 已包含（2 文件） | https://crates.io/crates/byteorder-lite/0.1.0 |
| 16 | `bytes` | 1.12.1 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/bytes/1.12.1 |
| 17 | `cfb` | 0.14.0 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/cfb/0.14.0 |
| 18 | `cfg-if` | 1.0.5 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/cfg-if/1.0.5 |
| 19 | `cookie` | 0.18.2 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/cookie/0.18.2 |
| 20 | `crc32fast` | 1.5.2 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/crc32fast/1.5.2 |
| 21 | `crossbeam-channel` | 0.5.17 | `MIT OR Apache-2.0` | 是 | 已包含（3 文件） | https://crates.io/crates/crossbeam-channel/0.5.17 |
| 22 | `crossbeam-deque` | 0.8.8 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/crossbeam-deque/0.8.8 |
| 23 | `crossbeam-epoch` | 0.9.21 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/crossbeam-epoch/0.9.21 |
| 24 | `crossbeam-utils` | 0.8.23 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/crossbeam-utils/0.8.23 |
| 25 | `ctor` | 1.0.13 | `Apache-2.0 OR MIT` | 是 | 已包含（2 文件） | https://crates.io/crates/ctor/1.0.13 |
| 26 | `deranged` | 0.5.9 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/deranged/0.5.9 |
| 27 | `dirs` | 7.0.0 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/dirs/7.0.0 |
| 28 | `dirs-sys` | 0.5.0 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/dirs-sys/0.5.0 |
| 29 | `dpi` | 0.1.2 | `Apache-2.0 AND MIT` | 是 | 已包含（2 文件） | https://crates.io/crates/dpi/0.1.2 |
| 30 | `dunce` | 1.0.5 | `CC0-1.0 OR MIT-0 OR Apache-2.0` | 是 | 部分（1 文件；见 §3.3） | https://crates.io/crates/dunce/1.0.5 |
| 31 | `either` | 1.19.0 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/either/1.19.0 |
| 32 | `equivalent` | 1.0.3 | `Apache-2.0 OR MIT` | 是 | 已包含（2 文件） | https://crates.io/crates/equivalent/1.0.3 |
| 33 | `erased-serde` | 0.4.10 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/erased-serde/0.4.10 |
| 34 | `fast_image_resize` | 5.5.0 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/fast_image_resize/5.5.0 |
| 35 | `fdeflate` | 0.3.7 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/fdeflate/0.3.7 |
| 36 | `flate2` | 1.1.10 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/flate2/1.1.10 |
| 37 | `fnv` | 1.0.7 | `Apache-2.0 / MIT` | 是 | 已包含（2 文件） | https://crates.io/crates/fnv/1.0.7 |
| 38 | `form_urlencoded` | 1.2.2 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/form_urlencoded/1.2.2 |
| 39 | `getrandom` | 0.3.4 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/getrandom/0.3.4 |
| 40 | `glob` | 0.3.4 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/glob/0.3.4 |
| 41 | `hashbrown` | 0.17.1 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/hashbrown/0.17.1 |
| 42 | `heck` | 0.5.0 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/heck/0.5.0 |
| 43 | `http` | 1.5.0 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/http/1.5.0 |
| 44 | `icu_collections` | 2.3.0 | `Unicode-3.0` | 是 | 已包含（1 文件） | https://crates.io/crates/icu_collections/2.3.0 |
| 45 | `icu_locale_core` | 2.3.0 | `Unicode-3.0` | 是 | 已包含（1 文件） | https://crates.io/crates/icu_locale_core/2.3.0 |
| 46 | `icu_normalizer` | 2.3.0 | `Unicode-3.0` | 是 | 已包含（1 文件） | https://crates.io/crates/icu_normalizer/2.3.0 |
| 47 | `icu_normalizer_data` | 2.3.0 | `Unicode-3.0` | 是 | 已包含（1 文件） | https://crates.io/crates/icu_normalizer_data/2.3.0 |
| 48 | `icu_properties` | 2.3.0 | `Unicode-3.0` | 是 | 已包含（1 文件） | https://crates.io/crates/icu_properties/2.3.0 |
| 49 | `icu_properties_data` | 2.3.0 | `Unicode-3.0` | 是 | 已包含（1 文件） | https://crates.io/crates/icu_properties_data/2.3.0 |
| 50 | `icu_provider` | 2.3.1 | `Unicode-3.0` | 是 | 已包含（1 文件） | https://crates.io/crates/icu_provider/2.3.1 |
| 51 | `idna` | 1.1.0 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/idna/1.1.0 |
| 52 | `idna_adapter` | 1.2.2 | `Apache-2.0 OR MIT` | 是 | 已包含（2 文件） | https://crates.io/crates/idna_adapter/1.2.2 |
| 53 | `image` | 0.25.10 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/image/0.25.10 |
| 54 | `indexmap` | 2.14.2 | `Apache-2.0 OR MIT` | 是 | 已包含（2 文件） | https://crates.io/crates/indexmap/2.14.2 |
| 55 | `infer` | 0.22.0 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/infer/0.22.0 |
| 56 | `itoa` | 1.0.18 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/itoa/1.0.18 |
| 57 | `json-patch` | 4.2.0 | `MIT/Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/json-patch/4.2.0 |
| 58 | `jsonptr` | 0.7.1 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/jsonptr/0.7.1 |
| 59 | `keyboard-types` | 0.8.3 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/keyboard-types/0.8.3 |
| 60 | `libc` | 0.2.190 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/libc/0.2.190 |
| 61 | `litemap` | 0.8.3 | `Unicode-3.0` | 是 | 已包含（1 文件） | https://crates.io/crates/litemap/0.8.3 |
| 62 | `lock_api` | 0.4.14 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/lock_api/0.4.14 |
| 63 | `log` | 0.4.34 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/log/0.4.34 |
| 64 | `memchr` | 2.8.3 | `Unlicense OR MIT` | 是 | 已包含（3 文件） | https://crates.io/crates/memchr/2.8.3 |
| 65 | `mime` | 0.3.17 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/mime/0.3.17 |
| 66 | `miniz_oxide` | 0.8.9 | `MIT OR Zlib OR Apache-2.0` | 是 | 已包含（4 文件） | https://crates.io/crates/miniz_oxide/0.8.9 |
| 67 | `miniz_oxide` | 0.9.1 | `MIT OR Zlib OR Apache-2.0` | 是 | 已包含（4 文件） | https://crates.io/crates/miniz_oxide/0.9.1 |
| 68 | `mio` | 1.2.4 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/mio/1.2.4 |
| 69 | `moxcms` | 0.8.1 | `BSD-3-Clause OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/moxcms/0.8.1 |
| 70 | `muda` | 0.20.0 | `Apache-2.0 OR MIT` | 是 | 已包含（3 文件） | https://crates.io/crates/muda/0.20.0 |
| 71 | `num-conv` | 0.2.2 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/num-conv/0.2.2 |
| 72 | `num-integer` | 0.1.47 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/num-integer/0.1.47 |
| 73 | `num-traits` | 0.2.19 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/num-traits/0.2.19 |
| 74 | `once_cell` | 1.21.4 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/once_cell/1.21.4 |
| 75 | `option-ext` | 0.2.0 | `MPL-2.0` | 是 | 已包含（1 文件） | https://crates.io/crates/option-ext/0.2.0 |
| 76 | `parking_lot` | 0.12.5 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/parking_lot/0.12.5 |
| 77 | `parking_lot_core` | 0.9.12 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/parking_lot_core/0.9.12 |
| 78 | `percent-encoding` | 2.3.2 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/percent-encoding/2.3.2 |
| 79 | `phf` | 0.13.1 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/phf/0.13.1 |
| 80 | `phf_shared` | 0.13.1 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/phf_shared/0.13.1 |
| 81 | `pin-project-lite` | 0.2.17 | `Apache-2.0 OR MIT` | 是 | 已包含（2 文件） | https://crates.io/crates/pin-project-lite/0.2.17 |
| 82 | `plist` | 1.10.1 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/plist/1.10.1 |
| 83 | `png` | 0.18.1 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/png/0.18.1 |
| 84 | `potential_utf` | 0.1.6 | `Unicode-3.0` | 是 | 已包含（1 文件） | https://crates.io/crates/potential_utf/0.1.6 |
| 85 | `powerfmt` | 0.2.1 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/powerfmt/0.2.1 |
| 86 | `pxfm` | 0.1.30 | `BSD-3-Clause OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/pxfm/0.1.30 |
| 87 | `quick-xml` | 0.42.0 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/quick-xml/0.42.0 |
| 88 | `raw-window-handle` | 0.6.2 | `MIT OR Apache-2.0 OR Zlib` | 是 | 已包含（3 文件） | https://crates.io/crates/raw-window-handle/0.6.2 |
| 89 | `rayon` | 1.12.0 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/rayon/1.12.0 |
| 90 | `rayon-core` | 1.13.0 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/rayon-core/1.13.0 |
| 91 | `regex` | 1.13.1 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/regex/1.13.1 |
| 92 | `regex-automata` | 0.4.18 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/regex-automata/0.4.18 |
| 93 | `regex-syntax` | 0.8.11 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/regex-syntax/0.8.11 |
| 94 | `rubato` | 5.0.1 | `MIT OR Apache-2.0` | 是 | 已包含（3 文件） | https://crates.io/crates/rubato/5.0.1 |
| 95 | `same-file` | 1.0.6 | `Unlicense/MIT` | 是 | 已包含（3 文件） | https://crates.io/crates/same-file/1.0.6 |
| 96 | `scopeguard` | 1.2.0 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/scopeguard/1.2.0 |
| 97 | `semver` | 1.0.28 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/semver/1.0.28 |
| 98 | `serde` | 1.0.229 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/serde/1.0.229 |
| 99 | `serde-untagged` | 0.1.9 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/serde-untagged/0.1.9 |
| 100 | `serde_core` | 1.0.229 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/serde_core/1.0.229 |
| 101 | `serde_json` | 1.0.154 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/serde_json/1.0.154 |
| 102 | `serde_spanned` | 1.1.2 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/serde_spanned/1.1.2 |
| 103 | `serde_with` | 3.24.0 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/serde_with/3.24.0 |
| 104 | `serialize-to-javascript` | 0.1.2 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/serialize-to-javascript/0.1.2 |
| 105 | `simd-adler32` | 0.3.10 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/simd-adler32/0.3.10 |
| 106 | `siphasher` | 1.0.4 | `MIT OR Apache-2.0` | 是 | 已包含（3 文件） | https://crates.io/crates/siphasher/1.0.4 |
| 107 | `smallvec` | 1.16.3 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/smallvec/1.16.3 |
| 108 | `socket2` | 0.6.5 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/socket2/0.6.5 |
| 109 | `softbuffer` | 0.4.8 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/softbuffer/0.4.8 |
| 110 | `stable_deref_trait` | 1.2.1 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/stable_deref_trait/1.2.1 |
| 111 | `tao` | 0.37.1 | `Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/tao/0.37.1 |
| 112 | `tauri` | 2.12.4 | `Apache-2.0 OR MIT` | 是 | 已包含（2 文件） | https://crates.io/crates/tauri/2.12.4 |
| 113 | `tauri-plugin-single-instance` | 2.5.4 | `Apache-2.0 OR MIT` | 是 | 已包含（3 文件） | https://crates.io/crates/tauri-plugin-single-instance/2.5.4 |
| 114 | `tauri-runtime` | 2.12.2 | `Apache-2.0 OR MIT` | 是 | 已包含（2 文件） | https://crates.io/crates/tauri-runtime/2.12.2 |
| 115 | `tauri-runtime-wry` | 2.12.2 | `Apache-2.0 OR MIT` | 是 | 已包含（2 文件） | https://crates.io/crates/tauri-runtime-wry/2.12.2 |
| 116 | `tauri-utils` | 2.10.2 | `Apache-2.0 OR MIT` | 是 | 已包含（2 文件） | https://crates.io/crates/tauri-utils/2.10.2 |
| 117 | `thiserror` | 2.0.21 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/thiserror/2.0.21 |
| 118 | `time` | 0.3.55 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/time/0.3.55 |
| 119 | `time-core` | 0.1.9 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/time-core/0.1.9 |
| 120 | `tinystr` | 0.8.4 | `Unicode-3.0` | 是 | 已包含（1 文件） | https://crates.io/crates/tinystr/0.8.4 |
| 121 | `tokio` | 1.53.3 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/tokio/1.53.3 |
| 122 | `toml` | 1.1.8+spec-1.1.0 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/toml/1.1.8%2Bspec-1.1.0 |
| 123 | `toml_datetime` | 1.1.2+spec-1.1.0 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/toml_datetime/1.1.2%2Bspec-1.1.0 |
| 124 | `toml_parser` | 1.1.5+spec-1.1.0 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/toml_parser/1.1.5%2Bspec-1.1.0 |
| 125 | `toml_writer` | 1.1.3+spec-1.1.0 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/toml_writer/1.1.3%2Bspec-1.1.0 |
| 126 | `tracing` | 0.1.44 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/tracing/0.1.44 |
| 127 | `tracing-core` | 0.1.36 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/tracing-core/0.1.36 |
| 128 | `tray-icon` | 0.25.1 | `MIT OR Apache-2.0` | 是 | 已包含（3 文件） | https://crates.io/crates/tray-icon/0.25.1 |
| 129 | `typeid` | 1.0.3 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/typeid/1.0.3 |
| 130 | `unicode-segmentation` | 1.13.3 | `MIT OR Apache-2.0` | 是 | 已包含（3 文件） | https://crates.io/crates/unicode-segmentation/1.13.3 |
| 131 | `url` | 2.5.8 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/url/2.5.8 |
| 132 | `urlpattern` | 0.6.0 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/urlpattern/0.6.0 |
| 133 | `utf8_iter` | 1.0.4 | `Apache-2.0 OR MIT` | 是 | 已包含（3 文件） | https://crates.io/crates/utf8_iter/1.0.4 |
| 134 | `uuid` | 1.28.0 | `Apache-2.0 OR MIT` | 是 | 已包含（2 文件） | https://crates.io/crates/uuid/1.28.0 |
| 135 | `walkdir` | 2.5.0 | `Unlicense/MIT` | 是 | 已包含（3 文件） | https://crates.io/crates/walkdir/2.5.0 |
| 136 | `wasapi` | 0.26.0 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/wasapi/0.26.0 |
| 137 | `web-time` | 1.1.0 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/web-time/1.1.0 |
| 138 | `webview2-com` | 0.39.1 | `MIT` | 是 | 缺失（0 文件；见 §3.3） | https://crates.io/crates/webview2-com/0.39.1 |
| 139 | `webview2-com-sys` | 0.39.1 | `MIT` | 是 | 缺失（0 文件；见 §3.3） | https://crates.io/crates/webview2-com-sys/0.39.1 |
| 140 | `winapi-util` | 0.1.11 | `Unlicense OR MIT` | 是 | 已包含（3 文件） | https://crates.io/crates/winapi-util/0.1.11 |
| 141 | `window-vibrancy` | 0.8.1 | `Apache-2.0 OR MIT` | 是 | 已包含（3 文件） | https://crates.io/crates/window-vibrancy/0.8.1 |
| 142 | `windowfunctions` | 0.1.1 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/windowfunctions/0.1.1 |
| 143 | `windows` | 0.62.2 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/windows/0.62.2 |
| 144 | `windows-capture` | 2.0.1 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/windows-capture/2.0.1 |
| 145 | `windows-collections` | 0.3.2 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/windows-collections/0.3.2 |
| 146 | `windows-core` | 0.62.2 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/windows-core/0.62.2 |
| 147 | `windows-future` | 0.3.2 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/windows-future/0.3.2 |
| 148 | `windows-link` | 0.2.1 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/windows-link/0.2.1 |
| 149 | `windows-numerics` | 0.3.1 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/windows-numerics/0.3.1 |
| 150 | `windows-result` | 0.4.1 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/windows-result/0.4.1 |
| 151 | `windows-strings` | 0.5.1 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/windows-strings/0.5.1 |
| 152 | `windows-sys` | 0.61.2 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/windows-sys/0.61.2 |
| 153 | `windows-threading` | 0.2.1 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/windows-threading/0.2.1 |
| 154 | `windows-version` | 0.1.7 | `MIT OR Apache-2.0` | 是 | 已包含（2 文件） | https://crates.io/crates/windows-version/0.1.7 |
| 155 | `winnow` | 1.0.4 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/winnow/1.0.4 |
| 156 | `writeable` | 0.6.4 | `Unicode-3.0` | 是 | 已包含（1 文件） | https://crates.io/crates/writeable/0.6.4 |
| 157 | `wry` | 0.57.0 | `Apache-2.0 OR MIT` | 是 | 已包含（3 文件） | https://crates.io/crates/wry/0.57.0 |
| 158 | `yoke` | 0.8.3 | `Unicode-3.0` | 是 | 已包含（1 文件） | https://crates.io/crates/yoke/0.8.3 |
| 159 | `zerofrom` | 0.1.8 | `Unicode-3.0` | 是 | 已包含（1 文件） | https://crates.io/crates/zerofrom/0.1.8 |
| 160 | `zerotrie` | 0.2.5 | `Unicode-3.0` | 是 | 已包含（1 文件） | https://crates.io/crates/zerotrie/0.2.5 |
| 161 | `zerovec` | 0.11.8 | `Unicode-3.0` | 是 | 已包含（1 文件） | https://crates.io/crates/zerovec/0.11.8 |
| 162 | `zmij` | 1.0.23 | `MIT` | 是 | 已包含（1 文件） | https://crates.io/crates/zmij/1.0.23 |

## 3. 专项说明

### 3.1 MPL-2.0：`option-ext 0.2.0`

| 项目 | 内容 |
|---|---|
| package / version | `option-ext` / `0.2.0` |
| license expression | `MPL-2.0` |
| 进入路径 | `tauri → dirs → dirs-sys → option-ext` |
| 本地许可文本 | 包内 `LICENSE.txt`（MPL-2.0），已拼入 `third-party-licenses.txt` |
| crates.io | https://crates.io/crates/option-ext/0.2.0 |
| 精确版本下载 | https://crates.io/api/v1/crates/option-ext/0.2.0/download |
| 上游仓库 | https://github.com/soc/option-ext.git |
| SHA-256（Cargo.lock） | `04744f49eae99ab78e0d5c0b603ab218f515ea8cfe5a456d7629ad883a3b6e7d` |
| 本地修改 | 本仓库未发现 `vendor/`、`[patch]` 或本地替换；按 crates.io registry 原包纳入（基于仓库配置与 package source 的静态核对） |

**源码获取说明（待法律审阅确认形式）**：MPL-2.0 通常要求使 MPL 覆盖文件的源码可获得。本候选记录了 crates.io 的精确版本、下载 URL、上游仓库与 SHA-256，但**尚未决定**采用“随包附源码”“可下载归档 URL”还是“书面要约”，也**未**在安装包中提供该说明。该形式需由律师结合发行方式确认。

**未修改声明**：没有发现该 crate 的本地补丁或 vendored 副本；若后续引入补丁，应按 MPL-2.0 的要求补充修改文件源码与说明。此段是工程事实记录，不是法律结论。

### 3.2 Apache-2.0 包与 NOTICE

本候选对“Apache 包”给出两个口径：

**严格口径（Apache-2.0 为唯一许可，或为 AND 组合的一部分）**

| package | version | license expression | 本地文件 | NOTICE 情况 |
|---|---|---|---|---|
| `dpi` | 0.1.2 | `Apache-2.0 AND MIT` | LICENSE, LICENSE-LIBM-MIT | 无 NOTICE* 文件 |
| `tao` | 0.37.1 | `Apache-2.0` | LICENSE, LICENSE.spdx | 无 NOTICE* 文件 |

- `tao 0.37.1`：Apache-2.0 only；本地包内有 `LICENSE`（Apache-2.0 全文）与 `LICENSE.spdx`（SPDX tag-value 元数据，含 `PackageCopyrightText`），**未发现 NOTICE 文件**。`LICENSE.spdx` 不是许可全文，已作为补充文件拼入 `third-party-licenses.txt`。
- `dpi 0.1.2`：`Apache-2.0 AND MIT`；本地包内有 `LICENSE` 与 `LICENSE-LIBM-MIT`，**未发现 NOTICE 文件**。

**宽口径（表达式含 Apache-2.0，共 116 个）**

116 个包的本地 registry 内容中均未发现文件名匹配 `NOTICE*` 的文件；完整名单如下：

<details>
<summary>116 个含 Apache-2.0 的 runtime 包（本地无 NOTICE* 文件）</summary>

`adler2`, `anyhow`, `audio-codec-algorithms`, `audioadapter`, `audioadapter-buffers`, `audioadapter-sample`, `base64`, `bitflags`, `bytemuck`, `cfg-if`, `cookie`, `crc32fast`, `crossbeam-channel`, `crossbeam-deque`, `crossbeam-epoch`, `crossbeam-utils`, `ctor`, `deranged`, `dirs`, `dirs-sys`, `dpi`, `dunce`, `either`, `equivalent`, `erased-serde`, `fast_image_resize`, `fdeflate`, `flate2`, `fnv`, `form_urlencoded`, `getrandom`, `glob`, `hashbrown`, `heck`, `http`, `idna`, `idna_adapter`, `image`, `indexmap`, `itoa`, `json-patch`, `jsonptr`, `keyboard-types`, `libc`, `lock_api`, `log`, `mime`, `miniz_oxide 0.8.9`, `miniz_oxide 0.9.1`, `moxcms`, `muda`, `num-conv`, `num-integer`, `num-traits`, `once_cell`, `parking_lot`, `parking_lot_core`, `percent-encoding`, `pin-project-lite`, `png`, `powerfmt`, `pxfm`, `raw-window-handle`, `rayon`, `rayon-core`, `regex`, `regex-automata`, `regex-syntax`, `rubato`, `scopeguard`, `semver`, `serde`, `serde-untagged`, `serde_core`, `serde_json`, `serde_spanned`, `serde_with`, `serialize-to-javascript`, `siphasher`, `smallvec`, `socket2`, `softbuffer`, `stable_deref_trait`, `tao`, `tauri`, `tauri-plugin-single-instance`, `tauri-runtime`, `tauri-runtime-wry`, `tauri-utils`, `thiserror`, `time`, `time-core`, `toml`, `toml_datetime`, `toml_parser`, `toml_writer`, `tray-icon`, `typeid`, `unicode-segmentation`, `url`, `utf8_iter`, `uuid`, `web-time`, `window-vibrancy`, `windows`, `windows-collections`, `windows-core`, `windows-future`, `windows-link`, `windows-numerics`, `windows-result`, `windows-strings`, `windows-sys`, `windows-threading`, `windows-version`, `wry`

</details>

> 事实边界：这里只检查了**本地 registry 包内的文件名**，没有联网核对上游仓库，也没有逐文件检索 NOTICE 文本。Apache-2.0 §4(d) 是否要求额外保留 NOTICE、`tao`/`dpi` 是否有上游 NOTICE，需法律审阅与上游核对。即使 `cargo deny check licenses` 通过，也不能替代这项核对。

### 3.3 本地缺失或仅部分覆盖的许可文本

**本地无许可/通知文件（3 个）**

| package | version | license expression | 本地 registry 包目录名 | 本地文件 | 处理 |
|---|---|---|---|---|---|
| `alloc-stdlib` | 0.3.0 | `BSD-3-Clause` | `alloc-stdlib-0.3.0` | 无 | 标记“本地未找到，需向上游获取”；未用近似文本替代 |
| `webview2-com` | 0.39.1 | `MIT` | `webview2-com-0.39.1` | 无 | 标记“本地未找到，需向上游获取”；未用近似文本替代 |
| `webview2-com-sys` | 0.39.1 | `MIT` | `webview2-com-sys-0.39.1` | 无 | 标记“本地未找到，需向上游获取”；未用近似文本替代 |

说明：

- `alloc-stdlib 0.3.0`：本地包内没有许可文件；**没有**用同系列 `alloc-no-stdlib` 的 BSD 文本替代。
- `webview2-com 0.39.1`、`webview2-com-sys 0.39.1`：本地包内没有许可文件；**没有**用其他 Microsoft/Tauri 包的 MIT 文本替代。
- 上述三项在 `third-party-licenses.txt` 中明确写为“本地未找到，需向上游获取”；在 SBOM 中 `licenseDeclared` 保留 Cargo.toml 声明值，`licenseConcluded` 为 `NOASSERTION`。

**本地有文本但未覆盖全部 OR 替代项（2 个）**

| package | version | license expression | 本地文件 | 自动检出许可族 | 缺口 |
|---|---|---|---|---|---|
| `brotli-decompressor` | 6.0.1 | `BSD-3-Clause/MIT` | `LICENSE` | BSD-3-Clause | 表达式还含 MIT；本地未检出对应文本 |
| `dunce` | 1.0.5 | `CC0-1.0 OR MIT-0 OR Apache-2.0` | `LICENSE` | CC0-1.0 | 表达式还含 Apache-2.0、MIT-0；本地未检出对应文本 |

> 这两个表达式都是 `OR` 组合；按某一许可路径可能只需其中一份文本，但本候选没有据此判断“已满足”。是否需要在通知中补齐全部替代文本，待法律审阅。

### 3.4 非规范 SPDX 斜杠表达式（5 个）

Cargo.toml 原值中的 `/` 不是规范 SPDX 写法。本文件保留原值；SBOM 中按 `OR` 映射，并在该包的 `licenseComments` 中记录原值。映射如下：

| package | version | Cargo.toml 原值 | SBOM `licenseDeclared` |
|---|---|---|---|
| `brotli-decompressor` | 6.0.1 | `BSD-3-Clause/MIT` | `BSD-3-Clause OR MIT` |
| `fnv` | 1.0.7 | `Apache-2.0 / MIT` | `Apache-2.0 OR MIT` |
| `json-patch` | 4.2.0 | `MIT/Apache-2.0` | `MIT OR Apache-2.0` |
| `same-file` | 1.0.6 | `Unlicense/MIT` | `Unlicense OR MIT` |
| `walkdir` | 2.5.0 | `Unlicense/MIT` | `Unlicense OR MIT` |

> 该映射是工程侧为生成 SBOM 做的规范化，不改变上游声明；若律师认为应按其他方式解释，需回改 SBOM。

### 3.5 未计入的包

- **workspace 自有 crate（8 个）**：`screen-rec`、`sr-audio`、`sr-capture`、`sr-core`、`sr-enum`、`sr-merge`、`sr-pipeline`、`sr-shot`。它们是第一方代码，不计入第三方 notices；其自身 `MIT OR Apache-2.0` 许可文本不在本文件范围。
- **build-only（74 个）**：在 `cargo tree -e normal` 的 244 个 target-resolved 包中，除 162 个第三方 runtime 与 8 个 workspace 外，剩余 74 个没有 target runtime 单元，包含 proc-macro 与仅由 build feature/构建依赖到达的包。代表例子：`tauri-build`、`cargo_metadata`、`schemars`、`dom_query`、`cssparser`、`selectors`、`dtoa-short`、`foldhash`。它们不进入本发行 notices；**不能**把其中的 MPL 包误列为发行物。
- **dev/test**：`sr-itests`、`rustfft` 等测试依赖不计入。
- **非 Rust 组件**：见 §0.2。

## 4. 配套文件与核对

### 4.1 `third-party-licenses.txt`

- 按 package/version 分段，共 162 段；每段有 `PACKAGE:`、license expression、crates.io 来源、本地文件名和分隔线。
- 159 个包至少拼入 1 个本地文件；3 个包明确标记“本地未找到，需向上游获取”。
- 文本逐字取自本地 cargo registry 包内文件，仅统一换行符；未联网、未用近似文本替代。

### 4.2 `sbom-rust.json`

- 格式：SPDX 2.3 JSON。
- 包数：162（仅第三方 runtime；不含 workspace 根包）。
- 每个包包含 `SPDXID`、`versionInfo`、`licenseDeclared`、crates.io/PURL `externalRefs`、SHA-256、`downloadLocation`。
- `licenseConcluded` 与 `copyrightText` 统一为 `NOASSERTION`；未做法律结论。
- 非规范 `/` 表达式在 `licenseDeclared` 中规范化为 `OR`，原值写入 `licenseComments`。
- 缺失文本的包在 `licenseComments` 中标注，未用其他包文本替代。

### 4.3 核对命令（工程自检）

```powershell
python -c "import json; d=json.load(open('docs/compliance/sbom-rust.json', encoding='utf-8')); print(len(d['packages']))"
# 期望输出 162

# 统计 notices 表格行、licenses 分段、SBOM 包数并用 Python 比对（脚本不随仓库提交）
```

> 本候选的验证 owner 是生成 lane；编排者需独立复核。工程自检只证明文件可解析、数量一致、文本非空，不证明法律义务已满足。

## 附录 A：本地许可/通知文件清单

| package | version | 本地文件（registry 包内） |
|---|---|---|
| `adler2` | 2.0.1 | `LICENSE-0BSD`, `LICENSE-APACHE`, `LICENSE-MIT` |
| `aho-corasick` | 1.1.5 | `COPYING`, `LICENSE-MIT`, `UNLICENSE` |
| `alloc-no-stdlib` | 3.0.0 | `LICENSE` |
| `alloc-stdlib` | 0.3.0 | —（本地未找到） |
| `anyhow` | 1.0.104 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `audio-codec-algorithms` | 0.8.1 | `LICENSE-0BSD`, `LICENSE-APACHE` |
| `audioadapter` | 5.0.0 | `LICENSE-APACHE`, `LICENSE-MIT`, `LICENSE.txt` |
| `audioadapter-buffers` | 5.2.0 | `LICENSE-APACHE`, `LICENSE-MIT`, `LICENSE.txt` |
| `audioadapter-sample` | 5.2.0 | `LICENSE-APACHE`, `LICENSE-MIT`, `LICENSE.txt` |
| `base64` | 0.23.1 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `bitflags` | 2.13.2 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `brotli` | 9.0.0 | `LICENSE.BSD-3-Clause`, `LICENSE.MIT` |
| `brotli-decompressor` | 6.0.1 | `LICENSE` |
| `bytemuck` | 1.25.2 | `LICENSE-APACHE`, `LICENSE-MIT`, `LICENSE-ZLIB` |
| `byteorder-lite` | 0.1.0 | `LICENSE-MIT`, `UNLICENSE` |
| `bytes` | 1.12.1 | `LICENSE` |
| `cfb` | 0.14.0 | `LICENSE` |
| `cfg-if` | 1.0.5 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `cookie` | 0.18.2 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `crc32fast` | 1.5.2 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `crossbeam-channel` | 0.5.17 | `LICENSE-APACHE`, `LICENSE-MIT`, `LICENSE-THIRD-PARTY` |
| `crossbeam-deque` | 0.8.8 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `crossbeam-epoch` | 0.9.21 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `crossbeam-utils` | 0.8.23 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `ctor` | 1.0.13 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `deranged` | 0.5.9 | `LICENSE-Apache`, `LICENSE-MIT` |
| `dirs` | 7.0.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `dirs-sys` | 0.5.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `dpi` | 0.1.2 | `LICENSE`, `LICENSE-LIBM-MIT` |
| `dunce` | 1.0.5 | `LICENSE` |
| `either` | 1.19.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `equivalent` | 1.0.3 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `erased-serde` | 0.4.10 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `fast_image_resize` | 5.5.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `fdeflate` | 0.3.7 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `flate2` | 1.1.10 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `fnv` | 1.0.7 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `form_urlencoded` | 1.2.2 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `getrandom` | 0.3.4 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `glob` | 0.3.4 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `hashbrown` | 0.17.1 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `heck` | 0.5.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `http` | 1.5.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `icu_collections` | 2.3.0 | `LICENSE` |
| `icu_locale_core` | 2.3.0 | `LICENSE` |
| `icu_normalizer` | 2.3.0 | `LICENSE` |
| `icu_normalizer_data` | 2.3.0 | `LICENSE` |
| `icu_properties` | 2.3.0 | `LICENSE` |
| `icu_properties_data` | 2.3.0 | `LICENSE` |
| `icu_provider` | 2.3.1 | `LICENSE` |
| `idna` | 1.1.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `idna_adapter` | 1.2.2 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `image` | 0.25.10 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `indexmap` | 2.14.2 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `infer` | 0.22.0 | `LICENSE` |
| `itoa` | 1.0.18 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `json-patch` | 4.2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `jsonptr` | 0.7.1 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `keyboard-types` | 0.8.3 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `libc` | 0.2.190 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `litemap` | 0.8.3 | `LICENSE` |
| `lock_api` | 0.4.14 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `log` | 0.4.34 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `memchr` | 2.8.3 | `COPYING`, `LICENSE-MIT`, `UNLICENSE` |
| `mime` | 0.3.17 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `miniz_oxide` | 0.8.9 | `LICENSE`, `LICENSE-APACHE.md`, `LICENSE-MIT.md`, `LICENSE-ZLIB.md` |
| `miniz_oxide` | 0.9.1 | `LICENSE`, `LICENSE-APACHE.md`, `LICENSE-MIT.md`, `LICENSE-ZLIB.md` |
| `mio` | 1.2.4 | `LICENSE` |
| `moxcms` | 0.8.1 | `LICENSE-APACHE.md`, `LICENSE.md` |
| `muda` | 0.20.0 | `LICENSE-APACHE`, `LICENSE-MIT`, `LICENSE.spdx` |
| `num-conv` | 0.2.2 | `LICENSE-Apache`, `LICENSE-MIT` |
| `num-integer` | 0.1.47 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `num-traits` | 0.2.19 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `once_cell` | 1.21.4 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `option-ext` | 0.2.0 | `LICENSE.txt` |
| `parking_lot` | 0.12.5 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `parking_lot_core` | 0.9.12 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `percent-encoding` | 2.3.2 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `phf` | 0.13.1 | `LICENSE` |
| `phf_shared` | 0.13.1 | `LICENSE` |
| `pin-project-lite` | 0.2.17 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `plist` | 1.10.1 | `LICENCE` |
| `png` | 0.18.1 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `potential_utf` | 0.1.6 | `LICENSE` |
| `powerfmt` | 0.2.1 | `LICENSE-Apache`, `LICENSE-MIT` |
| `pxfm` | 0.1.30 | `LICENSE-APACHE.md`, `LICENSE.md` |
| `quick-xml` | 0.42.0 | `LICENSE-MIT.md` |
| `raw-window-handle` | 0.6.2 | `LICENSE-APACHE.md`, `LICENSE-MIT.md`, `LICENSE-ZLIB.md` |
| `rayon` | 1.12.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `rayon-core` | 1.13.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `regex` | 1.13.1 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `regex-automata` | 0.4.18 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `regex-syntax` | 0.8.11 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `rubato` | 5.0.1 | `LICENSE-APACHE`, `LICENSE-MIT`, `LICENSE.txt` |
| `same-file` | 1.0.6 | `COPYING`, `LICENSE-MIT`, `UNLICENSE` |
| `scopeguard` | 1.2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `semver` | 1.0.28 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `serde` | 1.0.229 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `serde-untagged` | 0.1.9 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `serde_core` | 1.0.229 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `serde_json` | 1.0.154 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `serde_spanned` | 1.1.2 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `serde_with` | 3.24.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `serialize-to-javascript` | 0.1.2 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `simd-adler32` | 0.3.10 | `LICENSE.md` |
| `siphasher` | 1.0.4 | `COPYING`, `LICENSE-APACHE`, `LICENSE-MIT` |
| `smallvec` | 1.16.3 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `socket2` | 0.6.5 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `softbuffer` | 0.4.8 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `stable_deref_trait` | 1.2.1 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `tao` | 0.37.1 | `LICENSE`, `LICENSE.spdx` |
| `tauri` | 2.12.4 | `LICENSE-APACHE-2.0`, `LICENSE-MIT` |
| `tauri-plugin-single-instance` | 2.5.4 | `LICENSE.spdx`, `LICENSE_APACHE-2.0`, `LICENSE_MIT` |
| `tauri-runtime` | 2.12.2 | `LICENSE-APACHE-2.0`, `LICENSE-MIT` |
| `tauri-runtime-wry` | 2.12.2 | `LICENSE-APACHE-2.0`, `LICENSE-MIT` |
| `tauri-utils` | 2.10.2 | `LICENSE-APACHE-2.0`, `LICENSE-MIT` |
| `thiserror` | 2.0.21 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `time` | 0.3.55 | `LICENSE-Apache`, `LICENSE-MIT` |
| `time-core` | 0.1.9 | `LICENSE-Apache`, `LICENSE-MIT` |
| `tinystr` | 0.8.4 | `LICENSE` |
| `tokio` | 1.53.3 | `LICENSE` |
| `toml` | 1.1.8+spec-1.1.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `toml_datetime` | 1.1.2+spec-1.1.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `toml_parser` | 1.1.5+spec-1.1.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `toml_writer` | 1.1.3+spec-1.1.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `tracing` | 0.1.44 | `LICENSE` |
| `tracing-core` | 0.1.36 | `LICENSE` |
| `tray-icon` | 0.25.1 | `LICENSE-APACHE`, `LICENSE-MIT`, `LICENSE.spdx` |
| `typeid` | 1.0.3 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `unicode-segmentation` | 1.13.3 | `COPYRIGHT`, `LICENSE-APACHE`, `LICENSE-MIT` |
| `url` | 2.5.8 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `urlpattern` | 0.6.0 | `LICENSE` |
| `utf8_iter` | 1.0.4 | `COPYRIGHT`, `LICENSE-APACHE`, `LICENSE-MIT` |
| `uuid` | 1.28.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `walkdir` | 2.5.0 | `COPYING`, `LICENSE-MIT`, `UNLICENSE` |
| `wasapi` | 0.26.0 | `LICENSE.txt` |
| `web-time` | 1.1.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| `webview2-com` | 0.39.1 | —（本地未找到） |
| `webview2-com-sys` | 0.39.1 | —（本地未找到） |
| `winapi-util` | 0.1.11 | `COPYING`, `LICENSE-MIT`, `UNLICENSE` |
| `window-vibrancy` | 0.8.1 | `LICENSE-APACHE`, `LICENSE-MIT`, `LICENSE.spdx` |
| `windowfunctions` | 0.1.1 | `LICENSE.txt` |
| `windows` | 0.62.2 | `license-apache-2.0`, `license-mit` |
| `windows-capture` | 2.0.1 | `LICENCE` |
| `windows-collections` | 0.3.2 | `license-apache-2.0`, `license-mit` |
| `windows-core` | 0.62.2 | `license-apache-2.0`, `license-mit` |
| `windows-future` | 0.3.2 | `license-apache-2.0`, `license-mit` |
| `windows-link` | 0.2.1 | `license-apache-2.0`, `license-mit` |
| `windows-numerics` | 0.3.1 | `license-apache-2.0`, `license-mit` |
| `windows-result` | 0.4.1 | `license-apache-2.0`, `license-mit` |
| `windows-strings` | 0.5.1 | `license-apache-2.0`, `license-mit` |
| `windows-sys` | 0.61.2 | `license-apache-2.0`, `license-mit` |
| `windows-threading` | 0.2.1 | `license-apache-2.0`, `license-mit` |
| `windows-version` | 0.1.7 | `license-apache-2.0`, `license-mit` |
| `winnow` | 1.0.4 | `LICENSE-MIT` |
| `writeable` | 0.6.4 | `LICENSE` |
| `wry` | 0.57.0 | `LICENSE-APACHE`, `LICENSE-MIT`, `LICENSE.spdx` |
| `yoke` | 0.8.3 | `LICENSE` |
| `zerofrom` | 0.1.8 | `LICENSE` |
| `zerotrie` | 0.2.5 | `LICENSE` |
| `zerovec` | 0.11.8 | `LICENSE` |
| `zmij` | 1.0.23 | `LICENSE-MIT` |

---

**再次声明**：本文件是通知候选，需法律审阅；未接入安装包 About 页，不构成合规或审计结论。
