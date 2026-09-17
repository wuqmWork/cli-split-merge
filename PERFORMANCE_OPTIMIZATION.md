# CLI 分割性能优化 v4

本轮目标：在保持分割几何结果和 24 振镜输出规则不变的前提下，优先优化 ASCII CLI 的热点路径。

## 已完成优化

### P0-1：HATCHES 从“每条线段扫描 24 区域”改为二维快速候选定位

旧逻辑：

```text
每条 HATCH 线段
  -> 对 24 个振镜区域逐个做裁剪判断
```

新逻辑：

```text
线段 bbox
  -> 根据 X 直接定位 12 个列区间
  -> 根据 Y 判断上/下两排
  -> 只对真正可能相交的 1~4 个区域做裁剪
```

随机短线段测试下，候选区域平均约 1.09 个，而旧逻辑固定检查 24 个区域。
空间候选算法已用 100000 组随机 bbox 与旧 24 区域扫描做集合等价校验。

### P0-2：HATCHES 单遍分发

旧逻辑先把全部 HATCHES 解析成 `Vec<segment>`，随后 24 次遍历所有线段。

新逻辑边解析边分发：

```text
ASCII 字段 -> 坐标 -> 候选区域 -> clip -> C3→振镜局部 -> 对应区域缓冲
```

只扫描一次坐标数据。

### P0-3：复用 24 个 HATCH 输出缓存

24 个区域的 `Vec` 只创建一次，每条 HATCHES 使用 `clear()` 复用容量，减少大量小对象分配和 allocator 压力。

### P0-4：变换参数由 HashMap 改为固定数组

程序启动分割时一次性把 A0~F3 变换整理为与 `REGIONS` 同序的 `[Transform2D; 24]`。
热点循环中直接 `transforms[index]` 访问，不再做字符串 HashMap 查询。

### P0-5：避免每行 `to_ascii_uppercase()` 分配

CLI 命令判断改成 `eq_ignore_ascii_case` / 无分配前缀判断。
大文件不再为每一行额外创建 uppercase String。

### P0-6：数字解析改用 lexical-core

ASCII 坐标的 `f64 / usize / i32` 解析改为 `lexical-core`，减少标准字符串解析开销。

### P0-7：坐标输出避免每个数值创建 String

旧实现：

```rust
format!("{value:.6}")
```

每个坐标都会分配 String。

新实现：先保留 6 位小数精度，再用 `ryu::Buffer` 直接写入 `BufWriter`，避免坐标级 heap allocation。

### P0-8：降低 UI 进度事件频率

后端进度事件由约 120 ms 调整为 250 ms，减少 Tauri IPC 与 React 状态刷新对主处理线程的干扰。

### P0-9：调整默认写缓冲

24 个输出文件同时存在时，旧默认 32 MB/文件理论上可占约 768 MB BufWriter 缓冲。
默认改为 4 MB/文件，总计约 96 MB，减少内存占用与缓存污染。
读缓冲仍保留 16 MB。

## 未启用的优化

### 多输入文件并行

当前没有默认并行多个输入 CLI。原因是一个输入文件本身就会同时写 24 个输出文件；并行多个输入会进一步放大磁盘写竞争。
建议先在目标 SSD 上测试 v4；只有当 CPU 明显高而 SSD 利用率仍低时再启用文件级并行。

### GPU

当前热点仍是 ASCII 解析、裁剪、格式化和 24 路文件写入，不建议优先做 GPU。GPU 会引入主机/显存搬运和结果重排，对这种文本流式工作负载收益通常低于 CPU/IO 优化。

## 建议测试方式

固定同一份 CLI、同一份参数 Excel、同一输出 SSD，分别记录：

- 总耗时
- CPU 占用
- SSD 活动时间/写入速度
- 输出总大小
- 24 个输出的几何数量

至少连续测试 3 次，取中位数。

## 后续 P1

如果 v4 后 CPU 仍是瓶颈：

1. 将 `read_line(String)` 改为 `fill_buf()` + byte scanner，完全按 ASCII 字节解析。
2. 对 POLYLINE 引入可复用裁剪 scratch，减少 `Vec<Vec<Point>>` 分配。
3. 建立 reader -> parser/clip worker -> ordered writer 有界流水线。
4. 仅在 SSD 仍有明显余量时增加 2~4 个文件级 worker。

## v10：重叠区域预计算

- 单个 CLI 开始时预计算 `[Region; 24]`。
- POLYLINE、HATCH/HATCHES、候选索引、DIMENSION 共用同一份有效区域。
- 不再在每个实体上重复调用 `region_with_overlap`。
- `overlap=0` 直接复用静态 REGIONS。
- 增加等价性单元测试，确保预计算不改变区域边界。

## v13 极限性能补充

- split 主输入改为 memmap2 + memchr 零拷贝扫描。
- POLYLINE/HATCH/HATCHES 改为字节切片直接解析。
- 24 路输出采用可复用实体级 Vec<u8> 聚合后单次 write_all。
- HATCHES 并行阈值/块大小按线程数自适应。
- release 启用 LTO + codegen-units=1。
- 详细说明见 `PERFORMANCE_V13_MAX.md`。
