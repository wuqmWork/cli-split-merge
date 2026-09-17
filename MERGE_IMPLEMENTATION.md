# CLI 合并实现说明

## 规则

CLI 合并只采用一种规则：**按层合并**。

用户选择多个 `.cli` 文件后，程序按层高 `$$LAYER/<z>` 聚合：同一个层高只输出一次 `$$LAYER/`，其后的几何内容按照用户选择文件的顺序依次追加。最终只保留一个 CLI 头、一个 `$$GEOMETRYSTART` 和一个 `$$GEOMETRYEND`。

## 输出设置

UI 只有一个完整输出路径输入框，例如：

```text
D:\output\merged.cli
```

选择文件夹后，自动保留当前文件名。默认文件名为 `merged.cli`。用户可以直接在同一个输入框里修改 `.cli` 前面的名称。如果没有输入 `.cli` 扩展名，Rust 后端自动补全。

## Rust 实现

文件：`src-tauri/src/merge.rs`

采用双遍流式算法：

1. 第一遍扫描所有输入 CLI，只记录每个 `$$LAYER/` 的字节偏移，不加载完整几何。
2. 将层高建立为有序索引。
3. 第二遍按层高从小到大输出。
4. 同层内按输入文件选择顺序复制原始 ASCII 字节。
5. 用 `seek + 1 MiB copy buffer` 直接复制层数据，避免重新解析 POLYLINE/HATCH 数字。
6. 输出先写 `.merge.tmp`，全部成功后再替换最终文件，避免异常时留下损坏 CLI。

这种方式的主要性能优势是：合并不需要解析坐标、不需要重新格式化浮点数，几何主体基本是原始字节流复制。

## 兼容校验

- 至少需要两个 CLI。
- 输入必须是 `.cli`。
- 输出文件不能和任意输入文件相同。
- 检查输入文件的 `$$UNITS/` 是否一致，不一致直接终止。
- 必须存在 `$$LAYER/`。

## Tauri API

```text
merge_cli_files(request)
```

请求：

```ts
{
  inputFiles: string[];
  outputFile: string;
  readBufferMb: number;
  writeBufferMb: number;
}
```

进度事件：

```text
merge-progress
```

返回：输入文件数、合并层数、最终输出文件、耗时。

> **坐标系要求：** CLI 合并只适用于处于同一坐标系的输入文件。不要直接把 A0～F3 等不同振镜局部坐标系文件混合合并；如需合并，应先统一到同一坐标系。
