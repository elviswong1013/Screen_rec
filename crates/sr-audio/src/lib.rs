//! sr-audio：wasapi 客户端（线程内生命周期）、样本槽/静音补齐/迟到裁切、
//! 漂移滞回/限幅、WAV 写入。
//!
//! 职责（specs/001-screen-recorder.md §3.2）：REQ-14..19、REQ-26 的音频侧。
//! M4 实现：[`wasapi`]（进程环回采集核：身份前置校验/INCLUDE 树/MTA 线程/
//! 48k f32 立体声 autoconvert/时长钉齐）、[`slots`]（样本槽编排纯逻辑）、
//! [`resample`]（漂移滞回带/速率限幅决策核）。M6 扩展：[`wasapi`] 设备环回
//! （REQ-15）/麦克风（REQ-16）采集目标（同一采集核）、[`wav`]（REQ-19：流式写 +
//! 崩溃头部修复）。

/// wasapi 客户端（REQ-14..16/D2）：进程环回（INCLUDE_TARGET_PROCESS_TREE 由
/// wasapi 激活参数原生承载）、默认端点环回/麦克风（M6）；专用 MTA 线程内
/// COM 初始化/客户端创建/使用/释放（§3.1 线程规则，wasapi 类型 !Send）。
pub mod wasapi;

/// 样本槽与补齐/裁切（REQ-18）：48kHz/f32/立体声（autoconvert）；WASAPI 100ns
/// 单位统一 epoch（REQ-25）；静音补零；迟到包写前裁切（禁止同一时间段写两次）；
/// AUDCLNT_BUFFERFLAGS 语义区分静音/断流/设备错误。
pub mod slots;

/// 变比重采样与漂移记账（REQ-26）：滞回带/速率限幅参数化决策核（S6 定值前
/// 增益与滤波自由）；rubato 5.x Async+Sinc 变比重采样（延迟参数化补偿）；
/// 与阶跃校正分开记账。
pub mod resample;

/// WAV 写与恢复（REQ-19：各来源独立 WAV，48kHz s16）；停止即成品；
/// 崩溃恢复=头部重写+尾部不完整样本裁切（RIFF 4GiB 上限明确失败）。
pub mod wav;
