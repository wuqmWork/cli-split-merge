# 设置功能实现说明（v7）

本版本把此前审查中标为“即将上线”的设置改为真实功能，不再使用禁用占位控件。

## 1. 性能

### 并行处理
- CLI 分割：多个输入 CLI 可使用 Rayon 线程池并行处理。
- CLI 合并：第一遍层索引扫描可并行，第二遍仍按层有序写入，保证合并结果顺序稳定。
- `线程数 = 0`：自动读取系统可用逻辑 CPU 数。
- 自定义线程数：限制为 `1..输入文件数`。
- 并行只对多个输入文件生效；单个 CLI 仍采用已有的高性能流式解析。
- SSD 已接近满载时，建议线程数从 2～4 开始测试，避免磁盘争用。

前端请求新增：
```ts
parallel: boolean
workerCount: number
```
Rust 请求新增：
```rust
parallel: Option<bool>
worker_count: Option<usize>
```

## 2. 日志

Rust 后端新增 `runtime.rs`：
- 支持 `error / warn / info / debug` 日志级别。
- 日志目录：Tauri `app_data_dir/logs`。
- 日志按天写入 `app-YYYY-MM-DD.log`。
- 参数加载、分割、合并、设置更新、缓存清理等关键动作都会写入日志。
- 日志写入有全局互斥锁，避免并行任务行内容交错。
- 修改“日志保留天数”后自动清理过期日志。
- 设置页显示日志目录与当前占用空间，可直接“清理日志”。

## 3. 缓存

缓存目录：Tauri `app_data_dir/cache`。

当前缓存真实用于保存最近任务摘要：
- `last_split.json`
- `last_merge.json`

设置页：
- 显示缓存目录与占用空间。
- “清理缓存”会真实删除缓存目录内容并重新创建空目录。
- “退出时清理缓存”启用后，Tauri 收到退出事件时自动清理。

参数快照 `parameter_snapshot.json` 属于持久数据，不属于缓存，因此清理缓存不会删除分割参数。

## 4. 深色主题

主题已实现三种模式：
- 跟随系统
- 浅色
- 深色

实现方式：
- Tailwind `darkMode: 'class'`
- 根节点根据设置动态添加/删除 `dark` class。
- “跟随系统”监听 `prefers-color-scheme` 的实时变化。
- 主页面、侧栏、卡片、输入框、设置窗口、文件卡、进度区域均加入 dark 样式。

## 5. 图片相关设置

“程序 Logo”和“启动画面”已从占位按钮改为真实选择：
- 通过 Tauri 文件选择器选择 PNG/JPG/JPEG/WEBP/BMP。
- Rust 将图片复制到 `app_data_dir/assets`，避免源图片移动后失效。
- 程序 Logo 会显示在左侧标题区域与“关于”页。
- 启动画面设置后，程序界面启动时展示约 1.2 秒。
- Tauri asset protocol 仅开放 `$APPDATA/**`，不开放整个文件系统。

## 6. 新增后端命令

```text
apply_runtime_settings
get_storage_info
clear_cache
clear_logs
install_ui_image
```

## 7. 建议性能参数

对于大 ASCII CLI：

```text
并行处理：开启
线程数：2～4 起步
读缓冲：16 MB
写缓冲：4 MB
```

如果 CPU 高但 SSD 未满，可逐步增加线程数；如果 SSD 活动时间接近 100%，继续增加线程通常不会更快。
