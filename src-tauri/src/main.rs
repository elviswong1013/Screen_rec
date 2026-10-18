// 发布构建隐藏控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // M11-C P1-5 隐藏自检：只在显式传参时触发（不影响正常 GUI 启动）。
    if std::env::args_os().any(|a| a == "--self-check") {
        std::process::exit(screen_rec_lib::self_check());
    }
    screen_rec_lib::run()
}
