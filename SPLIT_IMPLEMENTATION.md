# CLI 分割实现说明（Rust）

## 当前状态

CLI 分割已经从 UI 演示逻辑改为真实的 Tauri → Rust 调用。

入口：

- 前端：`src/backend.ts` → `splitCliFiles()`
- Tauri command：`src-tauri/src/lib.rs` → `split_cli_files`
- 核心实现：`src-tauri/src/split.rs`

CLI 合并已在 `merge.rs` 中实现；本文仅说明分割模块。

## 分割流程

1. 用户多选一个或多个 `.cli` 文件。
2. 使用已经记忆的 Excel 参数快照；必须包含 `C3到各振镜` 的 A0～F3 共 24 条变换。
3. 用户选择输出根目录。
4. Rust 顺序读取每个 ASCII CLI，只扫描源文件一次。
5. 读取 `$$UNITS/`，内部统一换算到 mm。
6. `POLYLINE` / `HATCHES` 在全局 C3 坐标系中与 24 个振镜矩形区域裁剪。
7. 对裁剪结果使用 Excel 中的 `C3 → 振镜局部` 变换：

```text
x_local = a*x_c3 - b*y_c3 + tx
y_local = b*x_c3 + a*y_c3 + ty
```

8. 输出时恢复原 CLI 单位。
9. 输出 24 个 CLI，并发送 `split-progress` 事件更新 UI 进度、当前文件和运行时间。

## 24 振镜裁剪区域

区域坐标固定在全局 C3 坐标系：

```text
A0 [-1430,-1170] × [-140, 140]
A1 [-1430,-1170] × [-420,-140]
A2 [-1170, -910] × [-420,-140]
A3 [-1170, -910] × [-140, 140]

B0 [ -910, -650] × [-140, 140]
B1 [ -910, -650] × [-420,-140]
B2 [ -650, -390] × [-420,-140]
B3 [ -650, -390] × [-140, 140]

C0 [ -390, -130] × [-140, 140]
C1 [ -390, -130] × [-420,-140]
C2 [ -130,  130] × [-420,-140]
C3 [ -130,  130] × [-140, 140]

D0 [  130,  390] × [-140, 140]
D1 [  130,  390] × [-420,-140]
D2 [  390,  650] × [-420,-140]
D3 [  390,  650] × [-140, 140]

E0 [  650,  910] × [-140, 140]
E1 [  650,  910] × [-420,-140]
E2 [  910, 1170] × [-420,-140]
E3 [  910, 1170] × [-140, 140]

F0 [ 1170, 1430] × [-140, 140]
F1 [ 1170, 1430] × [-420,-140]
F2 [ 1430, 1690] × [-420,-140]
F3 [ 1430, 1690] × [-140, 140]
```

## 物理输出文件夹映射

```text
A0→22  A1→23  A2→4   A3→1
B0→3   B1→12  B2→7   B3→14
C0→15  C1→11  C2→10  C3→6
D0→19  D1→2   D2→21  D3→24
E0→16  E1→9   E2→13  E3→8
F0→20  F1→5   F2→17  F3→18
```

输出根目录会自动建立 `1`～`24` 文件夹。

例如输入第 1 个文件 `model.cli`，默认命名规则：

```text
{index:03}_{source_stem}_{mirror}.cli
```

则：

```text
22/001_model_A0.cli
23/001_model_A1.cli
...
6/001_model_C3.cli
...
18/001_model_F3.cli
```

输入第 2 个文件 `test.cli`：

```text
22/002_test_A0.cli
6/002_test_C3.cli
...
```

## CLI 支持内容

当前实现支持：

- `$$HEADERSTART / $$HEADEREND`
- `$$ASCII`
- `$$VERSION/`
- `$$UNITS/`
- `$$DIMENSION/`
- `$$LAYERS/`
- `$$LABEL/`（输出时改为 `源文件名_振镜名`）
- `$$GEOMETRYSTART / $$GEOMETRYEND`
- `$$LAYER/`
- `$$POLYLINE/`
- `$$HATCH/`
- `$$HATCHES/`

`POLYLINE` 使用 Liang–Barsky 线段裁剪。被区域边界切开的曲线会输出成新的开放 POLYLINE。

HATCH 支持两种常见格式：

```text
$$HATCHES/id,count,x1,y1,x2,y2,...
$$HATCHES/id,kind,count,x1,y1,x2,y2,...
```

## 大文件处理

实现没有把完整 CLI 加载进内存：

- `BufReader` 流式读取；默认 16 MB 读缓冲。
- 24 个目标文件使用 `BufWriter`；默认每个 32 MB 写缓冲。
- 缓冲大小由“设置 → 性能”传递给 Rust。
- 每个输入文件只扫描一次。
- 写入 `.cli.part` 临时文件，完成后再替换最终 `.cli`。

后续性能优化可以继续加入：

- 区域快速索引，减少每条几何对 24 区的判断。
- HATCH 批量解析和批量裁剪。
- 多输入文件并行。
- SIMD 数值解析。

## 前端进度事件

Rust 发送：

```text
split-progress
```

载荷：

```ts
{
  percent: number;
  completed: number;
  total: number;
  currentFile: string;
  currentFilePercent: number;
  message: string;
}
```

UI 的“任务进度 / 当前文件 / 运行时间”已经连接真实任务，不再使用模拟进度。

## 关键约束

- 当前输入 CLI 的坐标系固定视为全局 C3。
- Excel 参数必须是 `C3 → A0...F3`。
- 当前重叠量固定为 `0 mm`。
- CLI 合并已实现，详见 `MERGE_IMPLEMENTATION.md`。

## v4 性能优化

已加入空间候选索引、HATCHES 单遍分发、固定变换表、缓存复用、lexical-core 数字解析、ryu 无分配数字输出，以及更合理的 24 路写缓冲。详见 `PERFORMANCE_OPTIMIZATION.md`。


## 重叠区域（v8）

CLI 分割新增可配置重叠区域，默认 2.0 mm。该数值表示相邻区域最终公共重叠宽度；内部共享边界两侧各扩大一半，整体最外边界保持不变。UI 与 Rust 后端均校验 0~20 mm，并在本地记忆最近一次值。详见 `OVERLAP_IMPLEMENTATION.md`。

## v13 极限性能补充

- split 主输入改为 memmap2 + memchr 零拷贝扫描。
- POLYLINE/HATCH/HATCHES 改为字节切片直接解析。
- 24 路输出采用可复用实体级 Vec<u8> 聚合后单次 write_all。
- HATCHES 并行阈值/块大小按线程数自适应。
- release 启用 LTO + codegen-units=1。
- 详细说明见 `PERFORMANCE_V13_MAX.md`。
