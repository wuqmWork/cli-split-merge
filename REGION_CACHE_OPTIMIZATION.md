# v10 重叠区域预计算优化

## 目标

第二轮审查指出 `region_with_overlap` 会在候选区域筛选与几何裁剪中针对每个实体反复计算。
本版把单个 CLI 使用的 24 个有效分割区域在 `split_one_file` 开始时一次性预计算为：

```rust
[Region; 24]
```

之后整个文件处理周期复用该数组。

## 修改内容

### 1. 单文件开始时预计算

```rust
let effective_regions = build_effective_regions(overlap_mm);
```

- `overlap_mm == 0`：直接复制静态 `REGIONS`。
- `overlap_mm > 0`：仅执行一次 24 区域重叠扩展计算。

### 2. POLYLINE

候选区域计算与实际裁剪均直接读取 `effective_regions[r_index]`，不再调用 `region_with_overlap`。

### 3. HATCH / HATCHES

每条 hatch segment 的 bbox 候选筛选和 Liang–Barsky 裁剪都复用预计算区域。
对于大 ASCII CLI，这部分通常是高频热点。

### 4. 输出 DIMENSION

`open_outputs` 同样直接使用同一份 `effective_regions`，保证头部 DIMENSION 与几何裁剪采用完全一致的实际区域。

### 5. 测试

新增 `precomputed_regions_match_direct_overlap_calculation`，对 0 / 0.1 / 2 / 7.5 / 20 mm 五组重叠量逐区域比较预计算结果与原始计算结果。

## 性能影响

该优化不改变分割结果，只减少重复浮点比较、边界判断和 `Region` 构造。
单个实体的绝对收益较小，但在数百万 POLYLINE/HATCHES 线段的 ASCII CLI 中可累计降低 CPU 开销。

候选区域快速索引仍然保留，不会退化为 24 区域暴力扫描。
