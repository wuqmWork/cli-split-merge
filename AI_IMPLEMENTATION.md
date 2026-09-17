# CLI Split & Merge：AI 实现说明文档

## 1. 项目目标

在现有 UI 基础上完成 Windows 桌面程序：

- 多个 ASCII CLI 文件分割为 24 个振镜文件
- 多个 CLI 文件按层合并为 1 个 CLI 文件
- 参数来自 Excel
- 参数需要持久记忆
- 前后端分离
- Rust 负责全部核心处理

固定技术栈：

- Tauri 2
- React + TypeScript + Tailwind CSS
- Lucide React
- Rust
- 中文 UI

不要改变当前 UI 信息架构；不要把振镜映射放进 UI。

---

## 2. 已确定 UI

左侧只保留：

- CLI 分割
- CLI 合并

右上角齿轮打开设置。

设置使用 Tabs：

- 常规
- 外观
- 性能
- 日志与缓存
- 关于

主页面不显示运行日志。

### CLI 分割

1. 多选 CLI 文件
2. 参数 Excel
3. 输出文件夹
4. 开始分割
5. 任务进度

参数与输出文件夹并排显示。

参数区成功加载后显示：

```text
已记忆当前参数    加载于 2026-09-15 23:30
```

“加载于”使用灰色字体。

### CLI 合并

1. 多选 CLI 文件
2. 一个完整输出路径框
3. 开始合并
4. 任务进度

选择输出文件夹后自动生成：

```text
D:\output\merged.cli
```

用户直接在同一个路径框内修改 `merged` 即可，不单独增加文件名输入框。

合并只有一种规则：按层合并。

---

## 3. 用户提供的真实参数 Excel

真实参数工作簿包含：

```text
说明
组间_本组到邻组
组间_邻组到本组
组内_3到012
组内_012到3
C3到各振镜
各振镜到C3
拟合明细
```

CLI 分割运行时核心读取：

```text
C3到各振镜
```

不要在每次分割时重新由组内/组间表做链式计算；优先直接使用已经汇总好的 C3 → 24 振镜变换。

### 3.1 必需列

解析器按列名读取，不依赖 Excel 列号：

```text
类别
变换名称
源坐标系
目标坐标系
点数
a
b
tx
ty
旋转角deg
m11 m12 m13
m21 m22 m23
m31 m32 m33
```

`RMS误差`、`最大绝对误差` 可以存在且允许为空，当前运行不依赖它们。

### 3.2 变换模型

Excel 说明页定义：

```text
x_target = a*x_source - b*y_source + tx
y_target = b*x_source + a*y_source + ty
```

Rust：

```rust
fn apply(t: &MirrorTransform, x: f64, y: f64) -> (f64, f64) {
    (
        t.a * x - t.b * y + t.tx,
        t.b * x + t.a * y + t.ty,
    )
}
```

### 3.3 24 振镜校验

必须恰好存在：

```text
A0 A1 A2 A3
B0 B1 B2 B3
C0 C1 C2 C3
D0 D1 D2 D3
E0 E1 E2 E3
F0 F1 F2 F3
```

并校验：

- `源坐标系 == C3`
- 目标不重复
- 数值为有限值
- `a²+b²` 接近 1
- 24 条全部存在

新 Excel 任何校验失败时：

- 返回错误
- 不覆盖旧参数快照
- UI 保留原“已记忆当前参数”

当前工程已加入 Rust Excel 解析器：

```text
src-tauri/src/parameter.rs
```

---

## 4. 参数快照：必须由 Rust 持久化

不能只把 Excel 路径存在浏览器 localStorage。

成功加载 Excel 后，Rust 将完整解析结果保存为：

```text
<AppData>/parameter_snapshot.json
```

数据结构：

```rust
pub struct ParameterSnapshotDto {
    pub source_path: String,
    pub source_name: String,
    pub loaded_at: String,
    pub schema_version: u32,
    pub sheet_name: String,
    pub transform_count: usize,
    pub transforms: Vec<MirrorTransform>,
}
```

`MirrorTransform` 保存：

```text
category
transformName
source
target
pointCount
a
b
tx
ty
rotationDeg
3×3 matrix
```

### 4.1 生命周期

首次选 Excel：

```text
选择 Excel
→ Rust/calamine 解析
→ 完整校验
→ 生成 ParameterSnapshotDto
→ 原子写入 parameter_snapshot.json
→ 返回前端
```

程序再次启动：

```text
load_saved_parameter_snapshot
→ 读取 JSON
→ 校验 schemaVersion
→ 校验 24 振镜
→ 直接恢复
```

不重新选择 Excel时，不重新解析 Excel，不改变参数。

当前工程已接入 Tauri Commands：

```rust
load_parameter_excel
load_saved_parameter_snapshot
```

前端：

```text
src/backend.ts
```

---

## 5. 固定振镜→物理文件夹映射

固定在 Rust 后端：

```text
A0 -> 22   A1 -> 23   A2 -> 4    A3 -> 1
B0 -> 3    B1 -> 12   B2 -> 7    B3 -> 14
C0 -> 15   C1 -> 11   C2 -> 10   C3 -> 6
D0 -> 19   D1 -> 2    D2 -> 21   D3 -> 24
E0 -> 16   E1 -> 9    E2 -> 13   E3 -> 8
F0 -> 20   F1 -> 5    F2 -> 17   F3 -> 18
```

禁止让普通用户在 UI 修改。

分割输出示例：

```text
22/001_model_A0.cli
23/001_model_A1.cli
...
6/001_model_C3.cli
...
18/001_model_F3.cli
```

默认命名模板：

```text
{index:03}_{source_stem}_{mirror}.cli
```

命名规则只能从“设置 → 常规”调整。

---

## 6. CLI 分割引擎

建议模块：

```text
src-tauri/src/
├─ cli/
│  ├─ parser.rs
│  ├─ writer.rs
│  ├─ geometry.rs
│  └─ layer.rs
├─ split/
│  ├─ engine.rs
│  ├─ region.rs
│  └─ transform.rs
├─ parameter.rs
└─ config/
   └─ mirror_map.rs
```

### 6.1 单文件处理顺序

```text
读取 ASCII CLI
→ 解析 Header
→ 流式读取 Layer
→ 解析 POLYLINE / HATCHES
→ 判断几何属于哪些振镜工作区
→ 必要时裁剪到工作区
→ C3 全局坐标转对应振镜局部坐标
→ 写入对应物理文件夹
```

不要把整份 CLI 复制 24 份到内存。

### 6.2 输出策略

每个逻辑振镜维护独立缓冲 writer：

```rust
HashMap<MirrorId, BufWriter<File>>
```

或采用批次缓冲，避免逐点小写入。

输入文件非常大时：

- `BufReader`
- `memchr` 扫描 ASCII 换行/分隔符
- `lexical-core` 解析数字
- 避免 `String::from_utf8_lossy` 大量临时分配
- 优先借用字节切片

---

## 7. CLI 合并引擎

只有一种：按层合并。

不能直接把文件字节首尾拼起来。

推荐：

```text
解析所有输入 Header
→ 建立 layer key（层号/层高）
→ 同层收集各文件几何
→ 按用户输入文件顺序追加同层几何
→ 输出一个 Header
→ 一个 $$GEOMETRYSTART
→ 所有 Layer
→ 一个 $$GEOMETRYEND
```

默认输出：

```text
merged.cli
```

如果路径框最后没有 `.cli`，执行前自动追加。

---

## 8. Tauri API

已实现：

```rust
#[tauri::command]
fn load_parameter_excel(...)

#[tauri::command]
fn load_saved_parameter_snapshot(...)
```

后续增加：

```rust
#[tauri::command]
async fn split_cli(request: SplitRequest) -> Result<TaskId, String>;

#[tauri::command]
async fn merge_cli(request: MergeRequest) -> Result<TaskId, String>;

#[tauri::command]
async fn cancel_task(task_id: String) -> Result<(), String>;
```

进度使用事件：

```text
cli-task-progress
cli-task-finished
cli-task-error
```

事件：

```json
{
  "taskId": "...",
  "percent": 72,
  "completed": 3,
  "total": 4,
  "currentFile": "model_03.cli",
  "elapsedMs": 21000
}
```

不要让 React 用 100ms 定时轮询 Rust 状态。

---

## 9. 设置

### 常规

- 分割输出命名模板
- 记住上次输出目录
- 同名文件策略

### 外观

- 跟随系统
- 浅色
- 深色
- Logo / 启动画面等图片相关内容

### 性能

- 并行开关
- worker 数，0=自动
- 读 buffer
- 写 buffer
- 后续批处理大小

### 日志与缓存

- 日志级别
- 日志目录
- 日志保留天数
- 缓存目录
- 当前缓存大小
- 清理缓存
- 退出自动清理

安全要求：缓存清理只能删除应用自己创建的受控缓存目录，不能让任意用户输入路径后递归删除。

### 关于

- CLI Split & Merge
- 版本号
- Tauri 2
- React + TypeScript + Tailwind CSS
- Lucide React
- Rust

版本号读取 Cargo/Tauri 配置，不在 React 长期硬编码。

---

## 10. 开发顺序

编码 AI 按以下顺序：

1. 保持当前 UI 不变。
2. 参数 Excel 真实解析与快照恢复（工程已完成基础版本）。
3. 为参数模块补单元测试。
4. CLI Header/Layer/POLYLINE/HATCHES parser。
5. CLI writer。
6. 按层合并。
7. 单振镜区域裁剪和坐标变换。
8. 24 振镜分割。
9. 固定 1~24 物理目录分发。
10. 多 CLI 连续批处理。
11. 真实任务进度事件。
12. 取消任务。
13. 性能优化。
14. 日志与缓存。
15. 发布打包。

---

## 11. 验收标准

### 参数

- 能直接加载用户提供格式的 Excel。
- `C3到各振镜` 必须读取到 24 条。
- 新参数校验失败不覆盖旧快照。
- 关闭软件后重新启动，不选 Excel 也能继续使用快照。
- 原 Excel 被移动后，快照仍可使用。

### UI

- 中文。
- 图标全部 Lucide React。
- 左侧只有“CLI 分割 / CLI 合并”。
- 设置右上角进入。
- 主界面无日志。
- 参数加载状态仅显示“已记忆当前参数 + 灰色加载时间”。

### 合并

- 多文件按层合并。
- 输出合法 CLI。
- 默认 `merged.cli`。
- 同一完整输出路径框直接改文件名。

### 分割

- 24 振镜全部输出。
- 物理目录映射正确。
- 坐标变换方向必须为 `C3 -> 振镜局部`。
- 多输入文件按批次编号命名。

### 性能

- 大 ASCII CLI 流式处理。
- 不创建 24 份输入副本。
- 使用缓冲 I/O。
- UI 不被阻塞。

---

## CLI 分割已实现（2026-09-15）

分割核心已经落到 `src-tauri/src/split.rs`，不是 UI 模拟。

请后续 AI 不要重新设计分割方向：当前定义为 **输入 CLI = 全局 C3 → 按 C3 区域裁剪 → 使用 Excel `C3到各振镜` 转换到各振镜局部坐标 → 按物理编号 1~24 文件夹输出**。

固定物理目录映射：

`A0→22, A1→23, A2→4, A3→1, B0→3, B1→12, B2→7, B3→14, C0→15, C1→11, C2→10, C3→6, D0→19, D1→2, D2→21, D3→24, E0→16, E1→9, E2→13, E3→8, F0→20, F1→5, F2→17, F3→18`。

默认命名：`{index:03}_{source_stem}_{mirror}.cli`。

前端通过 `split-progress` 监听真实进度。CLI 合并已通过 `merge-progress` 接入真实后端。

## CLI 合并（已实现）

CLI 合并已经接入前端与 Rust 后端，规则固定为按层合并。后端模块为 `src-tauri/src/merge.rs`，Tauri command 为 `merge_cli_files`，进度事件为 `merge-progress`。采用两遍索引算法：第一遍只扫描 `$$LAYER/` 和字节偏移，第二遍按层高顺序通过 seek + 原始字节复制写出，避免重新解析几何坐标。详细说明见 `MERGE_IMPLEMENTATION.md`。


## 重叠区域（v8）

CLI 分割新增可配置重叠区域，默认 2.0 mm。该数值表示相邻区域最终公共重叠宽度；内部共享边界两侧各扩大一半，整体最外边界保持不变。UI 与 Rust 后端均校验 0~20 mm，并在本地记忆最近一次值。详见 `OVERLAP_IMPLEMENTATION.md`。

> **坐标系要求：** CLI 合并只适用于处于同一坐标系的输入文件。不要直接把 A0～F3 等不同振镜局部坐标系文件混合合并；如需合并，应先统一到同一坐标系。

## v13 极限性能补充

- split 主输入改为 memmap2 + memchr 零拷贝扫描。
- POLYLINE/HATCH/HATCHES 改为字节切片直接解析。
- 24 路输出采用可复用实体级 Vec<u8> 聚合后单次 write_all。
- HATCHES 并行阈值/块大小按线程数自适应。
- release 启用 LTO + codegen-units=1。
- 详细说明见 `PERFORMANCE_V13_MAX.md`。
