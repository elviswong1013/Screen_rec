# Screen_rec

Windows 桌面程序（开发中）：截图 / 录屏 / 录音。

## 核心目标能力

1. **按应用音频隔离**：多个应用同时播放声音时，录制指定应用时只写入该应用对应的音轨（不混入其它应用的声音）。
2. **多路同时录制**：支持同时录制 2 个及以上应用/窗口，各自的视频与音频相互独立。
3. **基础能力**：全屏/窗口/区域截图，屏幕录制，麦克风/系统/按应用录音。

## 项目状态

| 阶段 | 产物 | 状态 |
|---|---|---|
| PRD | `docs/prd/001-screen-recorder.md` | **已批准**（2026-10-15） |
| SDD 规格 | `specs/001-screen-recorder.md` | **已批准**（2026-10-15，Gate 2） |
| TDD 实现 | Rust + Tauri | M0–M10 完成；第二轮验收退回（2026-10-17）→ M12 整改中 |

## 文档

- `specs/README.md`：PRD → SDD → TDD 工作流说明
- 可行性研究与选型结论：见 PRD「背景研究」章节
- `docs/research/feasibility-2026-10-15.md`：可行性研究全文（含 §8 Rust 生态选型）
- `docs/reviews/001-sdd-review-2026-10-15.md`：SDD 评审记录（plan-reviewer + 圆桌）
- `docs/milestones/`：里程碑证据（bootstrap/红/绿）；`docs/spikes/`：Spike 报告（S1/S1b/S2/S3/S5）
