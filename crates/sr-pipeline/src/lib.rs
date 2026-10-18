//! sr-pipeline：sidecar 管理（启动/停止协议、分段、后台 concat、命名、恢复、探测）。
//!
//! 职责（specs/001-screen-recorder.md §3.2）：REQ-20..22、REQ-24、REQ-28（probe）。
//! M3 实现：[`sidecar`]（编码命令构建/sidecar spawn/编码器链探测）、[`naming`]
//! （REQ-24 命名）、[`route`]（单路视频 start_route/收尾）。
//! M8 实现：[`recovery`]（Manifest/崩溃恢复状态机、REQ-22 完整）、[`probe`]
//! （分层能力探测与降级注入面、REQ-28）、[`naming`] 会话临时目录与安全提交（D12）、
//! [`sidecar`] 锁定构建审计（REQ-29）与 Kill-on-close Job（D8）。
//! queue/segments/finalize 的完整形态属后续轮次。

/// ffmpeg sidecar 生命周期（REQ-20/D3/D4：锁定 LGPL n8.1 构建；编码命令骨架；
/// D4 编码器链实际初始化探测；M8：进程级 Job 绑定 + 锁定构建许可/校验和审计 REQ-29）。
pub mod sidecar;

/// 命名/目录/安全提交（REQ-24/D12：sanitize、时间戳、纯分配器禁覆盖；
/// M8：`.screenrec_tmp/<session_id>` 会话目录 + 安全提交临时名→验证→rename）。
pub mod naming;

/// 单路视频路由（M3：start_route/RouteHandle/Finalized；§3.1 启动协议 →
/// 「输出开始」；EOF 优雅停止 + 后台 concat 收尾；sidecar 死亡 → E_SIDECAR_EXIT）。
/// M4 音轨扩展（start_audio_route）；M5 多路批量（start_routes：全成功或回滚）；
/// M6 全音轨（System/Mic 源）与纯录音（start_audio_only：WAV 停止即成品，REQ-19）。
/// M8：会话目录/manifest 自动维护 + 磁盘写失败 E_DISK_FULL（REQ-22）。
pub mod route;

/// 有界队列（REQ-26：字节+时间双重限界）——M4+ 轮次实现。
pub mod queue {
    // TODO(M4): it_backpressure_step_correction。
}

/// 分段运行时校验（REQ-21 的段完整性/烂段剔除增强）——M4+/S5 语义深化。
pub mod segments {
    // TODO(M4): it_segment_keyframe_closed_gop（M3 由 route 内联实现 5s 分段）。
}

/// 后台收尾通用化（REQ-21）：ffconcat 收尾清单 + 段边界时间轴校准
/// （M10b P0：`duration` 取音轨内容跨度，消除 concat 段首前导空洞）。
pub mod finalize;

/// 恢复状态机（REQ-22：Manifest/目录重扫/崩溃收尾/丢弃；M8 落地）。
pub mod recovery;

/// 能力探测与降级（REQ-28：分层探测 + 可注入故障钩子；M8 落地）。
pub mod probe;

/// 时间轴单位换算（REQ-25：QPC ticks ↔ 100ns ↔ UNIX epoch + 公共 T0 时间轴）。
pub mod epoch;
