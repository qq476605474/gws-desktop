// 发布版不要弹控制台窗口（Windows）。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // **更新助手**：`<exe> --apply-update <安装包>` —— 不启动 GUI，只做
    // 「等父进程退出 → 静默安装 → 拉起新版」然后退出。
    //
    // 为什么要借自己的壳：Windows 上装更新时应用必须**已经退出**（NSIS 会以
    // 退出码 2 拒绝安装），而"退出之后还要有人把新版拉起来" —— 用 `cmd` 拼命令行
    // 会掉进引号/中文路径的坑（实测反馈：弹出「找不到 '\\' 文件」）。
    // 用自己当助手，全程 `Command` 参数分开传，不经过 shell。
    let mut args = std::env::args_os().skip(1);
    if let Some(flag) = args.next() {
        if flag == gws_core::update::APPLY_FLAG {
            // `<exe> --apply-update <安装包> <装完要拉起的程序>`
            let pkg = args.next();
            let app = args.next();
            if let (Some(pkg), Some(app)) = (pkg, app) {
                let _ = gws_core::update::run_update_helper(
                    std::path::Path::new(&pkg),
                    std::path::Path::new(&app),
                );
            }
            return;
        }
    }

    gws_desktop_lib::run();
}
