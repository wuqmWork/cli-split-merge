// release 构建声明为 Windows GUI 子系统，避免启动时弹出控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    cli_split_merge_lib::run();
}
