# M1-A 骨架引导（workspace + 最小 Tauri 壳）

- 日期: 2026-10-16
- 规格: specs/001-screen-recorder.md §3.1 / §3.2 / §2.8（Approved 2026-10-15）
- 范围: 仅结构与占位，不实现任何功能逻辑（功能由后续 TDD 轮次完成，见 §5 里程碑表）

## 1. 变更清单（全部新增）

```text
Cargo.toml                     # workspace（resolver=2, edition 2021, members ×10, workspace.package: version/edition/license/publish）
rust-toolchain.toml            # channel 1.99.0 + rustfmt/clippy（REQ-29 工具链锁定）
ui/index.html                  # 静态占位页（frontendDist；无 Node 构建链）
src-tauri/Cargo.toml           # tauri = "2.12.4"；tauri-build = "2"；lib screen_rec_lib(rlib)
src-tauri/build.rs             # tauri_build::build()
src-tauri/tauri.conf.json      # identifier com.screenrec.desktop；frontendDist ../ui；bundle.active=false
src-tauri/src/main.rs          # 调用 run()（release 隐藏控制台）
src-tauri/src/lib.rs           # 最小 run()：Builder::default().run(generate_context!())，无托盘/单实例/设置/commands
src-tauri/icons/icon.ico       # 32x32 占位图标（tauri-build 生成 Windows 资源的硬性要求）
crates/sr-core/                # 类型/错误/状态 占位（types, error, state）
crates/sr-enum/                # 枚举与身份解析 占位（windows, monitors, audio_sessions, identity）
crates/sr-capture/             # WGC 采集 占位（wgc, pacer, canvas, pipe）
crates/sr-audio/               # WASAPI 音频 占位（wasapi, slots, resample, wav）
crates/sr-pipeline/            # sidecar 管线 占位（sidecar, queue, segments, finalize, recovery, naming）
crates/sr-shot/                # 截图 占位（screenshot, compose, png）
crates/sr-merge/               # 合并 占位（inputs, merge）
tests/sr-itests/               # L2 集成测试容器（占位 lib，无用例）
tools/tone_player/             # 测试助手占位（tone, flash）
docs/milestones/m1-bootstrap.md  # 本证据文件
```

各 `src/lib.rs` 仅含 crate 文档 + `pub mod` 空模块 + TODO 注释（引用规格 REQ/§编号与对应 §4 测试名），无任何逻辑；未触碰 specs/、docs/ 既有内容、tools/spikes/、tools/ffmpeg/、tools/rust/。

## 2. 骨架内决策记录

| # | 决策 | 依据 |
|---|---|---|
| B1 | identifier = `com.screenrec.desktop`（任务示例为 com.screenrec.app） | Tauri v2 校验禁止以 `.app` 结尾的 identifier（macOS bundle 后缀保留），按任务「如」的示例性质取安全值 |
| B2 | 新增 `src-tauri/icons/icon.ico`（32x32 单色占位，经 System.Drawing 生成，766B） | tauri-build 在 Windows 编译资源文件时硬性要求该文件，缺失即构建失败（首次 check 已实测复现） |
| B3 | `bundle.active = false` | M1-A 仅需编译壳，不打包；打包配置随后续里程碑补充 |
| B4 | workspace.package 统一 version=0.1.0 / edition=2021 / license="MIT OR Apache-2.0" / publish=false | 任务要求基础字段；发布许可红线（REQ-29）另行审计，不影响 v0.1.0 内部版本 |
| B5 | license 暂取双许可占位 | 尚无发行决策；REQ-29 许可筛查（M0/M8）前不作承诺，届时空 crate 重命名字段无成本 |

## 3. 验证证据（实际执行，2026-10-16）

环境：Windows x64（MSI），rustup 已按 rust-toolchain.toml 激活 `1.99.0-x86_64-pc-windows-msvc`，MSVC BuildTools 2022 可链接。

| 命令 | 结果 |
|---|---|
| `cargo metadata --no-deps` | workspace_members ×10（7 库 crate + src-tauri + sr-itests + tone-player），清单解析通过 |
| `cargo check --workspace` | **通过**，`Finished dev profile`，exit 0；**0 warning**（首次运行曾因缺 icon.ico 失败，补 B2 后通过） |
| `cargo test --workspace` | **通过**，exit 0；21 个套件全部 `test result: ok. 0 passed; 0 failed`（10 库单测 + 10 lib 文档测试 + 1 bin 目标），无失败 |

注：Tauri 2.12.4 首次编译约 407 个依赖 crate，下载+编译耗时数分钟，缓存于 target/（已 gitignore）。

## 4. 验收对照

| 项 | 状态 |
|---|---|
| workspace 结构符合 §3.2（10 成员、模块占位与职责表一一对应） | ✅ |
| `cargo check --workspace` / `cargo test --workspace` 通过且 0 失败 | ✅ |
| Tauri 2.12.4 壳最小编译通过（未运行，按任务要求无需运行） | ✅ |
| 无功能实现（仅 pub mod 空占位 + TODO；无托盘/单实例/设置/commands） | ✅ |
| 未改动 specs/、docs/ 既有内容、tools/spikes/、tools/ffmpeg/、tools/rust/ | ✅ |
| 证据落盘（本文件） | ✅ |

## 5. 后续（不在本任务范围）

- M1 后续任务：REQ-1..4 枚举（TDD 红→绿）、REQ-30 托盘/单实例/设置、REQ-31 Channel 订阅。
- 打包图标替换为正式产品图标（当前为占位）。
