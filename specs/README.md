# Specs 工作流（PRD → SDD → TDD）

本仓库的规格文档遵循三阶段流水线，任何阶段不得先于上一阶段的 gate 开始。

## 阶段

1. **PRD**（`docs/prd/NNN-short-name.md`，`Status: Draft`）
   - 背景与目标、非目标、用户故事、编号验收标准 AC-1..N、约束、开放问题。
   - Gate：用户明确批准 AC 列表。批准前不写规格、不写代码。
2. **SDD 规格**（`specs/NNN-short-name.md`）
   - 编号需求 REQ-1..N（每个 AC 至少映射 1 个 REQ）、架构/接口/错误路径、测试计划（每个 REQ 映射到至少一个有名字的测试）、Out of scope。
   - Gate：@plan-reviewer 返回 APPROVED + 用户批准；`Status: Draft → Approved`。批准前不存在生产代码。
3. **TDD 实现**
   - 每个任务：Red（@test-writer 写失败测试）→ Green（@executor 最小实现）→ Refactor（保持全绿）。
   - Done：全套测试绿 + 每行改动可追溯到 REQ + @acceptor 返回 ACCEPTED；`Status: Approved → Implemented`。

## 状态流转

`Draft → Approved → Implemented（→ Superseded）`

已批准/已实现规格的修改属于变更请求：先同步代码与测试，再重跑受影响的 gate。

## 文件

- `README.md`（本文件）：工作流说明
- `_template.md`：规格模板
- `NNN-short-name.md`：具体规格（编号递增）
