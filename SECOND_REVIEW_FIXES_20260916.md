# 第二轮审查修复记录（v9）

基于《第二轮代码审查报告（2026-09-16，v8-overlap）》处理。

## 已修复

- B-1：Cargo.toml 为 tauri 启用 `protocol-asset`。
- B-2：merge 数值格式化与 split 统一，整数不再输出 `.0`。
- B-3：移除 `lib.rs` 未使用的 `Manager` import。
- H-1：分割启动前对全部 `24 × N` 最终输出路径做全局去重，命名模板冲突直接拒绝，避免并行 `.cli.part` 数据损坏。
- H-2：首次启动重叠量默认值恢复为 2.0 mm。
- H-3：重叠量输入框改为字符串中间态，允许正常键入 `0.5` / `2.5`，失焦或回车后再校验到 0～20 mm。
- H-4：文件对话框取消返回 `null`，不再清空已有 CLI 选择。
- M-1：split 前端进度使用 `Math.max` 保证不回跳；后端用 `AtomicUsize` 统计并行真实完成数。
- M-2：split 的非法 `$$UNITS/` 不再静默回退 1.0，改为明确报错；缺失 UNITS 仍按 CLI 默认 1.0。
- M-3：README / MERGE_IMPLEMENTATION / AI_IMPLEMENTATION 增加“合并输入必须同一坐标系”约束。
- L-1（部分性能/卫生）：
  - `format_cli_number` 与几何数字写出统一格式规则；
  - 接缝外场边界魔数提取为 `FIELD_X_MAX / FIELD_Y_MIN / FIELD_Y_MAX`；
  - merge 写阶段进度事件加入 250 ms 节流。

## 新增测试

- 命名模板导致跨输入输出冲突时必须拒绝。
- split 非法 UNITS 必须拒绝。

## 仍可继续的低优先级优化

- `region_with_overlap` 可进一步在单文件开始时预计算 `[Region; 24]`，减少每个实体重复构造；当前逻辑正确，属于微优化。
