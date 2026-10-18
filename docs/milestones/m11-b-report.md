# M11-B 验收整改报告（前端 · 区域截图物理坐标映射）

日期：2026-10-17
范围：仅 `ui/` + `tools/`（验证脚本）；未改 Rust / tests / specs；未 git 提交。

## 背景

@acceptor REJECTED：原区域截图为应用内 mock 覆盖层（ui/app.js 797–832），只发送
`clientX/clientY`（CSS 像素、相对主窗口），未映射到桌面物理坐标，REQ-6/REQ-8 不成立。

本次整改将区域框选迁移到后端全屏覆盖层窗口（label=`region-overlay`，加载
`ui/region.html`），按固定契约换算物理坐标后再调用 `take_screenshot`。

## 文件清单

| 文件 | 变更 |
|---|---|
| `ui/region.html` | 新增。覆盖层页面：全屏半透明遮罩（复用 styles.css 的 `.region-overlay/.region-hint/.region-sel/.region-size`），`body{background:transparent}` 依赖后端窗口透明以透出桌面 |
| `ui/region.js` | 新增。覆盖层交互：启动取 `get_overlay_geometry()`；左键拖拽绘制选区、尺寸提示实时显示物理像素；鼠标松开/Enter 确认 → `clientRectToPhysical` 换算后 `invoke('take_screenshot', {request:{target:{kind:'region', rect:{x,y,w,h}}}})`，成功后 `close_region_overlay()`；Esc 直接 `close_region_overlay()`；截图失败在覆盖层提示可重试；无 `__TAURI__` 时降级为浏览器 mock 便于调试 |
| `ui/region-geometry.js` | 新增。纯函数换算模块（UMD：`window.RegionGeometry` + `module.exports`），`clientToPhysical` / `normalizeClientRect` / `clientRectToPhysical`；页面与 Node 探针复用同一份实现 |
| `ui/api.js` | real 分支新增 `getOverlayGeometry` / `openRegionOverlay` / `closeRegionOverlay` 封装（含 JSDoc 几何类型）；mock 分支补浏览器降级（`window.open('region.html')`） |
| `ui/app.js` | 删除原应用内 mock 覆盖层逻辑（797–834）；`#btn-region-shot` 改为调用 `api.openRegionOverlay()`，失败 toast |
| `ui/index.html` | 删除已无引用的 `#overlay-root` 容器（mock 覆盖层唯一挂载点） |
| `tools/probe-region-geometry.js` | 新增。Node 探针，直接 `require` 生产代码 `ui/region-geometry.js` 举证换算 |

## 验证结果（观察到）

1. `node --check ui/region.js ui/region-geometry.js ui/api.js ui/app.js tools/probe-region-geometry.js` —— 全部语法通过（`ALL_SYNTAX_OK`）。
2. `node tools/probe-region-geometry.js` —— 6/6 PASS：
   - 主用例：窗口位于物理 `(500,300)`、`scale=1.5`，`client(10,20)` → 物理 `(515,330)` ✅；
   - `rect(10,20,100,50)` → `{x:515, y:330, w:150, h:75}` ✅；
   - 负 origin（多屏左副屏）`client(100,100)` → `(-1820, 40)` ✅；
   - `scale=1.25` 舍入 `client(3,7)` → `(504, 309)` ✅；
   - 微小矩形 w/h 下限 1px（后端拒绝 0）✅；
   - 反向拖拽归一化 ✅。
3. 静态检查：`grep overlay-root` / `grep "kind: 'region'" ui/app.js` 均无残留，mock 路径已清除。

## 手工验证步骤（需后端 M11 命令落地后在真实环境执行）

1. 启动应用 → 截图页 → 「框选区域截图」：应弹出无边框置顶全屏覆盖层，桌面透过半透明遮罩可见。
2. 在**跨两台显示器**的范围拖拽（含副屏负坐标区域），观察尺寸提示为物理像素（= client × scale，例如 150% 缩放屏上拖 100 CSS px 应提示 150）。
3. 松开鼠标确认：覆盖层关闭，PNG 保存到输出目录；用 ffprobe/属性核对 PNG 像素尺寸 = 提示的物理尺寸，内容与拖选区域一致。
4. 再次进入覆盖层按 **Esc**：覆盖层关闭、无残留窗口、输出目录无新文件。
5. 拖出 <10px 的微小选区：应提示「选区过小」并允许重新拖拽，不发截图请求。
6. 浏览器降级自测（无 Tauri）：直接双击 `ui/region.html`，拖选后控制台应输出 `[region mock] take_screenshot region rect = …`。

## 未决（契约缺口 / 假设，未私改 Rust）

1. **DTO 形状偏差（已按现状实现）**：任务描述的契约字面为 `Region:{x,y,width,height}`，
   但 `commands.rs` 实际 DTO 为 `TargetIn::Region { rect: RectIn{x,y,w,h} }`
   （`#[serde(tag="kind")]` → `{kind:'region', rect:{x,y,w,h}}`）。实现以现有 DTO 为准；
   若后端 M11 改动此形状需同步 `region.js` 的 `backend.takeScreenshot`。
2. **结果回传**：`take_screenshot` 由覆盖层窗口直接调用，主窗口「本次会话截图记录」
   不会自动追加保存路径——契约未定义完成事件/回传通道。当前行为：成功后覆盖层直接关闭。
3. **窗口透明假设**：`region.html` 设 `body{background:transparent}`，假定后端
   `region-overlay` 窗口创建时启用透明；若为不透明窗口，用户将看到纯色遮罩而非桌面。
4. **几何时序**：`region.js` 启动时一次性取 `get_overlay_geometry()`；假定窗口已按
   虚拟桌面全屏摆好且加载期间不发生移动/DPI 变化。
