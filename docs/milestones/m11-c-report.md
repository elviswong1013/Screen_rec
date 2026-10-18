# M11-C 验收整改报告（发行打包 · P1-5）

- 日期: 2026-10-17
- 角色: **executor-backup**（按 acceptor P1-5 整改；编排者独立复核）
- 范围: 运行时 ffmpeg/ffprobe 定位、Tauri bundle 配置、许可证随包、解析单测与安装验证
- 约束: 未 git 提交；未改 `specs/`、`docs/spikes/`；未修改既有测试断言；生产改动外科手术式

## 0. 结论

| 项 | 状态 | 关键证据 |
|---|---|---|
| 1. 运行时资源解析（统一 `resolve_tool_binary`） | **完成** | `sr-core/src/tools.rs:117/154`；sidecar（`sidecar.rs:100/107/193`）、merge（`merge.rs:134/281/307`）、finalize/probe/recovery 全部改用它；失败错误逐条列出尝试路径 |
| 2. 打包配置（bundle + resources） | **完成** | `src-tauri/tauri.conf.json:23–33`：`active=true`、`targets=["nsis"]`、ffmpeg/ffprobe + licenses 映射；`identifier`/图标保持 |
| 3. 验证 | **全绿** | `cargo check --workspace` 0 warning；`cargo test --workspace` **50 binaries / 44 passed / 0 failed / 42 ignored**；解析单测 6 passed；release 构建 + **实际 NSIS 打包成功**（82.48 MiB）+ 静默安装 + 安装后 `--self-check` 通过 |
| 4. 许可证随包 | **完成** | `src-tauri/resources/licenses/`：FFmpeg LICENSE 副本（SHA256 与构建包原文一致）+ 源码获取说明；安装目录 `licenses/` 实测存在 |
| 发行包不依赖源码目录 | **已实测** | 隐藏仓库 `tools/.../bin` 后，安装目录 `screen-rec.exe --self-check` 仍以 `exe-adjacent` 命中自带 ffmpeg/ffprobe，`-version` 正常、退出码 0 |

## 1. 运行时资源解析（P1-5 核心）

### 1.1 统一解析器

新增 `crates/sr-core/src/tools.rs`（`sr-core` 新模块 `tools`，`lib.rs` 注册）：

| 接口 | 位置 | 说明 |
|---|---|---|
| `FFMPEG_DIR_ENV = "SCREENREC_FFMPEG_DIR"` | `tools.rs:18` | ① 环境变量覆盖 |
| `resolve_with(&ToolSearchPaths, name)` | `tools.rs:117` | 纯解析：按优先级返回首个**存在的普通文件**；失败带全部候选 |
| `set_resource_dir` / `registered_resource_dir` | `tools.rs:138/143` | ② 宿主注册 Tauri `resource_dir` |
| `dev_fallback_dir()` | `tools.rs:148` | ④ 开发回退 `<repo>/tools/ffmpeg/n8.1/bin`（编译期常量） |
| `resolve_tool_binary(name)` | `tools.rs:154` | 运行时入口（env → resource → exe 同目录 → 开发回退） |
| `current_search_paths()` | `tools.rs:159` | 诊断/自检展示实际搜索根 |
| `self_check()` | `tools.rs:175` | `--self-check`：打印搜索根/命中来源并执行 `<tool> -version` |

候选布局（每层同时接受根目录与其子目录，避免安装布局歧义）：

| 优先级 | 来源 | 候选 |
|---|---|---|
| ① | `SCREENREC_FFMPEG_DIR` | `<dir>/<name>.exe`、`<dir>/bin/<name>.exe` |
| ② | Tauri `resource_dir` | `<dir>/<name>.exe`、`<dir>/ffmpeg/<name>.exe` |
| ③ | 当前 exe 同目录 | `<dir>/<name>.exe`、`<dir>/ffmpeg/<name>.exe` |
| ④ | 开发回退 | `<repo>/tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin/<name>.exe` |

失败返回 `ToolResolveError`，`Display` 形如：

```
无法定位 ffmpeg.exe（已尝试：
  - <env>/ffmpeg.exe
  - <env>/bin/ffmpeg.exe
  - <exe>/ffmpeg.exe
  - <exe>/ffmpeg/ffmpeg.exe
  - <repo>/tools/.../bin/ffmpeg.exe）
```

### 1.2 调用点统一

| 调用点 | 位置 | 变更 |
|---|---|---|
| sidecar 主入口 | `crates/sr-pipeline/src/sidecar.rs:100/107` | `ffmpeg_exe()/ffprobe_exe() -> Result<PathBuf,String>`，内部走 `resolve_tool_binary` |
| sidecar spawn | `sidecar.rs:193` | 解析失败 → `io::Error`（含完整尝试路径），不再静默用开发路径 |
| 编码器探测 | `sidecar.rs:120/140` | 解析失败 → 探测失败并打印原因 |
| 收尾时间轴 ffprobe | `crates/sr-pipeline/src/finalize.rs:95` | 统一解析；不可用时该段退回容器时长并打印 |
| 能力探测 | `crates/sr-pipeline/src/probe.rs:218` | 解析失败 → 各编码器 `init_ok=false`、detail 带错误 |
| 恢复扫描/收尾 | `crates/sr-pipeline/src/recovery.rs:486/555/621/658` | ffprobe/ffmpeg 均统一解析；扫描时不可用 → 段视为无效并仍列出会话 |
| 合并 sidecar | `crates/sr-merge/src/merge.rs:134/281/307` | 删除本地 `CARGO_MANIFEST_DIR` 路径，改用 `resolve_tool_binary` |
| 宿主注册 resource_dir | `src-tauri/src/lib.rs:106` | `app.path().resource_dir()` → `sr_core::tools::set_resource_dir` |
| 自检 CLI | `src-tauri/src/main.rs:6`、`src-tauri/src/lib.rs:91` | `--self-check` 在进入 Tauri 前执行（不影响正常启动） |

`sr-pipeline`/`sr-merge` 的 `Cargo.toml` 新增 workspace 内 `sr-core` 依赖（无新外部包）。

## 2. 打包配置

`src-tauri/tauri.conf.json:23–33`：

```json
"bundle": {
  "active": true,
  "targets": ["nsis"],
  "icon": ["icons/icon.ico"],
  "resources": {
    "../tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin/ffmpeg.exe": "ffmpeg/ffmpeg.exe",
    "../tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin/ffprobe.exe": "ffmpeg/ffprobe.exe",
    "resources/licenses/FFmpeg-LICENSE.txt": "licenses/FFmpeg-LICENSE.txt",
    "resources/licenses/ffmpeg-source-offer.txt": "licenses/ffmpeg-source-offer.txt"
  }
}
```

- `identifier`（`com.screenrec.desktop`）、`frontendDist`、主窗口配置保持不变；图标沿用
  `src-tauri/icons/icon.ico`。
- 只选 `nsis`（Windows 单安装器；避免同时依赖 WiX/MSI）。
- 资源目标目录：安装根下 `ffmpeg/`（二进制）与 `licenses/`（许可证/源码说明），与解析器
  ②/③ 布局一致。

## 3. 许可证随包（选择 `src-tauri/resources/licenses/`）

| 文件 | 来源/内容 |
|---|---|
| `src-tauri/resources/licenses/FFmpeg-LICENSE.txt` | 锁定构建包 `LICENSE.txt` **原样复制**（7651 B；SHA256 `DA7EABB7…44768` 与构建包一致） |
| `src-tauri/resources/licenses/ffmpeg-source-offer.txt` | LGPL 合规说明：构建来源 BtbN、n8.1 版本、zip/ffmpeg/ffprobe SHA256、FFmpeg n8.1 源码与构建脚本获取地址、独立进程调用声明、`SCREENREC_FFMPEG_DIR` 替换方式、构建配置审计项 |

选择 `src-tauri/resources/licenses/`（而非 `docs/third-party/`）的原因：该目录由 Tauri
`bundle.resources` 直接映射到安装包 `licenses/`，随安装包分发、无需构建脚本额外复制。

## 4. 验证（实际执行）

### 4.1 编译/单测

| 命令 | 结果 |
|---|---|
| `cargo check --workspace` | **Finished，0 warning / 0 error** |
| `cargo test --workspace` | **50 binaries：44 passed / 0 failed / 42 ignored**；唯一 warning 为既有 `tests/sr-itests/tests/probe.rs: unused import EncoderProbe`（非本轮） |
| `cargo test -p sr-core --test tool_resolution` | **6 passed** |

解析单测（`crates/sr-core/tests/tool_resolution.rs`）覆盖：
- ① env 覆盖优先于 resource/exe/dev（含 `bin/` 子目录）；
- ② resource 优先于 exe/dev；
- ③ 「模拟安装布局」`<app>/screen-rec.exe + <app>/ffmpeg/{ffmpeg,ffprobe}.exe`；
- ④ 开发回退最后命中且来源标记正确；
- 失败错误文本逐条包含所有候选路径；
- 仓库内开发回退文件真实存在。

### 4.2 release 构建

```
cargo build --release -p screen-rec
→ Finished release（1m 24s）
  target/release/screen-rec.exe                14,544,896 B
  target/release/ffmpeg/ffmpeg.exe            134,092,288 B
  target/release/ffmpeg/ffprobe.exe           133,887,488 B
  target/release/licenses/FFmpeg-LICENSE.txt        7,651 B
```
说明：`cargo build` 时 `tauri-build` 已按 `bundle.resources` 把资源复制到
`target/{debug,release}/ffmpeg|licenses`（实测存在），与解析器 ②/③ 布局一致。

### 4.3 实际打包（`cargo tauri` 不存在 → 使用 npx CLI）

```
$ cargo tauri --version
error: no such command: `tauri`
$ npx --yes @tauri-apps/cli@2.12.3 build
...
Built application at: ...\target\release\screen-rec.exe
Patching ... with bundle type information: nsis
Downloading nsis-3.13.zip / nsis_tauri_utils-v0.5.3.dll（校验 hash）
Running makensis to produce ...\target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe
Finished 1 bundle at: ...\Screen_rec_0.1.0_x64-setup.exe (82.48 MiB)
```
结果：**NSIS 安装包实际产出**（非伪报）。CLI 2.12.3 与 crate 2.12.4 的版本提示为 Info，
未影响构建。构建日志：`%TEMP%\m11c-tauri-build.log`。

### 4.4 安装后验证（真实安装 + 自检）

静默安装到临时目录（NSIS `/S /D=`），安装目录实测：

```
screen-rec.exe                14,540,288 B
uninstall.exe                     79,390 B
ffmpeg/ffmpeg.exe            134,092,288 B
ffmpeg/ffprobe.exe           133,887,488 B
licenses/FFmpeg-LICENSE.txt        7,651 B
licenses/ffmpeg-source-offer.txt   2,108 B
```

安装后 `screen-rec.exe --self-check`（重定向 stdout）：

```
[self-check] 搜索根（按优先级）
  env  SCREENREC_FFMPEG_DIR: (未设置)
  tauri resource_dir: (未设置)
  exe 同目录: C:\Users\elvis\AppData\Local\Temp\sr_m11c_install
  开发回退: C:\Users\elvis\...\crates\sr-core\../../tools/ffmpeg/.../bin
[self-check] ffmpeg: C:\Users\elvis\AppData\Local\Temp\sr_m11c_install\ffmpeg\ffmpeg.exe（来源：exe-adjacent）
[self-check]   -version: ffmpeg version n8.1.3-18-g46d899d7a0-20261015 ...
[self-check] ffprobe: C:\Users\elvis\AppData\Local\Temp\sr_m11c_install\ffmpeg\ffprobe.exe（来源：exe-adjacent）
[self-check]   -version: ffprobe version n8.1.3-18-g46d899d7a0-20261015 ...
EXIT=0
```

**源码目录独立性验证**：临时把仓库 `tools/ffmpeg/.../bin` 改名隐藏，重复运行安装目录
`--self-check` → 仍为 `来源：exe-adjacent`、`-version` 正常、`EXIT=0`；随后恢复目录
（`dev bin restored: True`）。

**环境变量覆盖验证**：`SCREENREC_FFMPEG_DIR=<repo>/tools/.../bin` → 两项均为
`来源：env:SCREENREC_FFMPEG_DIR`，`-version` 正常、`EXIT=0`。

**失败诊断验证**：空目录仅放 `screen-rec.exe`，隐藏仓库开发 bin，env 指向空目录 →
`EXIT=1`，stderr 逐条列出 env(2)/exe(2)/dev(1) 全部候选路径（resource_dir 未注册，
单测覆盖其候选）。

安装验证后已执行 `uninstall.exe /S` 清理（`install dir exists: False`）；安装包
`target\release\bundle\nsis\Screen_rec_0.1.0_x64-setup.exe` 保留作复核产物。

## 5. 未决 / 限制

1. **未做代码签名**：安装包为未签名 NSIS；生产分发需证书与签名步骤（本轮未配置）。
2. **② `resource_dir` 未经 GUI 上下文端到端点击验证**：`--self-check` 在 Tauri 启动前执行，
   没有 `AppHandle` 可调用 `resource_dir()`；其优先级由单测覆盖，且安装布局同时满足 ③
   （实测命中 exe-adjacent）。GUI 启动时注册见 `src-tauri/src/lib.rs:106`。
3. **无“干净源码机”实测**：开发回退路径仍编译进二进制（最后兜底），但在隐藏仓库 bin 后
   安装目录自检通过，证明实际不依赖它；仍未在完全无仓库的机器上跑过。
4. **WebView2 运行时前置**未在干净机审计（Tauri 默认安装器可能引导下载）；本机已有
   WebView2。
5. 安装包 82.48 MiB（ffmpeg + ffprobe 各 ~128 MiB 压缩后）；未做裁剪/分离下载策略
   （任务要求两者随包）。
6. `cargo tauri` 子命令不存在；本轮用 `npx @tauri-apps/cli@2.12.3` 成功打包。若 CI 需
   可复现，应固定 tauri-cli 版本（与 `tauri` crate 2.12.4 对齐）或安装 cargo-tauri。
7. 既有测试 warning：`tests/sr-itests/tests/probe.rs` 的 `unused import: EncoderProbe`
   （非本轮引入）。

## 6. 变更文件清单

| 文件 | 变更 |
|---|---|
| `crates/sr-core/src/tools.rs`（新增） | 统一工具解析器 + 自检（§1.1） |
| `crates/sr-core/src/lib.rs` | `pub mod tools;` |
| `crates/sr-core/tests/tool_resolution.rs`（新增） | 6 个解析单测 |
| `crates/sr-pipeline/Cargo.toml` | 新增 `sr-core` path 依赖 |
| `crates/sr-pipeline/src/sidecar.rs` | `ffmpeg_exe/ffprobe_exe -> Result`；spawn/探测统一解析 |
| `crates/sr-pipeline/src/{finalize,probe,recovery}.rs` | ffprobe/ffmpeg 调用点统一解析 |
| `crates/sr-merge/Cargo.toml` | 新增 `sr-core` path 依赖 |
| `crates/sr-merge/src/merge.rs` | 删除 `CARGO_MANIFEST_DIR` 定位，改用统一解析 |
| `src-tauri/src/lib.rs` | 注册 `resource_dir`；`self_check()` |
| `src-tauri/src/main.rs` | 隐藏 `--self-check` 入口 |
| `src-tauri/tauri.conf.json` | `bundle.active/targets/icon/resources` |
| `src-tauri/resources/licenses/FFmpeg-LICENSE.txt`（新增） | LGPL 文本原样复制 |
| `src-tauri/resources/licenses/ffmpeg-source-offer.txt`（新增） | 源码获取/合规说明 |
