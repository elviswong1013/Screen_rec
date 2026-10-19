//! sr-capture：WGC 会话、源时间戳帧槽节奏器、缩放+NV12、固定画布、管道写入。
//!
//! 职责（specs/001-screen-recorder.md §3.2）：REQ-9..13 的 M3 范围。
//!
//! - [`pacer`]：帧槽节奏器纯逻辑（槽位=源时间戳纯函数，REQ-10）；
//! - [`wgc`]：单路 WGC 捕获（windows-capture 2.0.1，独立线程 + frame pool，光标开关）；
//! - [`canvas`]：固定输出画布（等比缩放 + 黑边 + BGRA→NV12，S1 定稿 CPU 路径）；
//! - [`pipe`]：命名管道视频写（§3.3：Rust=服务端、8MiB、外置超时连接、阻塞写=背压）。

pub mod canvas;
pub mod pacer;
pub mod pipe;
pub mod region;
pub mod wgc;
