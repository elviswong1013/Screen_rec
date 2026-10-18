//! sr-shot：截图（显示器/窗口/区域 + 跨屏拼合 + PNG）。
//!
//! 职责（specs/001-screen-recorder.md §3.2）：REQ-5..8。
//!
//! 技术路径（§1 决策 + docs/research/feasibility-2026-10-15.md §4）：
//! - 捕获 = WGC 单帧（Windows.Graphics.Capture，free-threaded frame pool + 轮询取帧），
//!   在独立 worker 线程执行（MTA + per-monitor-v2 线程 DPI 上下文）——物理像素语义
//!   不受调用方进程 DPI 感知影响（REQ-6，避免虚拟化坐标）；
//! - 窗口备选 = PrintWindow(PW_RENDERFULLCONTENT)（WGC 不可用时读取窗口自身表面，
//!   研究报告 §4；遮挡语义已由 M2 红报告 §6.2 scratch 独立验证）；
//! - 区域 = 按各显示器物理帧在桌面物理坐标系裁剪拼合（REQ-6，含负坐标与跨多显示器）；
//! - 编码 = png crate（RGBA8，IHDR 即输出元数据）；输出命名最小实现（目录内、
//!   扩展名 png、进程内唯一、create_new 禁覆盖；REQ-24 全量规则属 M8）。
//!
//! 公共 API 契约（测试即契约）：`screenshot` 模块，见 docs/milestones/m2-red-report.md
//! §3/§10。`capture`/`compose`/`png` 为内部模块（M1-A 骨架中的同名占位模块转内部实现）。

mod capture;
mod compose;
mod png;

pub mod screenshot;
