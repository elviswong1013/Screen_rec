// ============================================================================
// tools/probe-region-geometry.js — M11-B 坐标换算探针（Node，无依赖）
//
// 直接 require 生产代码 ui/region-geometry.js，验证契约换算：
//   物理坐标 = origin + client * scale
// 核心举证：overlay 窗口位于物理 (500,300)、scale=1.5 时，
//   client(10,20) → 物理 (515,330)。
// 另覆盖：负 origin（多屏左侧显示器）、非整数缩放的四舍五入、w/h 下限 1。
//
// 运行：node tools/probe-region-geometry.js   （失败时 exit 1）
// ============================================================================
'use strict';
const G = require('../ui/region-geometry.js');

let failed = 0;
function check(name, actual, expected) {
  const ok = JSON.stringify(actual) === JSON.stringify(expected);
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}: got ${JSON.stringify(actual)}, want ${JSON.stringify(expected)}`);
  if (!ok) failed++;
}

// 主用例：窗口位于物理 (500,300)，scale=1.5，client(10,20) → 物理 (515,330)
const geom = { origin_x: 500, origin_y: 300, width: 2560, height: 1440, scale: 1.5 };
check('clientToPhysical (500,300)/1.5 client(10,20)', G.clientToPhysical(geom, 10, 20), { x: 515, y: 330 });
check('clientRectToPhysical (500,300)/1.5 rect(10,20,100,50)',
  G.clientRectToPhysical(geom, { x: 10, y: 20, w: 100, h: 50 }),
  { x: 515, y: 330, w: 150, h: 75 });

// 多屏负坐标：副屏在主屏左侧（虚拟桌面 origin 可为负，REQ-6）
const geomNeg = { origin_x: -1920, origin_y: -60, width: 1920, height: 1080, scale: 1.0 };
check('clientToPhysical 负 origin client(100,100)', G.clientToPhysical(geomNeg, 100, 100), { x: -1820, y: 40 });

// 非整数缩放四舍五入：scale=1.25，client(3,7) → 500+3.75≈504，300+8.75≈309
const geom125 = { origin_x: 500, origin_y: 300, width: 2560, height: 1440, scale: 1.25 };
check('clientToPhysical 1.25 舍入 client(3,7)', G.clientToPhysical(geom125, 3, 7), { x: 504, y: 309 });

// w/h 下限：物理尺寸至少 1px（后端 RectIn 拒绝 w/h=0）
const geom1 = { origin_x: 0, origin_y: 0, width: 1920, height: 1080, scale: 1.0 };
check('clientRectToPhysical 微小矩形下限 1px',
  G.clientRectToPhysical(geom1, { x: 5, y: 5, w: 0.4, h: 0 }),
  { x: 5, y: 5, w: 1, h: 1 });

// 拖拽归一化：反向拖拽得到非负宽高
check('normalizeClientRect 反向拖拽', G.normalizeClientRect(120, 90, 40, 30), { x: 40, y: 30, w: 80, h: 60 });

if (failed) {
  console.error(`\n${failed} 个用例失败`);
  process.exit(1);
}
console.log('\n全部通过');
