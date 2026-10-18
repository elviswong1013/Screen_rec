// ============================================================================
// ui/region-geometry.js — 覆盖层 client 坐标 → 桌面物理坐标换算（REQ-6/REQ-8）
//
// 契约（与后端 M11 固定，名称不可改）：
//   get_overlay_geometry() -> { origin_x: i32, origin_y: i32,
//                               width: u32, height: u32, scale: f64 }
//   origin_*：overlay 窗口左上角在虚拟桌面中的物理坐标（负坐标合法，多屏）。
//   scale：CSS 像素 → 物理像素的比例（DPI 缩放）。
//   物理坐标 = origin + client * scale（四舍五入到整像素）。
//
// 双环境加载：region.html 用经典 <script> 引入（挂 window.RegionGeometry）；
// Node 探针（tools/probe-region-geometry.js）用 require 引入（module.exports）。
// ============================================================================
(function (root, factory) {
  const api = factory();
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  root.RegionGeometry = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  /**
   * 单个 client 点 → 物理点。
   * @param {{origin_x:number, origin_y:number, width:number, height:number, scale:number}} geom
   * @param {number} cx client X（CSS px，相对 overlay 窗口左上）
   * @param {number} cy client Y
   * @returns {{x:number, y:number}} 物理像素坐标（可为负，REQ-6）
   */
  function clientToPhysical(geom, cx, cy) {
    return {
      x: Math.round(geom.origin_x + cx * geom.scale),
      y: Math.round(geom.origin_y + cy * geom.scale),
    };
  }

  /**
   * 归一化拖拽两端点为非负宽高的 client 矩形。
   * @returns {{x:number, y:number, w:number, h:number}}
   */
  function normalizeClientRect(x0, y0, x1, y1) {
    return {
      x: Math.min(x0, x1),
      y: Math.min(y0, y1),
      w: Math.abs(x1 - x0),
      h: Math.abs(y1 - y0),
    };
  }

  /**
   * client 矩形 → 物理矩形（RectIn 形状：{x, y, w, h}；w/h ≥ 1，物理像素）。
   * @param {{x:number, y:number, w:number, h:number}} rect client 矩形（CSS px）
   * @returns {{x:number, y:number, w:number, h:number}}
   */
  function clientRectToPhysical(geom, rect) {
    const p = clientToPhysical(geom, rect.x, rect.y);
    return {
      x: p.x,
      y: p.y,
      w: Math.max(1, Math.round(rect.w * geom.scale)),
      h: Math.max(1, Math.round(rect.h * geom.scale)),
    };
  }

  return { clientToPhysical, normalizeClientRect, clientRectToPhysical };
});
