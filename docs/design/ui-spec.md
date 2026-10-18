# Screen_rec UI 设计说明（M9）

适用范围：`ui/` 前端（中文单页桌面应用）。命令契约见 specs/001-screen-recorder.md §3.3，
数据模型见 §3.4。本文档描述布局与层级、交互状态机、命令映射、空态/错误态与键盘操作。

## 1. 技术形态

- 零构建链：原生 JS + 一份 CSS + Tauri `withGlobalTauri`（`window.__TAURI__.core`）。
  无框架、无打包器、无 CDN，浏览器直接打开 `ui/index.html` 即可运行（自动进入 mock 模式）。
- **加载方式用经典脚本而非 ES modules**：file:// 协议下 Chrome/Edge 的 CORS 策略会拦截
  外部 module import（双击打开时脚本完全不执行）；经典 `<script src>`（api.js 先于 app.js）
  在 file:// 与 Tauri webview 中均可用。模块边界不变：api.js 经 `window.SR` 导出。
- 分层：`ui/api.js` 是唯一与后端交互的模块。检测到 `window.__TAURI__` 时走真实 `invoke`；
  否则使用内置 mock（示例数据 + 模拟事件流）。接线时只替换 `api.js` 的 real 分支。
- 真实分支约定（Tauri v2）：Rust snake_case 参数在 JS 侧以 camelCase 传递
  （`stop_route(route_id)` → `{ routeId }`）；UI 附加的 `_ui_*` 展示元数据在 invoke 前剥离。
- mock 场景开关（URL query，用于自测/演示）：
  `?scenario=degraded`（能力降级）/ `error`（录制中注入 E_TARGET_LOST）/
  `empty`（空列表）/ `norecovery`（无恢复会话）。

## 2. 布局与层级

```
┌──────────┬────────────────────────────────────────────────┐
│ 导航栏    │ 恢复横幅（条件出现，最高优先级）                  │
│  品牌     ├────────────────────────────────────────────────┤
│  录制(●N) │ 视图区（录制 / 截图 / 合并 / 设置，互斥切换）      │
│  截图     │                                                │
│  合并     │                                                │
│  设置     │                                                │
│  ─────── │                                                │
│  环境徽标  │                                                │
└──────────┴────────────────────────────────────────────────┘
```

- **导航栏（196px）**：四个一级视图；「录制」项带活动任务计数徽标（红色，来自 route 状态聚合）；
  底部环境徽标（`演示数据（mock）` / `已连接后端`）+ OS build。
- **录制视图**（默认）：
  - 顶部：模式页签（录屏 / 纯录音）+ 能力徽标行（按应用音频 / 编码器 / 麦克风权限 / 画面捕获）。
  - 降级提示条（条件出现，warn）：逐条说明被禁用的能力与替代路径（AC-8），不静默。
  - **录制任务区**（条件出现，位于配置区上方）：卡片网格 + 「全部停止」。
    保留配置区可继续操作，以支持多路录制期间追加新一路（AC-3）。
  - 配置区双栏：左=目标（窗口/显示器页签、搜索、刷新）；右=录制配置
    （画布、帧率、光标、音轨模式、音源、输出目录）。`≤940px` 折为单栏。
  - 底部吸底开始栏：当前选择摘要（目标数 · 画布 · fps · 音轨模式）+ 开始按钮。
- **截图视图**：显示器 / 窗口 / 区域三卡片 + 本次会话截图记录列表。
- **合并视图**：左=输入文件（勾选 + 手动添加路径 + 合并顺序角标）；右=输出路径与进度。
- **设置视图**：单栏表单（输出目录、默认帧率、分段时长、默认音轨模式、麦克风设备、编码器、光标、保留原始分段）。

### 视觉

- 深色单一主题（录制工具场景，长时间注视友好）：背景 `#0e1014`，面板 `#1a1f2b`，
  边框 `#2b3242`；主色 `#4f7cff`（操作），录制红 `#f04747`（仅用于录制相关动作与状态），
  语义色：成功 `#3fb67c` / 警告 `#e0a03c` / 错误 `#e5534b`。
- 字体：系统栈（Segoe UI / 微软雅黑），数字与计时用等宽（Cascadia Mono / Consolas）；
  离线可用，无网络字体。
- 密度：适中——列表行高约 38px，卡片间距 12–16px，信息以单层平铺为主，不用折叠面板藏核心操作。
- 动效克制：视图切换 0.22s 淡入、录制红点 1.4s 呼吸、电平条 0.12s 线性跟随（≤10Hz 事件流）。
  无装饰性动画。

## 3. 交互状态机

### 3.1 应用级

```
启动加载 ──► 空闲(配置) ──开始录制──► 录制中 ──全部停止──► 收尾中 ──► 空闲
              ▲   │                      │                     │
              │   │ 部分停止              │ 单路错误(仅该路)       │ 成品就绪(route_finalized)
              │   └──────────────────────┘                     ▼
              └────────────────────────────── 完成卡片(可移除)
```

- 录制中**不禁用**配置区（支持追加新一路）；开始按钮在启动请求在途时置「启动中…」防重入。
- 收尾（finalizing）在后台进行，不阻塞其它操作（REQ-21 / D10）；完成经 `route_finalized` 通知。

### 3.2 单路（route 卡片）

后端状态机 `recording → stopping → finalized | failed`（route_finalized/route_error 为终态事件），
UI 卡片状态：

```
starting → recording → stopping → done(=finalized)
                │           └────► failed（route_state failed / route_error 终态）
                └─ route_error(severity=error) ─► failed；若随后仍来 route_finalized
                   （保留内容的优雅停止）→ done，错误框保留展示
```

- 卡片呈现：目标名、类型图标、状态徽标、计时（等宽 hh:mm:ss）、每音轨一条电平表、
  错误框（错误码+说明）、完成后成品路径（多文件逐行）、操作按钮（停止 / 移除）。
- 错误不阻断卡片生命周期展示：错误徽标+错误框保留，同时状态继续推进（优雅停止语义）。

### 3.3 恢复横幅

```
启动 recover_sessions() ──空──► 不显示
                        └─非空─► 横幅列出每个会话（会话号/路数/目标摘要/开始时间）
                                  ├─ 收尾（finalize_session，进度指示，失败保留可重试）
                                  └─ 丢弃（两步确认：先点「丢弃」→ 变为「确认丢弃？」→ discard_session）
```

### 3.4 区域截图

```
点击「框选区域截图」→ 覆盖层(crosshair) ──拖拽──► 松手且 ≥10×10 ──► take_screenshot(region)
                       │ Esc / 选区过小 ──► 取消（toast 提示）
```

浏览器 mock 中覆盖层作用于应用窗口（CSS 像素）；真实环境由后端全屏覆盖层实现同一交互，
坐标映射到物理像素并支持跨显示器/负坐标（REQ-6/8）。

## 4. 命令映射（契约定稿：docs/milestones/m9-commands-report.md）

命名规则：顶层参数 camelCase（`routeId`/`sessionId`/`outputDir`），对象内部字段 snake_case
（`canvas_w`/`created_at_unix_ms`/`output_dir`/`device_id`）；事件与快照字段 snake_case。

| UI 动作 | 命令 | 备注 |
|---|---|---|
| 启动加载 | `get_settings` / `list_audio_devices` / `probe_capabilities` / `recover_sessions` / `list_targets` | 顺序容错，单个失败仅 toast；probe 在 api.js 归一化（`init_ok→available`、`mic_privacy:bool→mic_ok`、`canvas_limits.max_width→max_w`），UI 不直接依赖后端字段名 |
| 刷新按钮 / Ctrl+R | `list_targets` + `list_audio_sources` | 骨架屏；清理已消失目标的选中态；音源条目含 `created_at_unix_ms`（REQ-14 身份校验必需） |
| 开始录制（录屏） | `start_routes({routes})` | 每选中目标一路；全成功或回滚 |
| 开始录音（纯录音） | `start_audio_only({sources, outputDir})` | 一次调用 N 源 → N 个共享 handle 的 route id（任一停止即整组停止）；UI 按 sources 顺序把 route_ids 映射为每源一张卡片 |
| 卡片「停止」 | `stop_route({routeId})` | 返回 `{requested}`；成品经事件 |
| 「全部停止」 | `stop_all()` | 返回 `{requested: [...]}` |
| 显示器/窗口/区域截图 | `take_screenshot({request: {target}})` | target 三选一 `{kind, hwnd|index|rect}`；`output_dir` 省略=设置值 |
| 开始合并 | `merge_recordings({paths, out})` | ≥2 输入；**无进度事件**（§8-3），UI 用不定态进度条 |
| 恢复横幅 | `recover_sessions` / `finalize_session({sessionId})` / `discard_session({sessionId})` | 见 §3.3 |
| 设置保存 | `set_settings({settings})` | 前端先做空值/范围校验 |
| 事件流 | `subscribe({channel})` → 立即返回 `StateSnapshot {routes:[{id,state,final_path}], recovery:[SessionOut]}` | api.js 把快照以 `_snapshot` 事件转交 UI 重建卡片（快照无目标名/音轨标签，重建卡片以 route id 为名） |

**后端校验在前端的预检**（避免整批回滚）：
- `isolated` 不得含 `system` 音源（`E_INVALID_AUDIO_PLAN`）→ 显示器目标一律按 `all_tracks`
  携带显式音源；窗口目标 isolated = 应用轨(+麦克风)。
- 任一路音源为空 → 前端拦截并提示（显示器目标默认勾系统音频，用户可取消）。

**恢复横幅规则**（SessionOut 新字段，§8-5/8-7/8-8）：
- `routes[].final_path` 全非空或 `state=="final"` → 「历史会话（已收尾）」，仅「清理」；
- `manifest_ok && sealed_segments 为空 && tail_segment==null` → 「空会话残留」，仅「清理」；
- `active==true` 不提供「丢弃」（discard 无条件删目录）；
- `finalize_session` 返回 `{finals, repaired_wavs}`，toast 展示成品路径。

**与后端的已知缺口（如实呈现，不伪造）**：
- `route_level` 暂无发布者（§8-1）：真实模式录制 3s 无电平事件 → 卡片显示
  「电平读数待管线接入，录制本身不受影响」；mock 正常出电平。事件 track 为 `"a0"` 字符串，UI 兼容数字下标。
- 合并无进度钩子（§8-3）：不定态进度条；mock 的 `merge_progress` 事件收到后切换为确定进度。
- 后端无 `finalizing` 进度态：状态机 `recording → stopping → finalized|failed`，
  UI 以 `applyRouteState` 映射（finalized→done）；卡片保留 finalizing 渲染以兼容未来扩展。
- `route_error` 为终态事件；若随后仍来 `route_finalized`（保留内容的优雅停止），以 finalize 为准，
  错误框保留展示。
- 区域录制（`kind:"region"` 的 RouteConfig）后端未实现（§8-2）→ UI 不提供区域录制入口，区域仅用于截图。

## 5. 空态 / 错误态 / 加载态

| 场景 | 呈现 |
|---|---|
| 目标列表加载中 | 骨架行 ×5 |
| 无窗口 / 搜索无结果 / 无显示器 | 图标 + 说明文案的空态 |
| 未选目标 / 未选音源 | 开始按钮禁用 + 摘要栏引导文案 |
| 无可用编码器 | 错误提示条（列出各编码器探测结果）+ 开始禁用（§9 T1） |
| 环回不可用 / 麦克风权限拒绝 | 警告提示条 + 对应控件禁用，其余能力可用（AC-8） |
| 应用音源为空（全音轨/纯录音） | 「当前没有正在发声的应用」 |
| 录制卡片 | starting（准备中）/ finalizing（进度条）/ 错误框 / 完成路径 各有专态 |
| 截图记录 / 合并候选为空 | 空态 + 引导（手动添加路径） |
| 命令失败 | toast（错误，保留 6.5s 若含路径）+ 原控件恢复可点；恢复操作失败保留横幅可重试 |
| 合并失败 | 面板内错误条（明示输入文件已保留）+ toast |

## 6. 键盘操作

| 按键 | 作用 |
|---|---|
| Ctrl+Enter | 录制视图：开始录制/录音（等同主按钮，含禁用态） |
| Ctrl+R | 录制视图：刷新目标与音源列表（浏览器中会 preventDefault） |
| Esc | 取消区域框选覆盖层 |

## 7. 与托盘的衔接

托盘（显示/退出）由 src-tauri 实现（REQ-30 骨架已在）。前端配合点：
- 关闭主窗口不退出进程，录制不中断；前端通过 Channel 快照重绑恢复状态（REQ-31）——
  mock 模式无需处理，真实模式下 `api.js` real 分支的 `subscribe` 返回快照后按 §3.2 重建卡片。
- 托盘「开始/停止」菜单项待后端追加；届时仅触发与主按钮相同的命令，无需前端改动。

## 8. 文案基调

如实、简洁、可操作：错误提示含错误码与一句人话说明；能力降级说明「什么不可用 + 为什么 + 还能用什么」；
不使用营销性修饰。
