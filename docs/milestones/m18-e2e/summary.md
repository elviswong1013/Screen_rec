# 区域截图 E2E 结果（自动生成）

- 时间：2026-10-19 03:53:10
- 应用：`target/debug/screen-rec.exe`
- 输入通道：mouse_event(legacy)
- 虚拟桌面：5280x3002 @ (0,0)
- 拖拽矩形（物理像素）：600,300 900x600

| 场景 | 结果 | 明细 | 证据 |
|---|---|---|---|
| d0. 区域录制覆盖层已打开 | PASS | overlay=5280x3002 |  |
| d1. 确认后覆盖层隐藏（DWM 合成窗口 ≥150ms） | PASS | gone=True |  |
| d1b. 覆盖层确认的最终尺寸可读（精确校验基准） | PASS | 398x120 |  |
| d2. 录制已启动（UIA 可见「全部停止」） | PASS | started=True |  |
| d3. 停止后产出成品 | PASS | final=C:\Users\elvis\Videos\Screen_rec\20261019_035206_Region 398x120 (U27G4WS).mkv |  |
| d4. 成品尺寸 = 覆盖层确认的最终尺寸（精确） | PASS | 398x120（确认 398x120；非整屏） |  |
| d5. MKV 容器 + 时长覆盖录制段（含 UIA 启动等待） | PASS | format="matroska,webm" dur=16.49s |  |
| d6. 全音轨（默认系统）恰 1 轨 | PASS | audioTracks=1 |  |
| d7. 真正第一帧与后续帧无覆盖层（纯绿目标未被遮罩压暗） | PASS | RGB(0,255,0) RGB(0,255,0) RGB(0,255,0) RGB(0,255,0) | docs\milestones\m18-e2e\record-first-frame.png |
| d8. 系统音轨含 660Hz 正例 | PASS | 660Hz=-41.1dB；880Hz=-69.3dB（未播放，仅参考，不作负例） |  |
| d9. 真实 UI 切到「隔离」音轨模式 | PASS | selected=True |  |
| d9b. 隔离场景覆盖层确认/隐藏 | PASS | gone=True |  |
| d10. UI 勾选 660 应用（UIA AutomationId = PID） | PASS | cfg-region-app-101932 |  |
| d11. 隔离场景录制已启动（UIA 可见「全部停止」） | PASS | started=True |  |
| d12. 隔离场景停止后产出成品 | PASS | final=C:\Users\elvis\Videos\Screen_rec\20261019_035253_Region 398x120 (U27G4WS).mkv |  |
| d13. 隔离成品恰 1 条应用音轨（无系统混音） | PASS | audioTracks=1 |  |
| d14. 隔离成品尺寸 = 确认矩形（精确） | PASS | 398x120（确认 398x120） |  |
| d15. 隔离音轨 FFT 单频点：660 存在、未选 880 至少低 15dB | PASS | 660Hz=17.7dB 880Hz=-110.2dB diff=127.9dB（Goertzel DFT bin） | docs\milestones\m18-e2e\region-record-isolated.mkv |

结论：全部 PASS
