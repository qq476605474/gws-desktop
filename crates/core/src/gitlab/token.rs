//! GitLab 访问凭据的解析与存储。
//!
//! # 凭据跟着 hub 走
//!
//! 每个 hub 绑定自己的 GitLab 实例与 token：**地址**写在 hub 的 `gws.json` 里，
//! **token** 存在该 hub 目录下的 `gitlab-token`。同一台机器上可以有多个 hub
//! 指向不同实例 / 不同账号，互不干扰 —— 这就是把凭据从「全局一份」改成
//! 「跟着 hub」的全部理由。
//!
//! # 为什么不用系统钥匙串（这里改过一次，把原因记下来）
//!
//! 最初的实现把 token 存系统钥匙串（macOS Keychain），理由是「比明文安全」。
//! 但实测发现**根本不可用**：macOS 对「不是本应用创建的条目」或「应用签名变了」
//! 的情况会弹框索要**登录钥匙串密码**——`tauri dev` 每次重新编译都会换签名，
//! ACL 永远对不上，于是每次打开设置页都弹框，而且输什么都不对。
//!
//! 改成文件存储，参照 `gh` 的做法（`~/.config/gh/hosts.yml` 也是明文存 token）：
//!
//! - 位置：`<hub>/gitlab-token`
//! - 权限：Unix 上 `0600`（仅属主可读写）
//! - hub 根的 `.gitignore` 里会写上它，避免用户把 hub 纳入 git 后误提交
//!
//! 安全级别确实比钥匙串低一档，但**一个总弹框要密码的方案等于没有方案**——
//! 用户被逼着回去往环境变量里写明文，反而更糟。
//!
//! # 环境变量仍然优先
//!
//! CI / 脚本 / 临时换账号需要显式传参能赢过本机持久值：
//! `GWS_GITLAB_TOKEN` > `GITLAB_TOKEN` > 该 hub 的凭据文件。
//!
//! **但地址不认环境变量** —— 地址必须来自 hub。允许环境变量覆盖地址，
//! 「地址不可改」就形同虚设了。
//!
//! # 状态查询必须是廉价的
//!
//! `source()` / `has_stored()` 只读环境变量与本地文件，**不弹框、不联网**。
//! 原来读钥匙串会在设置页加载时就触发系统授权框，正是「点设置被问密码」的根源。

use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// hub 内的凭据文件名（相对 hub 根）。
pub const HUB_TOKEN_FILE: &str = "gitlab-token";

/// hub 根 `.gitignore` 里必须有的两行。
///
/// 第一条防 token 被提交，第二条防配置原子写的临时文件被提交。
pub const GITIGNORE_LINES: [&str; 2] = ["gitlab-token", "gws.json.tmp"];

/// 某个 hub 的凭据文件路径。
pub fn token_file(hub_root: &Path) -> PathBuf {
    hub_root.join(HUB_TOKEN_FILE)
}

/// 解析该 hub 的 token。优先级：`GWS_GITLAB_TOKEN` > `GITLAB_TOKEN` > hub 凭据文件。
pub fn resolve(hub_root: &Path) -> Result<String> {
    if let Some(t) = from_env() {
        return Ok(t);
    }
    read_file(&token_file(hub_root))?.ok_or_else(|| Error::other(no_token_error(hub_root)))
}

/// 环境变量里的 token（如果有）。
pub fn from_env() -> Option<String> {
    for key in ["GWS_GITLAB_TOKEN", "GITLAB_TOKEN"] {
        if let Ok(v) = std::env::var(key) {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// token 来源说明（供界面展示，不泄露 token 本身）。
pub fn source(hub_root: &Path) -> &'static str {
    if std::env::var("GWS_GITLAB_TOKEN").map(|v| !v.trim().is_empty()).unwrap_or(false) {
        return "环境变量 GWS_GITLAB_TOKEN";
    }
    if std::env::var("GITLAB_TOKEN").map(|v| !v.trim().is_empty()).unwrap_or(false) {
        return "环境变量 GITLAB_TOKEN";
    }
    if read_file(&token_file(hub_root)).ok().flatten().is_some() {
        return "hub 内配置文件";
    }
    "未配置"
}

/// 把 token 写进**这个 hub** 的凭据文件（Unix 上权限 0600）。
pub fn store(hub_root: &Path, token: &str) -> Result<()> {
    let token = token.trim();
    if token.is_empty() {
        return Err(Error::other("token 不能为空"));
    }
    let path = token_file(hub_root);
    write_private(&path, &format!("{token}\n")).map_err(|e| Error::io(path.clone(), e))?;
    Ok(())
}

/// 删除该 hub 保存的 token。本来就没有也算成功（目标已达成）。
pub fn clear(hub_root: &Path) -> Result<()> {
    let path = token_file(hub_root);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(path, e)),
    }
}

/// 该 hub 是否已存有 token。
pub fn has_stored(hub_root: &Path) -> bool {
    read_file(&token_file(hub_root)).ok().flatten().is_some()
}

/// 存储位置（供界面告诉用户去哪儿找）。
pub fn store_path(hub_root: &Path) -> String {
    token_file(hub_root).to_string_lossy().to_string()
}

/// 读凭据文件。
///
/// `Ok(None)` = **确实没配**；`Err` = 文件在、但读不出来（权限不足、不是
/// UTF-8…）。两者必须分开：把「读不到」说成「还没配 token」，用户会去反复重填，
/// 而真正的原因是文件权限或编码，重填也解决不了（写回去的还是同一个坏状态）。
fn read_file(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let t = text.trim();
            Ok(if t.is_empty() { None } else { Some(t.to_string()) })
        }
        // 文件不存在 = 这个 hub 还没配过，是正常状态
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path.to_path_buf(), e)),
    }
}

/// **以 0600 创建**文件再写内容。
///
/// 不能写成「先 `fs::write` 再 `chmod`」：新建文件那一刻的权限是
/// `0666 & ~umask`（常见 `0644`），从创建到 chmod 之间有一个窗口，本机其他用户
/// 能读到 token；进程若在窗口里被杀、或 chmod 因文件系统限制失败（那里只忽略
/// 错误），文件就**长期**停在宽松权限，而 `store` 照样返回成功。
fn write_private(path: &Path, content: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(content.as_bytes())?;
        f.flush()?;
        drop(f);
        // `mode` 只在**创建**时生效；文件本来就存在（比如权限被改坏过）时，
        // 这里再收紧一次。幂等，失败仍然只忽略（网盘之类不支持改权限）。
        restrict_permissions(path);
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, content)
    }
}

/// Unix 上把权限收紧到 0600（仅属主可读写）。
///
/// 失败只忽略：某些文件系统（如挂载的网盘）不支持改权限，
/// 为此让「保存 token」整个失败是本末倒置。
///
/// 整个函数按平台 gate：Windows 上没有等价物（ACL 由目录继承），留着会在
/// `clippy -D warnings` 下报「never used」。
#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

fn no_token_error(hub_root: &Path) -> String {
    format!(
        "这个 hub 还没配 GitLab token。\n  \
         桌面端：「设置 → GitLab」填入 Personal Access Token（需要 api 权限）\n  \
         命令行：gws gitlab token <token>\n  \
         也可以临时用环境变量 GWS_GITLAB_TOKEN 覆盖（CI / 脚本常用）\n  \
         凭据文件位置：{}",
        token_file(hub_root).display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_token_error_tells_the_user_what_to_do() {
        let dir = tempfile::TempDir::new().unwrap();
        let msg = no_token_error(dir.path());
        assert!(msg.contains("桌面端"), "{msg}");
        assert!(msg.contains("GWS_GITLAB_TOKEN"), "{msg}");
        assert!(msg.contains("gws gitlab token"), "{msg}");
    }

    #[test]
    fn token_file_lives_in_the_hub_root() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = token_file(dir.path());
        assert_eq!(p.file_name().unwrap(), HUB_TOKEN_FILE);
        assert_eq!(p.parent().unwrap(), dir.path());
    }

    #[test]
    fn stored_token_is_readable_from_the_hub_file() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(!has_stored(dir.path()));

        store(dir.path(), "glpat-abc").unwrap();
        assert!(has_stored(dir.path()));
        // **直接读文件，不用 `resolve`** —— 开发机上完全可能 export 了
        // GWS_GITLAB_TOKEN / GITLAB_TOKEN，那种情况下 `resolve` 会（正确地）
        // 优先返回环境变量里的值，拿它当断言就会得到一个莫名其妙的假红。
        assert_eq!(
            read_file(&token_file(dir.path())).unwrap().as_deref(),
            Some("glpat-abc")
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode =
                std::fs::metadata(token_file(dir.path())).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "token 文件应仅属主可读写");
        }

        clear(dir.path()).unwrap();
        assert!(!has_stored(dir.path()));
        // 再清一次不该报错：本来就没有 = 目标已达成
        clear(dir.path()).unwrap();
    }

    #[test]
    fn empty_token_is_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(store(dir.path(), "   ").is_err());
    }

    #[test]
    fn env_var_is_recognized_and_named_in_source() {
        std::env::set_var("GWS_GITLAB_TOKEN", "from-env");
        assert_eq!(from_env().as_deref(), Some("from-env"));
        // 用不存在的目录：只测 source 的分支选择，不读文件
        assert_eq!(
            source(Path::new("/nonexistent-hub-for-test")),
            "环境变量 GWS_GITLAB_TOKEN"
        );
        std::env::remove_var("GWS_GITLAB_TOKEN");
        // 这里**不断言 `from_env()` 回到 None**：开发机上可能本来就 export 了
        // GITLAB_TOKEN，它会继续被读到 —— 那是正确的行为。
    }

    #[test]
    fn two_hubs_keep_separate_credentials() {
        // 「凭据跟着 hub」的核心回归：两个 hub 互不干扰
        let a = tempfile::TempDir::new().unwrap();
        let b = tempfile::TempDir::new().unwrap();

        store(a.path(), "token-a").unwrap();
        store(b.path(), "token-b").unwrap();
        // 同样直接读文件，避开环境变量
        assert_eq!(
            read_file(&token_file(a.path())).unwrap().as_deref(),
            Some("token-a")
        );
        assert_eq!(
            read_file(&token_file(b.path())).unwrap().as_deref(),
            Some("token-b")
        );

        clear(a.path()).unwrap();
        assert!(!has_stored(a.path()));
        assert!(has_stored(b.path()), "清掉一个 hub 的凭据不该影响另一个");
    }
}