# 可行性研究：Windows 截图/录屏/录音（按应用音轨隔离 + 多路同时录制）

日期：2026-10-15
状态：研究完成（结论供 PRD 引用）
方法：5 路并行文献/源码调研（Cap、OBS、其余 4 仓库、Windows 捕获栈、Windows 音频与封装）。标注：已核实=一手文档/源码；推断=基于证据的工程判断。

## 0. 总体结论

1. **没有任何现成免费程序完整满足需求**：
   - OBS 最接近：单应用「窗口视频+该应用音轨」原生可做；但单实例只有一个录制输出，多路并发需开多个实例（`--multi`），且无法拆分同一进程/浏览器标签内的音频。
   - Cap 不支持按应用音频（仅有未合并 PR #2368），也不支持多路并发。
   - 其余 4 仓库（Flutter 插件/Android 示例/Chrome 扩展/Web SaaS）平台不符，仅个别思路可参考。
   - Xbox Game Bar、Win11 截图工具、ShareX 均无「按应用音频 + 多路并发」能力。
2. **自研技术可行**：Windows.Graphics.Capture（WGC）+ WASAPI 进程环回（process loopback）+ 多音轨封装（MKV/FFmpeg 或 Media Foundation）均为成熟系统能力，且许可干净。
3. **硬性限制（需如实预期）**：
   - 按应用音频捕获最小粒度=进程树；同一进程内多路音频无法拆分，浏览器无法按标签隔离（Chrome 音频在独立子进程，只能整浏览器捕获）。
   - 个别应用实测无法捕获（如 MS Teams、Valorant/COD 语音）。
   - process loopback 需 Win10 2004+(19041) 实测可用；官方标注最低 20348（≈Win11/Server 2022）。需启动探测与降级提示。
   - 无音源播放时捕获流给静音/无包，需补静音维持时间轴。

## 1. OBS Studio（GPL-2.0-or-later）

- 多音轨上限 `MAX_AUDIO_MIXES = 6`（libobs/media-io/audio-io.h:26）；`obs_output_set_mixers` 位掩码选择输出音轨（libobs/obs-output.c:947）；每源勾选 Track1–6（高级音频属性）。封装经独立子进程 ffmpeg-mux（plugins/obs-ffmpeg/ffmpeg-mux/ffmpeg-mux.c）。MKV/MP4/MOV 支持多音轨；FLV 单轨；官方建议多音轨用 MKV 后 remux。
- 应用音频捕获（Beta）：WASAPI process loopback，`PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE`（plugins/win-wasapi/win-wasapi.cpp:690-720）；先找窗口→PID；粒度=进程树；OBS 要求 Win10 2004+。
- 窗口捕获：BitBlt 与 WGC 双路径（plugins/win-capture/、libobs-winrt/winrt-capture.cpp）；多窗口=多源；单实例仅一个录制输出，无原生多视频轨。
- 需求(1)操作路径：Window Capture 选目标 + Application Audio Capture 同应用 + 关闭 Desktop Audio + 只勾 Track1 + 输出 MKV → 单视频+单音轨。
- 需求(2)：仅多实例 workaround（`--multi` + 不同 profile）。
- 复用边界：GPL 传染，闭源不可抄代码；概念与微软 API 可自由实现（官方 ApplicationLoopback 示例许可宽松）。

## 2. Cap（整体 AGPL-3.0；scap-* / cap-camera* 为 MIT）

- Windows 捕获：WGC + D3D11（crates/scap-direct3d，基于 windows-capture crate 1.5）。
- 音频：WASAPI loopback（系统音）+ cpal 麦克风；**无 per-app**；多路混成单轨（sources/audio_mixer.rs）；单目标、单实例（single_instance.rs）。
- 编码：FFmpeg fork + enc-mediafoundation（Windows 硬件编码）；mux 为 fMP4 分段，非多轨。
- 可复用（MIT）：`scap-*` 与 `cap-camera*` 两个家族（scap-direct3d / scap-targets / scap-cpal 等）；`cap-enc-mediafoundation`、`cap-muxer` 等其余 crate 为 AGPL-3.0，仅可参考不可复用（2026-10-15 更正）。多数未发 crates.io，需 vendor 源码。
- PR #2368（open）拟加 per-app audio；未合并，不可依赖。

## 3. 其余 4 仓库（结论：基本不相关）

| 仓库 | 平台 | 音频能力 | 许可 | 结论 |
|---|---|---|---|---|
| Isvisoft/flutter_screen_recording | Android/iOS/Web（无 Windows） | 麦克风/系统音单轨 | MIT | 不相关 |
| saki4510t/ScreenRecordingSample | Android/Java | 多轨封装思路（MediaMuxer） | Apache-2.0 | 不相关（思路参考） |
| muaz-khan/Chrome-Extensions | Chrome 扩展 | tabCapture 标签级隔离 | MIT | 思路参考（浏览器标签） |
| MarconLP/snapify | Web SaaS（Next.js） | 仅麦克风单轨 | AGPL-3.0 | 不相关 |

## 4. Windows 捕获与实现栈（视频侧）

- WGC（Win10 1803+）：窗口+显示器捕获、多会话（同一 item 可多 session）、`IsCursorCaptureEnabled`(2004+)、`MinUpdateInterval`/`IsBorderRequired`(20348+/Win11)、HDR FP16 帧池；每会话独立 frame pool，建议保留 2 buffer；无法抓真正独占全屏（OpenGL/Vulkan）。
- DXGI Desktop Duplication：仅显示器级、需同 GPU；HDR 支持好；适合全屏低开销场景。
- 截图：WGC 复用录像管线为首选；PrintWindow(PW_RENDERFULLCONTENT) 可抓被遮挡窗口/Chromium 内容，作备选。
- 编码/封装：FFmpeg（LGPL 版可闭源分发，NVENC 兼容；GPL 版含 libx264 会传染）；Media Foundation（OS 原生、无 GPL）可用 SinkWriter 写 MP4 多流；硬件编码 NVENC/QSV/AMF 有并发会话限制（旧 GeForce 2 路）。
- 现成免费工具缺口：Game Bar（单应用、无按应用音频）、Win11 截图工具（无多路）、ShareX（单会话）。
- 实现栈候选：C# .NET 8（WPF/WinUI3）+ CsWinRT/NAudio/SinkWriter（适配度最高）；Rust + windows-rs + wgpu（资源占用最优，Cap 同路线）；C++/Qt（最强控制、最费力）；Electron（不推荐）。

## 5. Windows 音频隔离与封装（音频侧）

- process loopback API 流程：PID → `AUDIOCLIENT_ACTIVATION_PARAMS`(VT_BLOB) → `ActivateAudioInterfaceAsync(VAD\Process_Loopback)` → `IAudioClient.Initialize(SHARED|LOOPBACK|EVENTCALLBACK)` → `IAudioCaptureClient`；无物理端点、不能用 GetMixFormat（自建 float 格式）。
- 最低版本：官方两处文档 20348 与 20438 说法不一；实测/OBS 要求 2004+(19041)。建议启动探测。
- INCLUDE/EXCLUDE 进程树语义；目标无渲染流→静音；无数据需补静音防时间戳跳变；QPC 时间戳统一时基；多路独立时钟域可能漂移，需重采样校正。
- 容器：MKV 首选（多轨无压力）；MP4/MOV 支持多轨但有编解码兼容限制；FFmpeg `-map` 多输入多轨封装；MKV→MP4 remux 有坑。
- 输出策略取舍：每路独立文件（稳、容错）vs 单文件多轨（交付体面）→ 推荐「录制各写独立文件 + 收尾可选合并为 MKV 多音轨」。
- 虚拟声卡备选：VB-CABLE（donationware，有条件可随分发）、VoiceMeeter、VAC（商业）；需手动路由、非自动按进程隔离，仅作兜底。
- 注意：FFmpeg mainline 无 WASAPI 输入设备，必须自行采集 PCM 再喂 muxer。

## 6. 推荐架构（供 PRD/SDD 参考）

- 捕获层：每目标一路 WGC 会话（窗口/显示器），独立 frame pool。
- 音频层：每目标一路 process loopback（INCLUDE 树）+ 可选麦克风/系统音轨；48kHz float；QPC 时基；补静音。
- 封装层：每路独立文件（MKV 或分段 MP4）；可选收尾 FFmpeg(LGPL) 合成多轨 MKV；硬件编码优先 MF/NVENC。
- UI：待定（PRD 决策）。
- 风险：多路资源与编码器会话上限；音画同步/漂移；容器兼容性；Win10 门槛；个别应用不可捕获。

## 7. 关键证据链接（节选）

- OBS：github.com/obsproject/obs-studio（libobs/media-io/audio-io.h、plugins/win-wasapi/win-wasapi.cpp、plugins/obs-ffmpeg/ffmpeg-mux/）；obsproject.com/kb/application-audio-capture-guide
- Cap：github.com/CapSoftware/Cap（crates/scap-direct3d、crates/scap-cpal、apps/desktop-gpui）；PR #2368
- 微软：learn.microsoft.com（screen-capture (WGC)、audioclientactivationparams、ActivateAudioInterfaceAsync、loopback recording）；github.com/microsoft/Windows-classic-samples（ApplicationLoopback）
- FFmpeg：ffmpeg.org/ffmpeg.html（-map）、ffmpeg.org/legal.html
- 许可：vb-audio.com/Services/licensing.htm
- 其他：github.com/sskodje/ScreenRecorderLib（MIT，无 per-app audio）；github.com/MathewSachin/Captura（MIT，停更）；crates.io/crates/windows-capture（MIT）

## 8. Rust 生态选型补充（2026-10-15，SDD 输入）

- 捕获：`windows-capture` 2.0.1（MIT）——WGC 窗口/显示器捕获、光标/边框/最小间隔设置、PNG 截图、`start_free_threaded` 支持多会话；限制：Win10 无法去捕获边框（去边框需 Win11）、独占全屏不可抓；其内建编码器仅输出单轨 MP4，本项目只取其帧，不用其编码器。备选：`windows` 0.62 直调 WGC；`wgc` 2.0.1。
- 音频：`wasapi` 0.26（MIT，HEnquist）——同时覆盖设备环回 + 按进程环回（`new_application_loopback_client(pid, include_tree)`）+ 会话枚举；进程环回下 GetMixFormat 不可用，须自建 48k f32 格式；`cpal` 无进程环回。参考实现（只读思路）：wasapi-rs `examples/record_application.rs`、screenpipe、fluxer；Cap PR #2368（未合并，AGPL 仓库）不可抄码。
- 重采样：`rubato` 5.x（MIT/Apache-2.0）。
- 编码/封装：纯 Rust MKV 写入不可用（`matroska` crate 全量缓冲到 finish 才落盘；`muxide`/`mp4e` 不支持 MKV）→ 采用 **LGPL ffmpeg sidecar**（BtbN `win64-lgpl`/`lgpl-shared`）：编码 `h264_mf`/硬件编码器（LGPL 构建可用；libx264/libx265 仅 GPL 构建，禁入）+ AAC；`-f segment` 5s 分段 + 停止时 concat；合并用 `-map … -c copy`。备选：进程内 MF 编码（windows-rs `Win32_Media_MediaFoundation`；Cap 的 enc-mediafoundation 为 AGPL，仅读架构）。
- Tauri：`tauri` 2.12.4 + 插件（single-instance 2.5.4、dialog 2.8.3、store 2.5.0、global-shortcut 2.4.1 可选）；`tauri::ipc::Channel` 用于流式状态（事件不适合高吞吐）；托盘为核心 feature；sidecar 用 `bundle.externalBin`（文件名带 target-triple 后缀）；WebView2 建议默认下载器或 `embedBootstrapper`（离线环境）。
- CI/测试：托管 GitHub Actions windows runner 无法做真实捕获（WGC 黑帧，actions/runner-images#7227）→ 分层：L1 单元（CI）/ L2 本机或 self-hosted 集成（FFT/ffprobe 验证）/ L3 人工半自动。FFT 用 `rustfft`/`realfft`，画面质量 `av-metrics`。
- 参考项目（除 Cap）：screenpipe（许可待核）、nashaofu/xcap（Apache-2.0）、CapSoftware/scap（MIT）、NiiightmareXD/windows-capture（MIT）。
