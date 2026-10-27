//! 跨平台路径。
//!
//! 目标平台：**macOS / Windows / Linux 三端都要能用**（Windows 是当前需求，
//! Linux 是后续计划）。所以任何平台相关的路径约定都必须集中在这里，而不是
//! 散落在各处写 `~/.config`。
//!
//! ## 已经踩过的坑
//!
//! 主目录**不能只看 `HOME`**——那是 Unix 的约定，原生 Windows 进程里通常只有
//! `USERPROFILE`。Git Bash 里 `HOME` 恰好有值，所以在 mac 上开发时完全看不出
//! 问题，一到 Windows 原生运行就炸。

use std::path::{Path, PathBuf};

/// 取环境变量的内部表示，便于测试注入。
type Getter<'a> = &'a dyn Fn(&str) -> Option<std::ffi::OsString>;

fn non_empty(get: Getter<'_>, key: &str) -> Option<PathBuf> {
    get(key).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
}

/// 用户主目录。依次尝试 `HOME` → `USERPROFILE` → `HOMEDRIVE`+`HOMEPATH`。
pub fn home_dir() -> Option<PathBuf> {
    home_from(&|k| std::env::var_os(k))
}

/// `home_dir` 的纯函数版本。
///
/// **抽出来是为了可测**：原来的测试直接改进程环境变量，而测试是并发跑的——
/// 一个测试把 HOME 清掉，另一个正在读主目录的测试就偶发失败。注入一个取值函数
/// 就完全没有这个竞态了，也不用加锁。
pub fn home_from(get: Getter<'_>) -> Option<PathBuf> {
    if let Some(h) = non_empty(get, "HOME") {
        return Some(h);
    }
    if let Some(p) = non_empty(get, "USERPROFILE") {
        return Some(p);
    }
    // Windows 9x 时代的遗留写法，现代系统基本用不到，但代价只有三行
    match (non_empty(get, "HOMEDRIVE"), non_empty(get, "HOMEPATH")) {
        (Some(d), Some(p)) => {
            // Windows 的 HOMEPATH 形如 `\Users\me`。
            //
            // ⚠️ **不能把前导分隔符丢掉再 join**：`Path::new("C:").join("Users\\me")`
            // 得到的是 `C:Users\me` —— 那是 **drive-relative 路径**（"C 盘当前目录下
            // 的 Users\me"），不是 `C:\Users\me`。在 Unix 上 `join` 会补 `/`，所以
            // 这个错只在 Windows 上现形（CI 的 Windows job 第一次跑就抓到了）。
            // 两头都规范化：缺分隔符就补一个。
            let sep = |c: char| c == '/' || c == '\\';
            let d_s = d.to_string_lossy();
            let p_s = p.to_string_lossy();
            let joined = match (d_s.ends_with(sep), p_s.starts_with(sep)) {
                // `C:` + `\Users\me` / `C:\` + `Users\me` → 补一个分隔符
                (false, false) => format!("{d_s}{}{p_s}", std::path::MAIN_SEPARATOR),
                // `C:\` + `\Users\me` → 只留一个
                (true, true) => format!("{d_s}{}", &p_s[1..]),
                _ => format!("{d_s}{p_s}"),
            };
            Some(PathBuf::from(joined))
        }
        _ => None,
    }
}

/// 本工具的配置目录。
///
/// - Windows：`%APPDATA%\gws`（这是 Windows 上放用户配置的地方）
/// - 其它：`$XDG_CONFIG_HOME/gws`，缺省 `~/.config/gws`
///
/// 不用 macOS 的 `~/Library/Application Support`：这个工具主要在终端里用，
/// `~/.config` 更符合使用者（开发者）的预期，也让三端的说明文档能写成一句。
pub fn config_dir() -> Option<PathBuf> {
    config_from(&|k| std::env::var_os(k), home_dir())
}

/// `config_dir` 的纯函数版本，同样是为了可测。
pub fn config_from(get: Getter<'_>, home: Option<PathBuf>) -> Option<PathBuf> {
    if cfg!(windows) {
        if let Some(appdata) = non_empty(get, "APPDATA") {
            return Some(appdata.join("gws"));
        }
        return home.map(|h| h.join(".config").join("gws"));
    }

    if let Some(xdg) = non_empty(get, "XDG_CONFIG_HOME") {
        return Some(xdg.join("gws"));
    }
    home.map(|h| h.join(".config").join("gws"))
}

/// 全局评审人配置文件的路径（设计方案 §5.3 三层优先级的第三层）。
pub fn reviewers_config() -> Option<PathBuf> {
    config_dir().map(|d| d.join("reviewers.json"))
}

/// 确保配置目录存在，返回其路径。
pub fn ensure_config_dir() -> crate::Result<PathBuf> {
    let dir = config_dir()
        .ok_or_else(|| crate::Error::other("读不到用户主目录，无法定位配置目录"))?;
    std::fs::create_dir_all(&dir).map_err(|e| crate::Error::io(dir.clone(), e))?;
    Ok(dir)
}

/// 去掉 Windows 的逐字（verbatim）路径前缀 `\\?\`。
///
/// # 为什么必须去掉
///
/// `std::fs::canonicalize` 在 Windows 上返回的是**逐字路径**：`\\?\D:\gws\hub`。
/// 这种路径**喂给 git 会直接失败**。实测反馈：Windows 上添加仓库时
///
/// ```text
/// git 命令失败: git clone https://…/demo-admin-front.git  \\?\D:\gws\hub\my-hub\repos\demo-admin-front
/// ```
///
/// 界面上显示出来也是一串 `\\?\`，看着像路径坏了。
///
/// 两条规则（UNC 那条要注意，不能简单砍 4 个字符）：
///
/// - `\\?\D:\x`        → `D:\x`
/// - `\\?\UNC\srv\sh` → `\\srv\sh`
///
/// 非 Windows 形态的路径**原样返回** —— 这个函数在 mac/Linux 上调用是安全的，
/// 所以在哪台机器上都能单测。
pub fn strip_verbatim_prefix(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix("\\\\?\\UNC\\") {
        return PathBuf::from(format!("\\\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix("\\\\?\\") {
        return PathBuf::from(rest);
    }
    p.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    /// 用一个假的取值函数跑解析逻辑。**完全不碰进程环境变量**——
    /// 那正是原来那个偶发失败的根源（并发测试互相清对方的变量）。
    fn getter(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: Vec<(String, OsString)> =
            pairs.iter().map(|(k, v)| ((*k).to_string(), OsString::from(v))).collect();
        move |k: &str| map.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone())
    }

    #[test]
    fn home_prefers_home_over_userprofile() {
        let g = getter(&[("HOME", "/tmp/h"), ("USERPROFILE", "/tmp/p")]);
        assert_eq!(home_from(&g).unwrap(), PathBuf::from("/tmp/h"));
    }

    #[test]
    fn home_falls_back_to_userprofile() {
        // HOME 缺失/为空时退到 USERPROFILE —— 这正是 Windows 原生的情形
        let g = getter(&[("USERPROFILE", "/tmp/p")]);
        assert_eq!(home_from(&g).unwrap(), PathBuf::from("/tmp/p"));

        let g = getter(&[("HOME", ""), ("USERPROFILE", "/tmp/p")]);
        assert_eq!(home_from(&g).unwrap(), PathBuf::from("/tmp/p"), "空 HOME 视同未设置");
    }

    #[test]
    fn home_falls_back_to_drive_plus_path() {
        // 这一段只在 Windows 上有意义：C: + \Users\me → C:\Users\me
        let g = getter(&[("HOMEDRIVE", "C:"), ("HOMEPATH", "\\Users\\me")]);
        let h = home_from(&g).expect("drive+path 应能拼出主目录");
        if cfg!(windows) {
            assert_eq!(h, PathBuf::from("C:\\Users\\me"));
        } else {
            // 非 Windows 上不会走到这条路径，只确认逻辑没 panic、尾段拼对了
            assert!(h.to_string_lossy().contains("Users"), "{h:?}");
        }
    }

    #[test]
    fn home_is_none_when_nothing_is_set() {
        let g = getter(&[]);
        assert!(home_from(&g).is_none());
    }

    #[test]
    fn config_dir_is_absolute_and_named_gws() {
        // 用真实环境跑一次，确认在当前平台上能解析出绝对路径
        let dir = config_dir().expect("测试环境应有配置目录");
        assert!(dir.is_absolute(), "{dir:?}");
        assert_eq!(dir.file_name().unwrap(), "gws");
    }

    #[test]
    fn config_uses_appdata_on_windows_and_xdg_elsewhere() {
        let home = Some(PathBuf::from("/home/me"));
        let g = getter(&[("APPDATA", "C:\\Users\\me\\AppData\\Roaming"), ("XDG_CONFIG_HOME", "/xdg")]);
        let dir = config_from(&g, home.clone()).unwrap();

        if cfg!(windows) {
            assert_eq!(dir, PathBuf::from("C:\\Users\\me\\AppData\\Roaming").join("gws"));
        } else {
            assert_eq!(dir, PathBuf::from("/xdg").join("gws"));
        }

        // XDG 没设时退回 ~/.config/gws
        let g = getter(&[]);
        assert_eq!(config_from(&g, home).unwrap(), PathBuf::from("/home/me/.config/gws"));
    }

    #[test]
    fn reviewers_config_lives_under_the_config_dir() {
        let p = reviewers_config().expect("测试环境应有配置路径");
        assert_eq!(p.file_name().unwrap(), "reviewers.json");
        assert_eq!(p.parent().unwrap(), config_dir().unwrap());
    }

    #[test]
    fn strips_the_windows_verbatim_prefix() {
        // 驱动器盘符形态
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\D:\gws\hub")),
            PathBuf::from(r"D:\gws\hub")
        );
        // UNC 形态：砍掉前缀后要补回 `\\`，不然就成了 `srv\share`
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\UNC\srv\share\x")),
            PathBuf::from(r"\\srv\share\x")
        );
        // 正常路径（mac / 干净路径）原样返回
        assert_eq!(
            strip_verbatim_prefix(Path::new("/Users/me/hub")),
            PathBuf::from("/Users/me/hub")
        );
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"D:\gws\hub")),
            PathBuf::from(r"D:\gws\hub")
        );
    }
}
