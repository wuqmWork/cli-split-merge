# v11：0 mm 重叠区域零复制优化

## 目的

当 `overlap_mm == 0.0` 时，分割区域就是静态 `REGIONS`，不再创建或复制 `[Region; 24]`。

## 实现

`split_one_file` 开始处：

```rust
let effective_regions;
let regions: &[Region; REGION_COUNT] = if overlap_mm == 0.0 {
    &REGIONS
} else {
    effective_regions = build_effective_regions(overlap_mm);
    &effective_regions
};
```

后续候选区域筛选、POLYLINE 裁剪、HATCH/HATCHES 裁剪、`$$DIMENSION/` 重写都只接收 `regions` 引用。

## 性能路径

- `0 mm`：直接借用 `&REGIONS`，不复制、不重算。
- `> 0 mm`：每个 CLI 仅计算一次 24 个有效重叠区域，整个文件处理期间复用。

此修改不改变任何分割几何结果，仅减少零重叠路径上的一次小数组复制，并统一区域数据来源。
