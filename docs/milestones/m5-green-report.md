# M5-C 绿实现报告（TDD 绿阶段收尾：多路同时录制 REQ-12 的 M5 范围）

- 日期: 2026-10-17（夜间会话，跨 10-16/17；全部命令为本机实际执行）
- 角色: 后备代码执行通道（executor-backup；主执行通道额度耗尽的替代通道）
- 背景: 此前「M5 绿完成」回报经独立复核判定为不实/未落盘（核实时仓库仍红态：`cargo test --workspace` exit 101、E0432；无本报告）。本报告为**真实落盘**的实现与证据记录，所有结果均来自本会话实际命令输出，日志全文已随报告归档（§6 末尾）。
- 规格: specs/001-screen-recorder.md §2.3（REQ-12）、§3.1/§3.3（start_routes 批量契约/管道协议/ffmpeg 基准）、§3.4/§3.5、§4、§5 M5 行、§8 M0 冻结项
- 红阶段基线: docs/milestones/m5-red-report.md（§3 契约=绑定；§8 两个阻绿缺陷的修法；§6.4 scratch 验证记录）
- 环境: Windows 11 25H2，build **10.0.26200.9711**，SESSIONNAME=Console（交互式用户会话）；cargo/rustc **1.99.0 msvc**；sidecar/ffprobe/ffplay=锁定 `tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin`（§8 冻结）。

## 1. 绿前红态复现（修改前执行，exit 101）

命令：`cargo test --workspace` → **exit 101**，唯一错误 = 新测试引用的两个预期公共符号 unresolved，无其它类别错误：

```text
error[E0432]: unresolved imports `sr_pipeline::route::start_routes`, `sr_pipeline::route::MultiRouteConfig`
  --> tests\sr-itests\tests\multi_route.rs:53:5
   |
53 |     start_routes, Finalized, MultiRouteConfig, RouteError, RouteHandle, RouteTarget, VideoCfg,
   |     ^^^^^^^^^^^^             ^^^^^^^^^^^^^^^^ no `MultiRouteConfig` in `route`
error: could not compile `sr-itests` (test "multi_route") due to 1 previous error
```

日志：`docs/milestones/m5-green-red-repro.log`。

## 2. 变更文件清单（全部为生产代码/本报告；测试零改动）

| 文件 | 变更 |
|---|---|
| `crates/sr-capture/src/canvas.rs` | 缺陷 A：NV12 色度改为交错布局（§3）；删除随之无用的 `uv_w` 局部变量 |
| `crates/sr-pipeline/src/sidecar.rs` | 缺陷 B：`build_encode_args` 显式钉视频码率 `-b:v 12M`（§4）；模块头基准命令注释同步 |
| `crates/sr-pipeline/src/route.rs` | M5 API：新增 `MultiRouteConfig` 与 `start_routes`（§5）；模块头注释补 M4/M5 注记 |
| `crates/sr-pipeline/src/lib.rs` | `route` 模块文档注释补 M4/M5 注记（一行） |
| `docs/milestones/m5-green-report.md` | 本报告（新增） |
| `docs/milestones/m5-green-*.log` | 本会话 8 份原始日志归档（新增，§6 末尾） |

- **未修改任何测试文件**：`tests/sr-itests/tests/*.rs` 最新 mtime = 2026-10-16 13:22:24，早于本会话全部改动（2026-10-17 00:01+）；`multi_route.rs` 逐字未动。
- **未改** `specs/`、`docs/spikes/`；**未 git 提交**（仓库无提交历史，全部 untracked）。
- 既有 M3/M4 API（`start_route`/`start_audio_route`/`RouteConfig`/`AudioRouteConfig` 等）字面量零改动——route L2 回归 5/5 与 audio_isolation 复跑佐证。

## 3. 缺陷 A（阻绿）：canvas.rs NV12 色度交错布局修复

- 根因（m5-red §6.4-1/§8-1）：`bgra_to_nv12` 把 Cb/Cr 按**平面布局**（Cb 全平面 + Cr 全平面 = I420 形状）写入 **NV12（交错）**缓冲，而管道声明 `-pix_fmt nv12` ⇒ 解码侧色度错乱。M3/M4 的 luma-only 断言不可能发现；M5 内容级标记色断言如实失败直至修复。
- diff 摘要（`bgra_to_nv12` 内层循环，一字面修正）：

```diff
-            let u_idx = (y / 2) * uv_w + x / 2;
-            uv_plane[u_idx] = clamp_uv(((-43 * ravg - 85 * gavg + 128 * bavg) >> 8) + 128);
-            uv_plane[uv_w * (h / 2) + u_idx] = clamp_uv(((128 * ravg - 107 * gavg - 21 * bavg) >> 8) + 128);
+            let uv_i = (y / 2) * w + (x / 2) * 2;
+            uv_plane[uv_i] = clamp_uv(((-43 * ravg - 85 * gavg + 128 * bavg) >> 8) + 128);
+            uv_plane[uv_i + 1] = clamp_uv(((128 * ravg - 107 * gavg - 21 * bavg) >> 8) + 128);
```

  同时删除已无引用的 `let uv_w = w / 2;`（保持 0 warning）。
- 验证（本会话实测）：multi_route 两轮全绿，标记色占比本路最小 **0.0972–0.1033**（阈值 ≥0.02，约 5× 裕度）、其它路最大 **0.0000**（≤0.005）——色度正确落位；route 回归的非黑/运动帧断言不受影响（luma 路径未动）。

## 4. 缺陷 B（阻绿稳定性）：编码显式钉视频码率

- 根因（m5-red §6.4-2/§8-2）：NVENC 默认 ~2Mbps 在 1080p30 高运动内容下间歇性压垮色度（~1/8 轮某路 own 标记占比 0.097→0.0000，亮度/运动/时长全部正常）。scratch 以 `-b:v 12M` 验证 8/8 轮消除。
- diff 摘要（`build_encode_args`）：

```diff
     args.extend(["-c:v".into(), video_encoder.into()]);
+    args.extend(["-b:v".into(), "12M".into()]);
     args.extend(["-g".into(), (2 * video.fps).to_string()]);
```

  模块头基准命令注释同步补 `-b:v 12M`（与实现一致）。规格 §3.3 基准命令为「骨架，S2/S5/S6 校准」，该显式码率属 S2 矩阵校内校准；本机 NVENC 链取 12M。
- 验证（本会话实测）：multi_route **2/2 轮全绿**（39.67s / 38.45s），两轮 4 用例中 own 标记均稳定在 0.097–0.103、other 0.0000，未再出现压垮现象。样本量 2 轮，如实记录；编排者独立复跑可继续加样本。

## 5. M5 API 实现（`sr_pipeline::route` 多路扩展面）

按 m5-red §3 契约（绑定）新增，M3/M4 既有 API 逐字不动：

```rust
pub struct MultiRouteConfig {
    pub target: RouteTarget,
    pub video: VideoCfg,
    pub audio: Vec<AudioSourceSpec>,   // 空 = 纯视频路；非空 = 每 App 源一条独立音轨（M4 隔离语义）
    pub output_dir: PathBuf,
}

pub fn start_routes(cfgs: Vec<MultiRouteConfig>) -> Result<Vec<RouteHandle>, RouteError>;
```

实现要点（逐路委托既有 `start_route_impl`，即「逐路完成 §3.1 启动协议 → Ok = 输出开始」）：

- **Ok(handles)**：与 cfgs 同序同长（逐路启动、依次 push）；顺序启动合法（红报告 §6.4 scratch 实测 3 路 1080p 顺序启动 <1s）。
- **Err（严格回滚）**：任一路启动失败 → 对该失败点之前已启动的每一路 `request_stop()` → `wait_finalized(90s)` → 删除该路**成品/段/收尾清单**，全部完成清理后才返回失败路的 `RouteError`（无效目标 → `TargetInvalid`）。未被批次接受的产物不提升到输出目录（REQ-24 提交语义；m5-red §3.3/§8-4）。
- 句柄/收尾/错误语义复用 M3/M4（`request_stop`/`wait_finalized`/`segments`；`SidecarExit` 强匹配）。
- 空 `cfgs` 平凡返回 `Ok(vec![])`（红 §8-8 边界未钉，实现按平凡语义，未单测）。

回滚用例实测（`it_routes_batch_start_all_or_rollback`）：`[有效目标, 0xDEADBEEF]` → `Err(TargetInvalid)`；Err 返回后测试的 10s 轮询窗口内**无 ffmpeg 子进程残留**、有效路输出目录**无成品残留**——两轮均通过。

## 6. 复跑证据（本机实际执行；所有 L2 均为跨进程/进程内串行）

| # | 命令 | 结果（exit / 通过数 / 耗时） | 日志（docs/milestones/） |
|---|---|---|---|
| 1 | `cargo test --workspace`（修前红态） | **exit 101**；仅 E0432（`start_routes`/`MultiRouteConfig`），无其它错误 | `m5-green-red-repro.log` |
| 2 | `cargo check -p sr-capture -p sr-pipeline --all-targets` | **exit 0**（0 warning；首轮 1.27s，复跑即刻） | `m5-green-check.log` |
| 3 | `cargo test --workspace`（L1） | **exit 0**；L1 全绿：screen_rec_lib 7 / drift_hysteresis 1 / silence_pad_late_packet 1 / pacer_cfr 1 / channel_snapshot_rebind 1 / ffmpeg_args_snapshot 1 / output_naming 1；L2 按门 ignored：audio_isolation 4 / enum_audio 1 / enum_targets 4 / **multi_route 4** / route 5 / shot 4；0 failed、0 warning | `m5-green-l1.log` |
| 4 | `cargo test -p sr-itests --test multi_route -- --ignored --nocapture --test-threads=1`（第 1 轮） | **exit 0；4 passed；39.67s** | `m5-green-multi-route-run1.log` |
| 5 | 同上（第 2 轮） | **exit 0；4 passed；38.45s** | `m5-green-multi-route-run2.log` |
| 6 | `cargo test -p sr-itests --test route -- --ignored --nocapture --test-threads=1` | **exit 0；5 passed；60.83s** | `m5-green-route-regress.log` |
| 7 | `cargo test -p sr-itests --test audio_isolation -- --ignored --nocapture --test-threads=1`（第 1 轮） | exit 101；3 passed / **1 failed**——`it_backpressure_step_correction` 时长 Δ=0.765s vs 容差 0.75s（已知测试侧边际，见 §7-1） | `m5-green-audio-isolation-run1.log` |
| 8 | 同上（第 2 轮） | **exit 0；4 passed；48.44s**（bp Δ=0.680s） | `m5-green-audio-isolation-run2.log` |

multi_route 两轮关键实测值（内容级判定）：两路 A/B own 标记 0.0972/0.1033、other 0.0000、运动对 28/28 与 16/16、音轨 RMS 0.0217（阈值 0.005）、时长 A 9.769s vs B 5.833s（span 9.041s/5.500s）；三路 own 0.0972/0.1033/0.0972、other 0.0000、时长 10.633/6.500/10.466s（span 10.05/6.00/10.05s）、中间路停后两侧 > 中间 +1.5s；故障注入恰 1 路 `Err(SidecarExit)`（保留 2 段、首段 6.100s 可播）、幸存路 8.5s 有效。route 回归 5/5：强杀 SidecarExit、命名/关键帧 `[0.0,2.0,4.0,6.1]`、分段收尾 Σ段=成品 12.733s、光标开关、非黑/运动。运行结束后 `Get-Process ffmpeg,ffplay` 抽查 **0 残留**。

日志说明：本表第 4–8 行为 `%TEMP%\opencode\m5_green_*.log` 的归档副本（第 1/3 行同）；控制台中文在日志中偶现编码折损（cp936↔UTF-8），ASCII 关键值（exit/计数/阈值/dB/Δ）逐字可核。

## 7. 未决问题（如实记录）

1. **audio_isolation bp 时长断言边际（测试侧，既有）**：本会话第 1 轮 Δ=0.765s > 0.75s 失败（第 2 轮 0.680s 通过）。该现象在 M4 绿报告 §6-2 与 AV 任务板 §9.5 已有 19 次实测族（0.624–1.193，过 13/不过 6），与本 M5 改动无因果链（bp 用例不使用 `start_routes`；音轨时长口径为已知启动瞬态+AAC 帧取整）。**未改任何测试断言**；编排者复跑可能再遇此 flake，属既有测试侧口径问题。
2. **12M 码率的 S2 矩阵语境**：本值按 m5-red §8-2 指引取 scratch 验证值（本机 NVENC 链）；换编码器链（QSV/AMF/h264_mf）或极端主题下的正式标定仍属 S2 矩阵轮次。
3. **回滚产物口径**：本实现取「严格回滚」（删除成品/段/清单）。m5-red §8-4 留了「保留产物供恢复」备选；若后续里程碑改变语义，需与测试作者协商（当前测试断言与严格回滚一致，两轮实测通过）。
4. **三套 config 收敛未做**：`RouteConfig`/`AudioRouteConfig`/`MultiRouteConfig` 并列为增量形态（m5-red §8-6；M3/M4/M5 测试全字段字面量约束）。
5. **sidecar→路由映射未暴露**（m5-red §8-7）：故障用例维持对称断言（恰 1 Err / 恰 1 Ok）。
6. **边界未钉**：空 `cfgs`（实现返回 `Ok(vec![])`）、单元素批次、Monitor/Region 多路目标——红测试未构造，沿红报告 §8-8。
7. **目标窗口关闭+音频路同在**的语义（m5-red §8-5）、**WGC 对被遮挡 GDI 窗口的表面节流观察**（§8-3）仍属后续（M6/REQ-13 范围）。
8. **FFT 泄漏阈值余量偏薄**（audio_isolation）：本会话 2 轮实测 −42.4~−45.9dB（阈值 −40），未动阈值；偶发回归风险如实记录（M4 既有观察）。

## 8. 验收结论（对照任务验收项）

| 验收项 | 状态 | 依据 |
|---|---|---|
| L1 `cargo test --workspace` exit 0 | ✅ | 证据 #3（0 failed；0 warning） |
| `multi_route` 4/4 | ✅ | 证据 #4/#5（两轮 4 passed，exit 0） |
| `route` 回归 5/5 | ✅ | 证据 #6 |
| `audio_isolation` 回归 4/4 | ✅ | 证据 #8（第 1 轮为既有测试侧边际 flake，见 §7-1） |
| `m5-green-report.md` 落盘且与事实一致 | ✅ | 本文件 + 8 份原始日志归档 |
| 测试文件零改动 / 未提交 / specs 未动 | ✅ | §2（mtime 核实） |

结论：M5 绿阶段达成——两个阻绿缺陷已修（色度交错、码率钉死），`MultiRouteConfig`/`start_routes` 已按红契约真实落盘，四项门全部通过；建议编排者按 §6 命令独立复跑核验（预算：L1 ≈1–2min、multi_route ≈40s/轮、route ≈1min、audio_isolation ≈50s；audio_isolation 若遇 §7-1 已知 flake 复跑一轮即可）。
