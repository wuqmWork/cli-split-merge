# CLI 分割重叠区域实现

## 功能

CLI 分割页新增“重叠区域”，默认 `2.0 mm`。该值表示**相邻两个振镜分割区域最终公共重叠宽度**，而不是每侧各 2 mm。

- 默认：`2.0 mm`
- 允许范围：`0 ~ 20 mm`
- UI 步长：`0.1 mm`
- 本地记忆：`cli-split-merge.overlap-mm.v1`
- `0 mm`：恢复无重叠模式，并启用共享边界唯一归属
- `> 0 mm`：内部共享边界两侧各扩大 `overlap / 2`，重叠带内几何允许同时写入相邻振镜
- 24 振镜整体最外边界不扩大

## 2 mm 示例

原始上下区域边界：

```text
上区 Y = [-140, 140]
下区 Y = [-420, -140]
```

设置 `2.0 mm` 后：

```text
上区 Y = [-141, 140]
下区 Y = [-420, -139]
公共重叠 = [-141, -139] = 2 mm
```

左右内部边界采用完全相同的规则。四区交点会自然形成约 `2 mm × 2 mm` 的公共重叠区域。

## UI 布局

```text
参数文件                         重叠区域
[ params.xlsx ][选择文件]       [ 2.0 ] mm
已记忆当前参数  加载于 ...

输出文件夹
[ D:\output ][选择文件夹] [开始分割]
```

## 后端

`SplitRequest` 新增：

```rust
pub overlap_mm: Option<f64>
```

未传入时后端同样默认 `2.0 mm`，防止非 UI 调用绕过默认值。

主要实现：

- `region_with_overlap()`：只扩展内部边界
- `candidate_regions_for_bbox(..., overlap_mm)`：候选区域索引同步考虑重叠宽度
- `clip_polyline(..., unique_shared_boundary)`：重叠开启时允许接缝重复写入
- `process_hatches_line(..., overlap_mm, ...)`：HATCH/HATCHES 同步使用扩展区域
- `rewrite_split_dimension()`：输出 CLI 的 `$$DIMENSION/` 使用扩展后的区域计算

## 测试

新增 `two_mm_overlap_creates_two_mm_shared_band`：

1. 验证上下区域各扩展 1 mm；
2. 验证最终公共重叠带正好 2 mm；
3. 验证原接缝上的线在重叠开启时同时进入相邻两个振镜。
