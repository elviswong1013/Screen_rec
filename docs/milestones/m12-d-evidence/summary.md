# 区域截图 E2E 结果（自动生成）

- 时间：2026-10-17 21:03:55
- 应用：`C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\target\release\screen-rec.exe`
- 输入通道：mouse_event(legacy)
- 虚拟桌面：5280x3002 @ (0,0)
- 拖拽矩形（物理像素）：600,300 900x600

| 场景 | 结果 | 明细 | 证据 |
|---|---|---|---|
| a0. 覆盖层 WebView2 初始化 | PASS | overlay=5280x3002；children=WRY_WEBVIEW[True], Chrome_WidgetWin_0[True], Chrome_WidgetWin_1[True], Chrome_RenderWidgetHostHWND[True], Intermediate D3D Window[True] | C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\docs\milestones\m12-d-evidence\overlay-mask.png |
| a1. 覆盖层遮罩真实绘制 | PASS | 桌面亮度均值 116.587 → 50.347（ratio=0.432；遮罩 alpha=0.55） | C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\docs\milestones\m12-d-evidence\overlay-mask.png |
| a2. 拖拽生成 PNG | PASS | C:\Users\elvis\Videos\Screen_rec\shot_region_1792242209017_0000.png（覆盖层已关闭=True，耗时 1880ms） | C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\docs\milestones\m12-d-evidence\region-shot.png |
| a3. PNG 物理尺寸 = 拖拽矩形 | PASS | 900x600（期望 900x600） | C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\docs\milestones\m12-d-evidence\region-shot.png |
| a4. 成品 = 覆盖前底层（无遮罩/边框污染） | PASS | meanDiff=0 maxDiff=0 changedPixels=0/540000 | C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\docs\milestones\m12-d-evidence\region-shot.png |
| b1. Esc 关闭、无新文件、无残留窗口 | PASS | gone=True newPng=0 appAlive=True residualOverlay=0 | C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\docs\milestones\m12-d-evidence\after-esc.png |
| c1. 失败时覆盖层保持可见并显示错误 | PASS | overlayVisible=True errorHintPixels=2843 outputDirCreated=False | C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\docs\milestones\m12-d-evidence\failure-output-unavailable.png |

结论：全部 PASS
