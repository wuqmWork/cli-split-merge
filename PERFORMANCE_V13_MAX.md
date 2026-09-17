# v13 极限性能优化

本版建立在 v12 的单文件 HATCHES 并行和 ryu 数值输出优化之上，目标是继续降低单个大型 ASCII CLI 分割时的单核解析开销。

## 1. mmap 零拷贝输入

- split 不再使用 `BufReader::read_until()` 把每一行复制到 `Vec<u8>`。
- 使用 `memmap2` 将 CLI 只读映射到虚拟内存。
- 使用 `memchr` 直接定位换行符，每一行都是映射文件上的 `&[u8]` 切片。
- 对超长 `$$HATCHES` 行尤其有利，减少内存复制与扩容。

## 2. 几何区纯字节解析

- `POLYLINE / HATCH / HATCHES / LAYER / GEOMETRYEND` 全部直接在 `&[u8]` 上识别。
- 数值字段使用 `lexical-core` 直接从字节切片解析。
- 几何热路径不再执行 `String::from_utf8_lossy()`。
- 未知实体和 GBK 注释以原始字节透传，不再发生 lossy 转码。
- 只有很小的 CLI 头部继续转为 String，用于 LABEL / DIMENSION 重写。

## 3. 单实体输出聚合

v12 虽然数值格式化不再分配 String，但一个 HATCHES 仍会产生大量：

`write comma -> write number -> write comma -> write number ...`

v13 为 24 个 OutputFile 各维护一个可复用 `line_buffer: Vec<u8>`：

1. 在内存缓冲中拼完整的 POLYLINE/HATCHES ASCII 行；
2. `ryu` 直接 append 浮点数字节；
3. `itoa` 直接 append id/count/kind；
4. 最后对 BufWriter 只做一次 `write_all()`。

这显著减少函数调用和 BufWriter 热路径开销。

## 4. 自适应 HATCHES 并行

v12 固定 `4096` 条线段才启用并行，若文件由大量 1000~3000 段 HATCHES 构成，CPU 仍可能偏低。

v13 根据线程数自适应阈值：

- 阈值约 `max(512, threads × 256)`；
- chunk 大小根据 `count / threads / 4` 动态计算，并限制在 128~2048；
- 线程池仍然每个 CLI 只创建一次。

## 5. release 编译配置

新增：

```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

正式速度测试必须使用 `npm run tauri build` 的 release 包，不要用 debug 开发模式评估性能。

## 6. 保留的优化

- 24 区候选快速索引；
- overlap 区域每文件只算一次；
- overlap=0 直接借用静态 REGIONS；
- 大 HATCHES 多线程裁剪/变换；
- ryu 浮点输出；
- 24 路 BufWriter；
- 多输入文件并行；
- 250ms UI 进度节流。

## 建议测试设置

单个大型 ASCII CLI：

- 并行处理：开
- 线程数：0（自动）
- 写缓冲：4~8 MB
- 重叠：按实际业务值

v13 的 split 输入由 mmap 完成，因此“读缓冲大小”对 split 主路径不再是性能参数；该设置保留用于兼容/其它模块。

## 仍可能存在的最终瓶颈

经过 v13 后，如果 CPU 仍明显低于 50%，需要用实际 CLI 做 profiler（推荐 Windows Performance Recorder / cargo-flamegraph）确认：

- 文件是否主要由大量很小的 POLYLINE 构成；
- 单个 HATCHES 的平均 segment 数；
- 24 路输出格式化是否成为主热点；
- 防病毒/文件系统过滤驱动是否拦截大量输出文件。

在没有真实 profiling 数据前，再继续增加线程或复杂流水线可能反而降低速度。
