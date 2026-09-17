# CLI Split & Merge UI

基于 Tauri 2 + React + TypeScript + Tailwind CSS + Lucide React 的桌面 UI 实现。

## 已实现

- CLI 分割页面
- CLI 合并页面
- 左侧导航仅保留「CLI 分割 / CLI 合并」
- 右上角设置入口
- 设置：常规 / 外观 / 性能 / 日志与缓存 / 关于
- CLI 多文件选择、圆角文件卡片、单文件删除、清空
- 分割参数 Excel 选择与“参数快照”本地记忆
- 分割输出目录
- 合并输出路径直接编辑，默认 `merged.cli`
- 任务进度、当前文件、已处理、运行时间
- 图标统一为 Lucide React

## 运行

```bash
npm install
npm run dev
```

Tauri 桌面运行：

```bash
npm run tauri dev
```

生产构建：

```bash
npm run tauri build
```

> 当前仓库已完成 UI、Excel 参数快照、Rust CLI 分割与按层合并核心实现。


## 参数 Excel

项目已按真实参数工作簿格式接入 Rust 参数解析器。运行时读取 `C3到各振镜`，校验 A0~F3 共 24 个振镜，并把完整解析结果持久化为参数快照。详见 [PARAMETER_FORMAT.md](./PARAMETER_FORMAT.md) 和 [AI_IMPLEMENTATION.md](./AI_IMPLEMENTATION.md)。

参数加载相关 Tauri Commands：

- `load_parameter_excel`
- `load_saved_parameter_snapshot`

只有用户主动选择新的 Excel 且新文件完整校验成功后，当前参数快照才会被替换。

## 当前实现进度

- [x] CLI 分割 UI
- [x] Excel `C3到各振镜` 参数解析与快照记忆
- [x] Rust ASCII CLI 流式分割
- [x] POLYLINE / HATCH / HATCHES 裁剪
- [x] C3 → 24 振镜局部坐标转换
- [x] 1～24 物理目录分发
- [x] 分割真实进度 / 当前文件 / 运行时间
- [x] CLI 合并（按层合并）

详见 `SPLIT_IMPLEMENTATION.md`。

## CLI 合并

CLI 合并已实现。规则固定为按层合并，使用两遍层偏移索引 + 原始字节流复制，详见 `MERGE_IMPLEMENTATION.md`。

## v7 设置功能实装

v7 已把设置页中的并行处理、线程数、日志、缓存清理、退出自动清理、深色主题、程序 Logo 与启动画面改为真实功能。详见 `SETTINGS_IMPLEMENTATION.md`。


## 重叠区域（v8）

CLI 分割新增可配置重叠区域，默认 2.0 mm。该数值表示相邻区域最终公共重叠宽度；内部共享边界两侧各扩大一半，整体最外边界保持不变。UI 与 Rust 后端均校验 0~20 mm，并在本地记忆最近一次值。详见 `OVERLAP_IMPLEMENTATION.md`。

> **坐标系要求：** CLI 合并只适用于处于同一坐标系的输入文件。不要直接把 A0～F3 等不同振镜局部坐标系文件混合合并；如需合并，应先统一到同一坐标系。

### v10 性能优化

分割器会在每个 CLI 开始处理时一次性预计算 24 个实际重叠区域，并在候选筛选、几何裁剪和 DIMENSION 重写中复用，减少大 ASCII CLI 上的重复区域计算。详见 `REGION_CACHE_OPTIMIZATION.md`。

## v11 区域引用优化

CLI 分割在重叠量为 `0 mm` 时直接引用静态 `REGIONS`，不会复制 24 个区域；重叠量大于 0 时则只在每个 CLI 开始时预计算一次有效区域。详见 `REGION_ZERO_COPY_OPTIMIZATION.md`。

## v13 极限性能优化

分割热路径新增 mmap + memchr 零拷贝行扫描、几何区纯字节解析、单实体聚合输出、自适应 HATCHES 并行以及 release LTO。详见 `PERFORMANCE_V13_MAX.md`。
