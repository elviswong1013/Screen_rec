# S5 Spike 报告：分段 MKV 崩溃恢复验证（Screen_rec）

- 日期：2026-10-16（实验执行 00:03–00:20 +08:00）
- 依据：specs/001-screen-recorder.md §8 S5、REQ-20/21/22、§9 T4
- 性质：M0 Spike 验证实验，无生产代码改动
- 环境：Windows（本机 x64），ffmpeg N-127564-g3c92f003c7-20261015（LGPL 构建，`--disable-libx264`，bin 下三件套见 `tools/ffmpeg/ffmpeg-master-latest-win64-lgpl/bin/`）
- 编码器：`h264_qsv`（主测，本机 Intel QSV 初始化成功）、`h264_mf`（对照）；音频 `aac`
- 输入：合成信号 `testsrc2 1920x1080@30` + 双音轨 `sine 440Hz/48k`、`sine 880Hz/48k`（模拟"1 路视频 + 2 音轨"的录制布局）
- 产物目录约定：分段与成品在 `%TEMP%\opencode\s5\<Tag>\`；脚本在 `tools/spikes/s5/*.ps1`；原始日志在 `tools/spikes/s5/logs/`
- 标记约定：【实测】= 本次实际执行并观察到；【未验证】= 未覆盖或需后续 spike/L2 测试确认

## 1. 命令基准（原样记录）

录制命令（任务给定基线，`tools/spikes/s5/s5-run.ps1` 生成，原文存 `logs/<Tag>_cmd.txt`）：

```
ffmpeg -hide_banner -nostdin
  -f lavfi -i testsrc2=size=1920x1080:rate=30
  -f lavfi -i sine=frequency=440:sample_rate=48000
  -f lavfi -i sine=frequency=880:sample_rate=48000
  -map 0:v -map 1:a -map 2:a
  -c:v h264_qsv -g 60 -c:a aac
  -f segment -segment_time 5 -reset_timestamps 1 -segment_format matroska
  <outdir>\out_%03d.mkv
```

变体：`-re`（实时供数，内容时间≈墙钟，模拟生产管道节奏）；优雅停止对照去掉 `-nostdin`；`-force_key_frames expr:gte(t,n_forced*5)` 对齐变体；`h264_mf` 替换 `-c:v`。

## 2. 实验 0（意外发现）：stderr 未排空 → ffmpeg 停摆

【实测】按任务基线命令**不加 `-re`** 首跑（Tag `crash_qsv_seg5`，logs/crash_qsv_seg5_stderr.log）：测试驱动未异步排空 stderr，ffmpeg 在自身 elapsed≈5.16s 后停止写进度、输出文件停止增长，直至 13s 强杀。

- 机理：stderr 为管道、读端不消费 → ~4KB 内核缓冲写满 → ffmpeg 阻塞在 progress 打印 → 整条管线停摆。
- 结果：5.16s 内以 10.6x 速度产出 54.65s 内容、11 个分段后卡死；kill 后 89ms 内进程回收。
- **结论（对 REQ-20 的直接实证）：sidecar 管理必须持续排空 stderr/stdout**（REQ-20 已有此要求，本实验为反面样例）；后续所有实验改为异步排空后无此现象。

## 3. 实验 1（主实验）：5s 分段 + 13s 后 taskkill /F 强杀

Tag `crash_qsv_seg5_rt`（`-re` 实时 + stderr 排空）。

运行与强杀：【实测】spawn 后 `Start-Sleep 13`，`taskkill /F /PID` 返回 SUCCESS，`WaitForExit` 50–59ms 内回收，exit code 1（终止符）。

- 时间线要点【实测】：墙钟（Get-Date）显示强杀间隔 13.006s，但 ffmpeg 单调进度钟显示 elapsed=14.45s、`time=00:00:14.52`，且 NTFS 文件时间与之一致 —— **实验期间系统墙钟被回拨约 1.4s（疑似午夜后 NTP 校时）**。δ 记账必须用 QPC/单调时钟域（与 REQ-25 单一 epoch 精神一致），墙钟仅作展示。
- 段文件列表（`%TEMP%\opencode\s5\crash_qsv_seg5_rt\`）：

| 文件 | 大小(B) | 容器时长 | 流数 | 视频帧数 | 音频帧(a0;a1) | 首视频包 | 解码检查 |
|---|---|---|---|---|---|---|---|
| out_000.mkv | 3,863,273 | 6.067s | 3 | 180 | 279;279 | pts=0.067, **K** | exit 0，0 错误 |
| out_001.mkv | 2,571,352 | 4.027s | 3 | 120 | 186;186 | pts=0.027, **K** | exit 0，0 错误 |
| out_002.mkv（被杀尾段） | 2,359,296 | **N/A** | 3 | 110（可解码） | 171;171 | pts=0.038, **K** | exit 0，仅 "File ended prematurely" 提示 |

关键观察【实测】：
1. **所有已完成分段独立可播**：ffprobe 正常、完整解码 0 错误、每段以关键帧（IDR）开头、每段 3 条流（1 视频+2 音轨）齐全。
2. **无 `-force_key_frames` 时段长不规则**：名义 5s，实测 6.067s / 4.027s（`-g 60`@30fps → 关键帧每 2s，切段对齐到 ≥5s 的下一关键帧，且 muxer 判定导致后续段提前到 4.03s）。段长 = f(segment_time, GOP)，**不可假设等于 segment_time**。
3. **尾段（未封口段）仍大部分可播**：无 duration/Cues，但已刷盘的 cluster 可完整解码 110 帧（本地 0.038→3.804s ≈ 3.77s 内容）；探测/解码仅报 "File ended prematurely"。
4. 最后进度 `time=14.52`（内容时间，单调钟）。

### δ 量化（本次运行）

- **结构性上界（丢弃尾段的保守口径）**：δ_struct = 杀时内容时间 − 最后封口段结束 = 14.52 − 10.094 = **4.43s** ≤ 实际段长上界（6.07s）≈ segment_time + 关键帧对齐漂移（漂移实测 ≤ +1.07s）。
- **播放口径（含尾段已刷盘内容）**：可解码内容到全局 ≈13.90s（10.094 + 3.804）→ δ_play = 14.52 − 13.90 ≈ **0.62s**（编码器管线滞后 + MKV cluster 未刷盘部分；matroska muxer 按 ~2MB/5s 关闭 cluster，见 §7 MF 对照）。
- 对 §9 T4（损失 ≤5s+δ，δ 目标 ≤1s）：**若恢复时回收尾段（remux 封口），δ_play≈0.6s（QSV）满足 δ≤1s；若恢复只认封口段，损失上界是 segment_time+漂移（实测最坏 6.07s 段长本身），δ_struct 的定义需要按"最后封口段结束点"记账而非名义 5s**。

## 4. 实验 2：优雅停止对照（stdin 写 `q`）

Tag `grace_qsv_seg5`（去掉 `-nostdin`，其余同主实验；`logs/grace_qsv_seg5_stderr.log`）。

【实测】13s 后向 stdin 写 `q`：**561ms 内干净退出（exit 0）**，编码器排空到内容 13.41s，3 段全部封口：6.067s(180帧) / 4.027s(120帧) / **3.538s(105帧，正常 duration、0 解码错误)**。前两段与强杀运行**逐字节同尺寸**（3,863,273 / 2,571,352）——合成输入+确定性编码下切割点可复现，交叉印证段边界行为一致。

与强杀的差异【实测】：
- 优雅：尾段有 duration/Cues、AAC 编码器统计收尾（Qavg 等）、`muxing overhead` 汇总行、exit 0；成品即"完整段"。
- 强杀：尾段无 duration、截断告警、被杀时**在途内容丢失**（强杀 14.52s vs 优雅排空到 13.41s 属不同杀点；同点对比见两运行前两段逐字节一致）。
- 退出延迟：优雅 `q`→exit 561ms（含排空）；强杀 kill→reap 50–59ms。生产停止协议（REQ-20 EOF→排空→超时强杀）两条路径均有界。

## 5. 实验 3：concat 收尾与校验

脚本 `tools/spikes/s5/s5-concat.ps1`；列表文件 `ffconcat version 1.0` + `file 'out_XXX.mkv'`（相对文件名，规避 Windows 反斜杠转义）。

### 5.1 优雅运行三段收尾（对照基线）

【实测】`ffmpeg -f concat -safe 0 -i list.txt -map 0 -c copy final.mkv`：
- **耗时 92ms**（13.6s 内容；no-map 对照 98ms）。
- 成品：duration **13.653s** ≈ 段和 13.632s（+21ms 边界）；3 流保留（h264 + aac×2）✓。
- **不带 `-map 0` 的任务原样命令 `… -c copy final.mkv` 实测只产出 2 条流（h264+aac 单音轨）——880Hz 轨被 ffmpeg 默认流选择静默丢弃**。REQ-21"显式全轨映射 `-map 0`"由此坐实为硬性要求，不可省略。

### 5.2 段边界质量（final.mkv，405 帧逐包分析）

【实测】
- 视频帧数 405 = 180+120+105，**无帧丢失**；但按排序 pts 检查，段拼接处时间戳跳变：边界1（6.07s 处）**127ms**、边界2（10.09s 处）**71ms**（QSV h264 带 B 帧，pts 容器序乱序，需按排序/DTS 分析）。
- 音频（两条轨一致）：边界跳变 115ms / 59ms；换算为**样本内容缺口 ≈94ms / ≈38ms**（成因：AAC priming 使每段首包 pts 为 **-0.021s（负 PTS）** + concat 按容器时长偏移的取整）。
- 每段首视频包 pts 偏移不一（0.067/0.027/0.038s，QSV 编码延迟抖动）加剧边界跳变。
- 评价：跳变与缺口均 <150ms（REQ-25 预算内），但**每段边界都发生一次**；"无可检测爆音/跳变"（§8 S5 验收）在本实验只能给出"无包丢失、时戳跳变 ≤127ms、音频缺口 ≤94ms"的量化结论，**主观听感属【未验证】**，应在 S6 内容级同步测试的 concat 边界测点复验（REQ-25 已含该测点）。

### 5.3 崩溃恢复收尾（核心场景：含截断尾段）

【实测】对强杀运行 `crash_qsv_seg5_rt` 的 3 段（含截断尾段）直接 concat：
- 命令同上，**成功**：89ms，duration **13.919s**，3 流齐全，410 帧全部保留（180+120+110）。
- stderr 仅尾段 "File ended prematurely" 与 "Auto-inserting h264_mp4toannexb" 提示（h264_mp4toannexb 为 concat demuxer 读 MKV 时的正常处理提示）。
- 成品完整解码：**exit 0，0 错误**。
- **结论：强杀后的"已完成段+截断尾段"可直接 concat 出可全局解码的成品**；恢复实现可选"直接收尾"或"先 remux 尾段再收尾"，两者本实验均可行（remux 方案可消除截断告警并补 duration，【未验证】逐项对比）。

### 5.4 seg1 运行收尾（7 段）

【实测】`crash_qsv_seg1` 7 个封口段 concat：**119ms**，duration 14.298s ≈ 段和 14.28s，3 流齐全（no-map 版本同样丢音轨）。

## 6. 实验 4：segment_time=1 对照

Tag `crash_qsv_seg1`（强杀，实时，13s）。

【实测】kill 时内容时间 14.61s。产出 **7 个完整段 + 1 个 0 字节尾段**：
- 段长全部 ≈2.03s（2.027–2.067s），**不是名义 1s**：segment_time(1) < GOP(2s) → 切点对齐到 2s 关键帧网格。**有效段长 = max(segment_time, 对齐后的关键帧间隔)**。
- 7 段全部 ffprobe 正常、解码 0 错误、首包 K、3 流。
- **out_007.mkv = 0 字节**：切点处 muxer 创建了新文件，但 cluster（~2MB/5s 阈值）未及刷盘 → 进程被杀后留下空文件；ffprobe 报 `EBML header parsing failed`（exit=AVERROR_INVALIDDATA）。
- δ：δ_closed = 14.61 − 14.28(7 段和) = **0.33s**；尾段 0 可播内容（无 cluster 刷盘）。**同样的杀点，seg1 的结构损失（0.33s）远小于 seg5（4.43s）**：段越长，崩溃时未封口窗口越大——δ 上界与有效段长成正比，与名义 segment_time 无关。

## 7. 实验 5：h264_mf 对照

Tag `crash_mf_seg5`（强杀，实时，13s）。

【实测】kill 时内容 14.61s；切点模式与 QSV 相同（6.058s / 4.026s）；3 段 ffprobe/解码正常（尾段 90 帧可解码，仅截断提示）。
- 尾段刷盘量 786KB / 90 帧（QSV 同杀点为 2.36MB / 110 帧）→ δ_play(MF) ≈ 14.61 − (10.084+3.03) ≈ **1.5s**，大于 QSV 的 0.62s。
- **δ_play 依赖编码器出帧/出流节奏（MF 系统编码器延迟更高），δ≤1s 的承诺不能脱离编码器矩阵（S2）单独冻结**。

## 8. 实验 6：force_key_frames 对齐变体

Tag `crash_qsv_seg5_fkf`（强杀，实时，13s，`-force_key_frames expr:gte(t,n_forced*5)`）。

【实测】段长变为 **5.067s / 5.030s**（各 150 帧，整齐一致），杀点 14.67s，尾段 110 帧。0 解码错误。
- 首次执行时该选项被误置于输出文件名之后，ffmpeg 报 `Trailing option(s) found in the command: may be ignored.` 并忽略（段尺寸与无 fkf 运行逐字节相同暴露了这一点）；**选项必须位于输出之前**——生产侧命令拼装需有参数顺序防错。
- 意义：fkf 使段长可预测（5.0x s），δ_struct 上界收紧为 segment_time + ~0.1s（vs 无 fkf 的 +1.07s 漂移）。**建议生产命令保留 spec §3.3 的 `-force_key_frames`（任务基线命令中无此项，实测应补上）**。

## 9. 边界情形

### 9a. 只读输出目录（模拟权限/磁盘失败）

【实测】`icacls <dir> /deny <user>:(OI)(CI)W` 后启动录制（`logs/readonly_error.log`）：
- ffmpeg **快速失败**（exit=-13，"Conversion failed!"），关键错误 `[out#0/segment] Could not write header (incorrect codec parameters ?): Permission denied`，无残段、无部分文件。
- 错误含 "Permission denied" 字样，可分类映射为 E_DISK_FULL/E_IO 类恢复路径（REQ-22 磁盘不足/IO 错误行为：安全停止并保留内容）。【未验证】录制**中途**磁盘写失败（满盘注入）需 L2 `it_fault_injection_disk_full` 覆盖。

### 9b. 强杀后的文件句柄/锁释放

【实测】`tools/spikes/s5/s5-locktest.ps1`：spawn→13s→`taskkill /F`→WaitForExit（50ms）→立即对全部分段执行重命名/移出移回/副本删除：
- 全部成功，单文件操作 1–5ms，无任何共享冲突。
- **结论：Windows 进程终止时句柄随句柄表同步回收，WaitForExit 返回后即可安全移动/删除分段**——恢复扫描可直接重整目录，无需等待或重试锁。

### 9c. 0 字节/损坏段混入 concat 列表（防御性）

【实测】把 out_007.mkv（0 字节）写入 ffconcat 列表再 concat：concat **不失败**（exit 0），仅一行 stderr（`Error during demuxing: Invalid data found`），**成品静默缩短**（只含 out_000，2.021s）。
- **恢复收尾若仅凭目录文件名生成列表，坏段会被无声丢弃** → 必须按 REQ-22"目录重扫核对 manifest"，收尾后校验成品时长 ≈ 封口段时长和，不符则告警重试。

## 10. δ 与关键数据汇总

| 运行 | 杀时内容时间(单调钟) | 封口段结束 | δ_struct(保守) | 尾段可播 | δ_play(含尾段) | 备注 |
|---|---|---|---|---|---|---|
| crash_qsv_seg5_rt | 14.52s | 10.094s | **4.43s** | 110 帧/3.77s | **0.62s** | QSV，无 fkf，段长 6.07/4.03s |
| crash_qsv_seg5_fkf | 14.67s | 10.097s | 4.57s | 110 帧/3.77s | ≈0.8s | QSV，fkf，段长 5.03/5.07s |
| crash_qsv_seg1 | 14.61s | 14.28s | **0.33s** | 0（0 字节文件） | n/a | 段长被 GOP 抬到 2.03s |
| crash_mf_seg5 | 14.61s | 10.084s | 4.53s | 90 帧/3.03s | **1.5s** | MF，δ_play 超出 1s 目标 |
| grace_qsv_seg5（对照） | 排空至 13.41s | 13.632s（3 段全封口） | 0 | 全部 | 0 | `q` 退出 561ms |

段边界质量（concat 成品）：视频无帧丢失、时戳跳变 ≤127ms；音频样本缺口 38–94ms（AAC priming + 时长取整）；两音轨全程保留。

## 11. 对 REQ-20/21/22 的结论与建议

REQ-20（管线）：
1. 【实测·必须】持续异步排空 stderr/stdout：不排空 ~4KB 即停摆（§2）。
2. 【实测】停止协议两条路径均有界：优雅 EOF/`q` → 561ms 干净退出；强杀 → 50ms 回收。"排空超时强杀"的超时取 1–2s 足够（本实验排空 <600ms）。
3. 【实测】参数顺序防错：输出相关选项必须在输出 URL 之前（fkf 首跑被静默忽略）。

REQ-21（分段与收尾）：
1. 【实测·必须】concat 必须 `-map 0`：缺省流选择丢第 2 音轨（§5.1）。
2. 【实测·建议】录制命令加 `-force_key_frames expr:gte(t,n_forced*segment_secs)`：段长可预测（5.03–5.07s），δ_struct 上界从 "+GOP 漂移(实测+1.07s)" 收紧到 "+0.1s"。任务基线命令应修订。
3. 【实测】段长不可假设等于 segment_time：无 fkf 时受 GOP 网格支配（6.07/4.03s 实录）；segment_time < GOP 时被抬到 GOP 网格（seg1→2.03s）。**损失上界公式应为 Σ(有效段长) 而非名义值**；若要 5s 名义=实际，需 fkf + GOP 整除 segment_time（如 g=fps×segment_secs 或 fps）。
4. 【实测】"崩溃损失 ≤ segment_secs + δ"在两种恢复口径下分别成立：(a) 仅认封口段 → 损失 ≤ 有效段长 + ε（本实验最坏 6.07s 段长、δ_closed≤4.6s）；(b) 回收尾段（remux）→ 损失 = δ_play = 编码器管线+未刷 cluster ≈ 0.6s(QSV)/1.5s(MF)。**建议产品语义取 (b)，δ 按编码器矩阵在 S2 后分编码器冻结；QSV 路径满足 δ≤1s，MF 不满足**。
5. 【实测】收尾耗时量级：concat `-c copy` 13–14s 内容 = 89–136ms（后台收尾目标宽裕）；含 ffprobe 校验的完整收尾预计仍在秒级。【未验证】成品 ≤10s 的实际分布需长会话样本。

REQ-22（恢复状态机）——manifest 需要记录的信息（本实验推导）：
1. 段清单每项：`{path, index, t_start, t_end(内容时间,QPC域), size_bytes, closed, streams_expected}`；`closed` 判定 = "下一 index 段存在"（段 muxer 语义：开新段即封上一段）或优雅 EOF。**0 字节或 probe 失败的段必须排除在封口集合外**（out_007 实证）。
2. 恢复扫描规则（全部实证支持）：排除 (a) size=0；(b) ffprobe EBML 解析失败；(c) mtime 晚于 manifest 最后更新（"段完成但清单未更新"→ 重扫）；(d) 进程仍活着时正在写的尾段（Job 对象保证主进程死后 sidecar 已亡，届时尾段即冻结）。
3. 收尾前置校验 + 收尾后校验：concat 会**静默跳过**坏段（§9c），故 (a) 收尾前列表必须来自 manifest∩重扫结果；(b) 收尾后 `ffprobe` 成品 duration ≈ Σ(封口段 duration)（容差 ~100ms），不符 → 失败重试/告警，不删原段。
4. 尾段回收（可选优化）：`ffmpeg -v error -i tail.mkv -map 0 -c copy tail_fixed.mkv` 可产出补 duration 的可播段，成功则并入、失败则弃（保留原文件）。
5. δ 记账与时间轴用单调钟（QPC）：本次实验墙钟被 NTP 回拨 1.4s，若按墙钟记账会得出"kill 早于最后进度"的荒谬结论（§3）。

## 12. 未决问题 / 未验证项

1. 【未验证】段边界音频缺口（38–94ms）的主观可闻度与爆音风险 → S6 内容级同步测试（REQ-25 concat 边界测点）；如需消除，候选：concat 音频重编码 / `-af aresample` 收尾 / 调整 muxer flush 参数（另测）。
2. 【未验证】录制**中途**满盘/IO 挂起注入（本 spike 仅验证启动期权限失败）→ L2 故障注入测试。
3. 【未验证】段首/段尾不同杀点的矩阵（本 spike 固定 ~14.5s 杀点 ≈ 第 3 段中部）与多路并发强杀 → M3 `it_crash_kill_matrix_recovery_state_machine`。
4. 【未验证】尾段 remux 封口方案与直接 concat 的逐项对比（本次确认两者都可行）。
5. 【未验证】concat 边界 "Auto-inserting h264_mp4toannexb" 在部分播放器上的兼容性（成品为合法 MKV，完整解码通过；播放器矩阵属 L3）。
6. 【未验证】QSV δ_play 0.62s 在真实 WGC 管道供数（命名管道、背压）下是否保持——本 spike 用 lavfi 合成输入，供数特性与生产不同。

## 13. 产物清单

- 脚本：`tools/spikes/s5/{s5-run,s5-verify,s5-concat,s5-locktest}.ps1`
- 日志：`tools/spikes/s5/logs/`（`<Tag>_cmd.txt|_meta.txt|_stderr.log|_probe_*.txt|_decode_*.txt|_verify_summary.json|_concat_*.log|_concat_meta.txt|_locks.json|ffmpeg_version.txt|readonly_error.log|concat_zerobyte_error.log`）
- 运行数据（临时目录，不入仓库）：`%TEMP%\opencode\s5\{crash_qsv_seg5, crash_qsv_seg5_rt, grace_qsv_seg5, crash_qsv_seg1, crash_mf_seg5, crash_qsv_seg5_fkf, locktest_kill}\`
- 本报告：`docs/spikes/s5/s5-crash-recovery.md`

验收对照：强杀后完成段可播证据 ✓（§3 表：全段 ffprobe+解码通过）；concat 合成校验通过 ✓（§5：3 流保留、时长吻合、成品解码 0 错误）；δ 量化 ✓（§10）；优雅停止对比 ✓（§4）；报告落盘 ✓（本文）。
