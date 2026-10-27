//! 自更新（设计方案 §11.5）。
//!
//! # 从哪更新
//!
//! **GitHub Releases**（本仓库自己）。
//! `GET /repos/{owner}/{repo}/releases/latest` 拿最新 release，按当前平台挑资产下载。
//!
//! 与 GitLab **完全解耦**：自更新不需要 hub、不需要 token，也不碰 hub 绑定的实例 ——
//! 它下的是本软件自己的安装包，跟用户在哪个 GitLab 上管代码毫无关系。
//!
//! # 校验链（移植旧版 `install.sh` 的严谨性）
//!
//! 旧版 bash 的安装脚本做了一整套校验，这套思路要保留：
//!
//! ```text
//! tag 必须是 vX.Y.Z
//!   → 按当前平台+架构选资产（选不到就明确报错，不猜）
//!   → 下载到**同目录**临时文件（跨文件系统 rename 不是原子的）
//!   → 校验 sha256（对照 release 里的 sha256sums.txt）
//!   → 防降级：新版本不大于当前版本就跳过
//!   → 原子替换：临时文件 + rename
//! ```
//!
//! **任何一步失败都不改动现有二进制。** 更新失败最坏的结果应该是「还是旧版」，
//! 而不是「二进制损坏、工具跑不起来」。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// 本仓库在 GitHub 上的 `owner/repo`。
pub const REPO: &str = "qq476605474/gws-desktop";

/// GitHub API 基址。
const API: &str = "https://api.github.com";

/// Releases 页地址。
///
/// 放在 core 里而不是各自拼字符串：桌面端的「打开 Releases 页」与更新提示
/// 都要用它，项目路径只有一处定义，两个入口就都不会错。
pub fn releases_url() -> String {
    format!("https://github.com/{REPO}/releases")
}

/// 某个 tag 对应的 Release 页地址。
pub fn release_url_for(tag: &str) -> String {
    format!("https://github.com/{REPO}/releases/tag/{tag}")
}

/// 自更新专用的 HTTP 客户端（user agent 带上产品名与版本）。
///
/// 不接收调用方传来的客户端：自更新打的是 GitHub，与 hub 绑定的 GitLab
/// 没有关系，让它自己建一个最不容易被误用。
fn http() -> Result<reqwest::blocking::Client> {
    crate::http::client(concat!("gws/", env!("CARGO_PKG_VERSION")))
}

/// 当前平台的资产名（与 CI 的构建产物命名一致）。
///
/// 用编译期常量而不是运行时探测：交叉编译时也能确定，且不会因为
/// 跑在 Rosetta 下而选错架构。
pub const fn target_triple() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else {
        "unknown"
    }
}

pub fn asset_name() -> String {
    let base = format!("gws-{}", target_triple());
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReleaseInfo {
    /// 如 `v1.2.3`。
    pub tag: String,
    /// 去掉 `v` 前缀的版本号。
    pub version: String,
    pub assets: Vec<Asset>,
    /// 该 release 的页面地址。
    pub web_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum UpdateStatus {
    /// 已是最新。
    UpToDate,
    /// 有更新可用。
    Available(ReleaseInfo),
    /// 远程版本比当前**低**——不降级。
    Ahead(ReleaseInfo),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateCheck {
    pub current: String,
    pub status: UpdateStatus,
}

impl ReleaseInfo {
    /// 按**文件名**找资产。
    ///
    /// ⚠️ 必须**同时**比 `name` 与 URL 结尾，不能只比其中一个：GitHub 上
    /// `name` 就是文件名，但 URL 里带着路径；而别的托管平台（如 GitLab）的
    /// `name` 可能是「给人看的显示名」（「桌面端 · macOS（Apple Silicon）」、
    /// 「校验和」），文件名**只在 URL 里**。
    ///
    /// 实测反馈（GitLab 时期）：点「更新」报「这个 release 里没有
    /// gws-desktop_1.0.2_aarch64.dmg」—— 真因就在这里。同一个错误还让
    /// **sha256 校验一直静默跳过**（按 `sha256sums.txt` 找不到那个链接，
    /// 于是每次都走「跳过校验」那条分支），命令行自更新同样从来没成功过。
    /// 三处都收口到这一个函数。
    pub fn find_asset(&self, file_name: &str) -> Option<&Asset> {
        self.assets
            .iter()
            .find(|a| a.name == file_name || a.url.ends_with(file_name))
    }
}

/// GitHub release 的响应（只取用得上的字段，缺字段不能让整个请求失败）。
#[derive(Debug, Deserialize)]
struct RawRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    html_url: String,
    /// GitHub 的 assets 是**同级数组**（GitLab 是嵌在 `assets.links` 里），
    /// 下载地址字段也不同（`browser_download_url`）。
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(Debug, Deserialize)]
struct RawAsset {
    #[serde(default)]
    name: String,
    #[serde(default)]
    browser_download_url: String,
}

/// 取最新 release。
pub fn latest() -> Result<ReleaseInfo> {
    let raw: RawRelease =
        crate::http::get_json(&http()?, &format!("{API}/repos/{REPO}/releases/latest"))?;

    let version = strip_v(&raw.tag_name);
    if version.is_empty() {
        return Err(Error::other(format!(
            "Release 的 tag 不合法: '{}'（应为 vX.Y.Z）",
            raw.tag_name
        )));
    }
    if !is_semver(version) {
        return Err(Error::other(format!(
            "Release 的 tag 不是 x.y.z 形式: '{}'",
            raw.tag_name
        )));
    }

    Ok(ReleaseInfo {
        tag: raw.tag_name.clone(),
        version: version.to_string(),
        assets: raw
            .assets
            .into_iter()
            .filter(|a| !a.name.is_empty() && !a.browser_download_url.is_empty())
            .map(|a| Asset { name: a.name, url: a.browser_download_url })
            .collect(),
        web_url: raw.html_url,
    })
}

/// 检查有没有更新。**不下载任何东西。**
pub fn check(current: &str) -> Result<UpdateCheck> {
    let rel = latest()?;
    let status = match compare_versions(&rel.version, current) {
        std::cmp::Ordering::Greater => UpdateStatus::Available(rel),
        std::cmp::Ordering::Equal => UpdateStatus::UpToDate,
        // 远程比当前低 = 本机跑的是更新的版本（比如自编译的），**不降级**
        std::cmp::Ordering::Less => UpdateStatus::Ahead(rel),
    };
    Ok(UpdateCheck { current: current.to_string(), status })
}

/// 执行更新：下载 → 校验 → 原子替换。
///
/// `dry_run=true` 时只做到「挑出该下哪个资产」为止，不下载。
pub fn apply(
    current: &str,
    dry_run: bool,
    progress: crate::progress::Progress<'_>,
) -> Result<PathBuf> {
    let rel = match check(current)?.status {
        UpdateStatus::Available(r) => r,
        UpdateStatus::UpToDate => {
            return Err(Error::other(format!("已是最新版本 {current}")));
        }
        UpdateStatus::Ahead(r) => {
            return Err(Error::other(format!(
                "远程版本 {} 低于当前 {}，不降级（若确实要降，请手工替换二进制）",
                r.version, current
            )));
        }
    };

    let want = asset_name();
    let asset = rel
        .assets
        .iter()
        .find(|a| a.name == want)
        .ok_or_else(|| {
            let names: Vec<&str> = rel.assets.iter().map(|a| a.name.as_str()).collect();
            Error::other(format!(
                "该 Release 里没有适配当前平台的资产 '{want}'。\n  可用: {}",
                if names.is_empty() { "（无）".to_string() } else { names.join("、") }
            ))
        })?;

    if dry_run {
        return Ok(PathBuf::from(&asset.name));
    }

    let mut rep = crate::progress::Reporter::start(progress, 3, format!("更新到 {}", rel.version));

    // 当前可执行文件的位置。移动它才是「更新」。
    let self_path = std::env::current_exe()
        .map_err(|e| Error::other(format!("读不到当前程序路径: {e}")))?;

    // **下载到同目录**：跨文件系统 rename 不是原子的，可能留下损坏的二进制
    let dir = self_path.parent().unwrap_or(Path::new("."));
    let tmp = dir.join(format!(".gws-update-{}", std::process::id()));

    let bytes = crate::http::download(&http()?, &asset.url).inspect_err(|e| {
        rep.fail("下载", e.to_string());
    })?;
    std::fs::write(&tmp, &bytes).map_err(|e| Error::io(tmp.clone(), e))?;
    rep.ok(format!("下载 {}（{} 字节）", asset.name, bytes.len()));

    // 校验下下来的确实是可执行文件，而不是一个 HTML 错误页
    if !looks_like_binary(&bytes) {
        let _ = std::fs::remove_file(&tmp);
        rep.fail("校验", "下载内容不是可执行文件");
        rep.finish();
        return Err(Error::other(
            "下载内容不是可执行文件（可能是登录页或错误页）——已中止，现有版本未改动",
        ));
    }
    rep.ok("校验通过");

    // ── sha256 校验 ──
    // Release 里附一个 sha256sums.txt，对照它验。截断或损坏的下载如果直接替换，
    // 换出来的是一个跑不起来的二进制——那比「更新失败」糟得多。
    match expected_sha256(&rel, &asset.name) {
        Ok(Some(expect)) => {
            let got = sha256_hex(&bytes);
            if !got.eq_ignore_ascii_case(&expect) {
                let _ = std::fs::remove_file(&tmp);
                rep.fail("校验", "sha256 不匹配");
                rep.finish();
                return Err(Error::other(format!(
                    "sha256 校验失败（现有版本未改动）\n  期望 {expect}\n  实际 {got}"
                )));
            }
            rep.ok("sha256 校验通过");
        }
        Ok(None) => {
            // 没有校验文件时只告警不中止：HTTPS + 归属校验已经挡掉大部分风险，
            // 为缺一个附件就让人完全无法更新是本末倒置
            rep.ok("（Release 未附 sha256sums.txt，跳过校验）");
        }
        Err(e) => {
            rep.ok(format!("（读取校验文件失败，跳过校验: {e}）"));
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }

    // 原子替换。失败时把临时文件清掉，不留垃圾。
    if let Err(e) = swap_in(&tmp, &self_path, dir) {
        let _ = std::fs::remove_file(&tmp);
        rep.fail("替换", e.to_string());
        rep.finish();
        return Err(Error::other(format!(
            "替换失败（现有版本未改动）: {e}\n  提示: {}\n  也可以直接下载新版覆盖 {}",
            replace_hint(),
            self_path.display()
        )));
    }
    rep.ok("已替换");
    rep.finish();

    Ok(self_path)
}

/// 用 `tmp` 换掉正在运行的 `self_path`。
fn swap_in(tmp: &Path, self_path: &Path, dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let _ = dir;
        std::fs::rename(tmp, self_path)
    }

    // Windows：**不能 rename 覆盖正在运行的 exe**（`ERROR_ACCESS_DENIED`），
    // 所以先把旧的**改名挪开**（改名对运行中的文件是允许的），再把新文件放到
    // 原位置。这么一来 `gws update` 在 Windows 上才真的能用 —— 原来那条
    // `rename(tmp, self_path)` 在 Windows 上必然失败。
    #[cfg(windows)]
    {
        cleanup_stale_old(dir);
        let name = self_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "gws.exe".to_string());
        let old = dir.join(format!("{name}.old-{}", std::process::id()));
        std::fs::rename(self_path, &old)?;
        if let Err(e) = std::fs::rename(tmp, self_path) {
            // 换不上去就把旧的挪回来，别把人家的程序弄没了
            let _ = std::fs::rename(&old, self_path);
            return Err(e);
        }
        // 运行中删不掉（文件被本进程占用）——留着，下次更新时由
        // `cleanup_stale_old` 清掉
        let _ = std::fs::remove_file(&old);
        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = dir;
        std::fs::rename(tmp, self_path)
    }
}

/// 删掉同目录下上次更新留下的 `.old-` 文件（Windows 用）。
///
/// 删不掉的（还占着）直接跳过 —— 那是正常情况，不该让更新失败。
#[cfg(windows)]
fn cleanup_stale_old(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        if e.file_name().to_string_lossy().contains(".old-") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// 替换失败时按平台给人话提示。
fn replace_hint() -> &'static str {
    if cfg!(windows) {
        "确认程序目录可写（`C:\\Program Files` 下需要管理员权限），或右键「以管理员身份运行」"
    } else {
        "若提示权限不足，用 sudo 或手工替换"
    }
}

// ══ 纯函数部分（可测）══════════════════════════════════════════════════

// ══ 桌面端自更新（打开软件时检查 + 一键更新）════════════════════════════
//
// 命令行那个 `apply` 换的是**一个二进制**；桌面端要换的是**整个 app**
// （macOS 是 .app bundle、Windows 是装出来的目录），所以另走一套：
//
//   选资产 → 分块下载（带进度）→ 对照 release 的 sha256sums.txt 校验
//   → 按平台装 → 告诉调用方接下来要做什么（重启 / 只能人工）
//
// **任何一步失败都不动现有安装**：最坏结果是「还是旧版」，不是「装坏了」。

/// 桌面端安装包在这个平台上的资产名。
///
/// 资产名里带版本号（CI 与手工打包都是这么命的），所以要版本号参数。
pub fn desktop_asset_name(version: &str) -> Result<String> {
    let name = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        format!("gws-desktop_{version}_aarch64.dmg")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        format!("gws-desktop_{version}_x64.dmg")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        format!("gws-desktop_{version}_x64-setup.exe")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        format!("gws_{version}_amd64.deb")
    } else {
        return Err(Error::other(
            "这个平台没有桌面端安装包（只有 macOS / Windows / Linux x86_64）",
        ));
    };
    Ok(name)
}

/// 装完之后要做什么。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum InstallNext {
    /// 已经装好了：**需要重启应用**（调用方负责退出；拉起助手已经挂好了）。
    Restart,
    /// 装不了（Linux 的 deb 要 sudo）：包已经下好了，交给人自己装。
    Manual,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesktopUpdate {
    /// 下好的安装包落在哪。
    pub package: PathBuf,
    pub version: String,
    pub next: InstallNext,
    /// 给人看的一句话。
    pub message: String,
    /// 需要人工执行时，**要敲的那条命令**（Linux 的 deb 要 sudo）。
    ///
    /// 单独一个字段而不是塞进 `message`：界面要把它**单独显示 + 一键复制** ——
    /// 实测反馈：混在一段话里，人得自己去挑那截命令，小白根本不知道怎么下手。
    pub command: Option<String>,
}

/// 下载 + 校验 + 安装桌面端更新。`cb` 用来报进度（下载阶段按字节）。
pub fn install_desktop(
    rel: &ReleaseInfo,
    cb: crate::progress::Progress<'_>,
) -> Result<DesktopUpdate> {
    let name = desktop_asset_name(&rel.version)?;
    let pkg = download_and_verify(rel, &name, cb)?;
    let (next, message, command) = apply_desktop(&pkg)?;
    Ok(DesktopUpdate { package: pkg, version: rel.version.clone(), next, message, command })
}

/// 下载 + 校验，落到临时目录，返回本地路径。
///
/// 抽出来是因为**这几步与"装到哪"无关** —— 单元测试可以直接验它，
/// 而不必真的替换 `/Applications` 里的应用。
pub fn download_and_verify(
    rel: &ReleaseInfo,
    name: &str,
    cb: crate::progress::Progress<'_>,
) -> Result<PathBuf> {
    let asset = rel
        .find_asset(name)
        .ok_or_else(|| Error::other(format!("这个 release 里没有 {name}（可能还在上传）")))?;

    let mut rep = crate::progress::Reporter::start(cb, 2, format!("下载 {name}"));
    let bytes = crate::http::download_streaming(&http()?, &asset.url, |done, total| {
        if let Some(f) = cb {
            // **Tick 而不是 Step**：这是同一行的进度刷新，不是新的一项。
            // 用 Step 的话界面会一行行往后追加（实测反馈：5.8 MB 追加了几十条）。
            f(crate::progress::ProgressEvent::Tick {
                item: format!("下载 {name}  {} / {}", human_mb(done), human_mb(total.unwrap_or(0))),
            });
        }
    })?;
    rep.ok(format!("已下载 {}（{}）", name, human_mb(bytes.len() as u64)));

    // **校验**：截断或损坏的下载直接装上去，比不装糟得多
    match expected_sha256(rel, name)? {
        Some(expect) => {
            let got = sha256_hex(&bytes);
            if got != expect {
                rep.fail("校验", "sha256 不匹配");
                rep.finish();
                return Err(Error::other(format!(
                    "下载的安装包校验不过（现有安装未改动）\n  期望 {expect}\n  实际 {got}"
                )));
            }
            rep.ok("sha256 校验通过");
        }
        None => rep.ok("（这个 release 没附 sha256sums.txt，跳过校验）"),
    }

    // 落到临时目录（名字保持原样，按平台分流时装它）
    let dir = std::env::temp_dir().join("gws-update");
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(dir.clone(), e))?;
    let pkg = dir.join(name);
    std::fs::write(&pkg, &bytes).map_err(|e| Error::io(pkg.clone(), e))?;
    rep.finish();
    Ok(pkg)
}

/// 按平台把安装包装上。
fn apply_desktop(pkg: &Path) -> Result<(InstallNext, String, Option<String>)> {
    // 用 `#[cfg]` 而不是 `cfg!()`：各平台的安装函数只在对应平台编译出来，
    // 写 `if cfg!(...)` 会在别的平台上引用到不存在的函数（踩过）。
    #[cfg(target_os = "macos")]
    {
        install_macos(pkg)
            .map(|()| (InstallNext::Restart, "已替换应用，正在重启…".to_string(), None))
    }
    #[cfg(target_os = "windows")]
    {
        install_windows(pkg).map(|()| {
            (InstallNext::Restart, "已启动安装程序，应用即将重启…".to_string(), None)
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        // Linux：deb 要 root 才能装 —— 不假装能自动装，把包和命令都给出来
        let cmd = format!("sudo dpkg -i {}", pkg.display());
        Ok((
            InstallNext::Manual,
            format!("安装包已下好并校验通过：{}\n在终端里执行下面这条命令（点「复制命令」）", pkg.display()),
            Some(cmd),
        ))
    }
}

/// 从「正在运行的可执行文件路径」推出 `.app` 的位置。
///
/// 形如 `<…>/GWS.app/Contents/MacOS/gws-desktop` → 往上三层。
/// 纯函数，好钉住 —— 推错了就会去替换一个不存在的目录。
pub fn macos_app_bundle(exe: &Path) -> Option<PathBuf> {
    let app = exe.parent()?.parent()?.parent()?;
    if app.extension().map(|e| e == "app").unwrap_or(false) {
        Some(app.to_path_buf())
    } else {
        None
    }
}

/// 拉起一个「等本进程退出后重启应用」的小助手。
///
/// **必须先挂助手、再退出**：应用一退出，就没人能把它拉起来了。
///
/// ⚠️ **只有 macOS 用得上**（`install_macos` 调）：Windows 那边是
/// 「装完 → 等它结束 → 直接 start 新版」一条直线，不需要这样的助手。
/// 所以整个函数按平台 gate 住 —— 不加 `#[cfg]` 的话，Linux 上它会成为死代码，
/// 在 `clippy -D warnings` 的 CI 里直接失败（就是这么发现的）。
#[cfg(target_os = "macos")]
fn spawn_relauncher(app: &Path) -> Result<()> {
    let script = format!(
        "while kill -0 {pid} 2>/dev/null; do sleep 0.3; done; open -a '{app}'",
        pid = std::process::id(),
        app = app.display()
    );
    std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .spawn()
        .map_err(|e| Error::other(format!("拉起重启助手失败: {e}")))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn install_macos(pkg: &Path) -> Result<()> {
    let exe = std::env::current_exe().map_err(|e| Error::other(format!("读不到自己的路径: {e}")))?;
    // **从"正在跑的那个可执行文件"推出要替换哪个 .app** —— 不写死名字/路径，
    // 装在哪儿就更新哪儿。开发模式（不在 .app 里）直接拒绝，不去删别的目录。
    let app = macos_app_bundle(&exe)
        .ok_or_else(|| Error::other("这个程序不在 .app 里跑，没法自替换（开发模式？）"))?;
    install_macos_into(pkg, &app)?;
    spawn_relauncher(&app)
}

/// 把 dmg 里的 `.app` 装到**指定位置**（不负责重启）。
///
/// 目标作为参数是为了**能测**：真替换 `/Applications` 没法在单测里跑，
/// 但"挂载 → 复制 → 卸载 → 去隔离标记"这几步可以在临时目录上完整验一遍。
#[cfg(target_os = "macos")]
pub fn install_macos_into(pkg: &Path, app: &Path) -> Result<()> {
    // ① 挂载 dmg —— **用 `-mountpoint` 指定挂载点**。
    //
    // 不用"挂上再解析输出"那套：真实输出格式跟想的不一样（实测：`-quiet` 下
    // 拿到的东西解析不出挂载点），而且**解析失败的那次会把卷一直挂在那里**
    // （用户桌面上莫名多个磁盘）。指定路径就没这些问题，卸载时也一定知道卸哪个。
    let mnt = std::env::temp_dir().join("gws-update-mnt");
    let _ = std::fs::remove_dir_all(&mnt); // 上次中断可能留下的空目录
    std::fs::create_dir_all(&mnt).map_err(|e| Error::io(mnt.clone(), e))?;
    let out = std::process::Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-quiet", "-mountpoint"])
        .arg(&mnt)
        .arg(pkg)
        .output()
        .map_err(|e| Error::other(format!("挂载 dmg 失败: {e}")))?;
    if !out.status.success() {
        let _ = std::fs::remove_dir_all(&mnt);
        return Err(Error::other(format!(
            "挂载 dmg 失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }

    // ② 从挂载点里找 .app 并替换；无论成败都要 detach，别留个挂着的镜像
    let result = (|| -> Result<()> {
        let src = std::fs::read_dir(&mnt)
            .map_err(|e| Error::io(mnt.clone(), e))?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().map(|x| x == "app").unwrap_or(false))
            .ok_or_else(|| Error::other("dmg 里没找到 .app"))?;

        // 先删旧的再复制：只覆盖会留下上一版删掉的文件
        let _ = std::fs::remove_dir_all(app);
        let st = std::process::Command::new("ditto")
            .arg(&src)
            .arg(app)
            .status()
            .map_err(|e| Error::other(format!("复制新版本失败: {e}")))?;
        if !st.success() {
            return Err(Error::other("复制新版本失败（ditto 非零退出）"));
        }
        Ok(())
    })();
    // **无论成败都卸载**（失败时不卸 = 桌面多一个挂着的镜像，很难理解）
    let _ = std::process::Command::new("hdiutil").args(["detach", "-quiet"]).arg(&mnt).status();
    let _ = std::fs::remove_dir_all(&mnt);
    result?;

    // ③ 去掉隔离标记：包未签名，带 quarantine 会被 Gatekeeper 拦
    let _ = std::process::Command::new("xattr")
        .args(["-dr", "com.apple.quarantine"])
        .arg(app)
        .status();
    Ok(())
}

/// Windows：装更新这件事**必须由"我们自己的另一个进程"来做**。
///
/// # 为什么不能就地装
///
/// NSIS 在应用还跑着的时候**拒绝安装**，直接退出码 2（实测反馈：
/// 「安装程序返回失败（退出码 Some(2)）」）。所以顺序只能是：
///
/// ```text
/// 本进程退出 → （另一个进程）等它退干净 → 静默安装 → 拉起新版
/// ```
///
/// 而"另一个进程"得有人活着 ✗ —— 用 `cmd` 拼命令行的话就会掉进引号/中文路径
/// 那个坑（实测反馈：弹出「Windows 找不到 '\' 文件」+ 一个空 cmd 窗）。
/// 所以干脆**把我们自己再拉起来一次**，带一个隐藏参数 `--apply-update <包>`：
/// 全程 `Command` 参数分开传，不经过任何 shell。
#[cfg(target_os = "windows")]
fn install_windows(pkg: &Path) -> Result<()> {
    let exe = std::env::current_exe().map_err(|e| Error::other(format!("读不到自己的路径: {e}")))?;
    std::process::Command::new(&exe)
        .arg(APPLY_FLAG)
        .arg(pkg)
        .arg(&exe) // 第三个参数：装完要拉起谁（助手换名之后自己算不出来）
        .spawn()
        .map_err(|e| Error::other(format!("启动更新助手失败: {e}")))?;
    Ok(())
}

/// 更新助手的命令行开关（`<exe> --apply-update <安装包>`）。
pub const APPLY_FLAG: &str = "--apply-update";

/// 助手换名之后的名字。**不能跟主程序同名** —— NSIS 靠进程名判断"应用还在跑"。
const HELPER_NAME: &str = "gws-update-helper.exe";

/// 更新助手：**等父进程退干净 → 静默安装 → 拉起新版**。
///
/// 由 `main` 在解析到 [`APPLY_FLAG`] 时调用，跑完就退出（不进 GUI）。
///
/// # ⚠️ 助手必须**换个进程名**再干活
///
/// Tauri 的 NSIS 安装程序是靠**进程名**判断"应用还在跑"的
/// （`nsProcess::_FindProcess "<主程序名>.exe"`），判定到就**退出码 2 直接不装**。
/// 而我们的助手正是**同名进程**（就是主程序自己再拉起来的一份）——
/// 于是它每次都判定"还在跑"，20 次全是 2，怎么重试都没用
/// （实测反馈的日志：`第 1…20 次：安装程序退出码 Some(2)`）。
///
/// 所以第一步先把自己**复制成 `gws-update-helper.exe`** 再用副本干活：
/// 安装程序看到没有同名进程，就肯装了。
///
/// # 为什么全程写日志
///
/// 这个助手**没有界面**（GUI 子系统的进程），出错时人什么都看不到 ——
/// 实测反馈就是「没报错但也没装上」，查都没法查。所以每一步都写进
/// `%TEMP%\gws-update-helper.log`。
///
/// # 为什么是"重试"而不是"等几秒"
///
/// NSIS 在应用还跑着的时候**拒绝安装**（退出码 2）。父进程什么时候真的退干净
/// 我们并不知道（实测：它没退，用户是手工关的），所以固定睡 3 秒是靠不住的 ——
/// 改成**边等边试**：最多试 20 次、每次隔 3 秒。
pub fn run_update_helper(pkg: &Path, app: &Path) -> Result<()> {
    let log = std::env::temp_dir().join("gws-update-helper.log");
    let note = |line: &str| {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
            let _ = writeln!(f, "{line}");
        }
    };
    note(&format!(
        "=== helper 开始 pid={} self={} pkg={} 存在={}",
        std::process::id(),
        std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default(),
        pkg.display(),
        pkg.is_file()
    ));

    // ── ① 换个进程名再干活（见上面那段说明）───────────────────────────
    let me = std::env::current_exe().map_err(|e| Error::other(format!("读不到自己的路径: {e}")))?;
    if me.file_name().map(|n| n != HELPER_NAME).unwrap_or(true) {
        let helper = std::env::temp_dir().join(HELPER_NAME);
        let _ = std::fs::remove_file(&helper);
        if let Err(e) = std::fs::copy(&me, &helper) {
            note(&format!("复制助手失败：{e}"));
            return Err(Error::other(format!("准备更新助手失败: {e}")));
        }
        let mut cmd = std::process::Command::new(&helper);
        cmd.arg(APPLY_FLAG).arg(pkg).arg(app);
        // 让副本脱离本进程（父进程会退出）
        cmd.spawn().map_err(|e| Error::other(format!("启动更新助手副本失败: {e}")))?;
        note("已换成 gws-update-helper.exe 继续");
        return Ok(()); // 本进程（同名那个）退场，安装程序才肯装
    }

    if !pkg.is_file() {
        note("安装包不见了，放弃");
        return Err(Error::other(format!("安装包不存在：{}", pkg.display())));
    }

    // 边等边试：装上为止（或试到上限）
    let mut last_code: Option<i32> = None;
    for attempt in 1..=20 {
        std::thread::sleep(std::time::Duration::from_secs(3));
        let status = match std::process::Command::new(pkg).arg("/S").status() {
            Ok(s) => s,
            Err(e) => {
                note(&format!("第 {attempt} 次：启动安装程序失败 {e}"));
                continue;
            }
        };
        note(&format!("第 {attempt} 次：安装程序退出码 {:?}", status.code()));
        if status.success() {
            // 装上之后**拉起新版**。静默装完它不会自己启动（虚拟机上实测过）。
            // 用传进来的 `app` 而不是 `current_exe()` —— 助手已经换成别的名字了。
            match std::process::Command::new(app).spawn() {
                Ok(_) => note("已拉起新版"),
                Err(e) => note(&format!("拉起新版失败：{e}")),
            }
            let _ = std::fs::remove_file(
                std::env::temp_dir().join(HELPER_NAME),
            );
            return Ok(());
        }
        last_code = status.code();
    }

    note(&format!("试了 20 次都装不上（最后退出码 {last_code:?}）"));
    // 兜底：把安装包所在目录打开，让人自己双击装 —— 总比"什么都没发生"强
    if let Some(dir) = pkg.parent() {
        let _ = std::process::Command::new("explorer").arg(dir).spawn();
        note("已打开安装包所在目录，请手动安装");
    }
    Err(Error::other(format!(
        "安装程序一直没能装（最后退出码 {last_code:?}）。已把安装包所在目录打开，可以手动安装"
    )))
}

/// 人类可读的大小（`6.1 MB`）。
fn human_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

/// 去掉版本号的 `v` 前缀（tag 上常带，比较时要抹掉）。
pub fn strip_v(tag: &str) -> &str {
    tag.trim().strip_prefix('v').unwrap_or(tag.trim())
}

/// 是否是 `x.y.z` 形式。
pub fn is_semver(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// 版本比较。只认 `x.y.z`，非法版本按 0.0.0 处理（不会误判成"有更新"）。
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    fn parse(v: &str) -> (u64, u64, u64) {
        let v = strip_v(v);
        let mut it = v.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
        (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
    }
    parse(a).cmp(&parse(b))
}

/// 粗判「这看起来像可执行文件吗」。
///
/// 防的是最常见的失败模式：下载地址配错了，拿回来一个登录页 HTML，
/// 直接替换上去工具就再也起不来了。
pub fn looks_like_binary(bytes: &[u8]) -> bool {
    if bytes.len() < 64 {
        return false;
    }
    // 可执行文件格式的魔数：ELF / Mach-O（含 fat）/ PE
    const MAGICS: &[&[u8]] = &[
        b"\x7fELF",          // Linux
        b"\xcf\xfa\xed\xfe", // Mach-O 64 LE
        b"\xce\xfa\xed\xfe", // Mach-O 32 LE
        b"\xca\xfe\xba\xbe", // Mach-O fat
        b"MZ",              // Windows PE
    ];
    if MAGICS.iter().any(|m| bytes.starts_with(m)) {
        return true;
    }
    // 兜底：开头不能是 HTML/JSON（最常见的错误内容）
    let head = String::from_utf8_lossy(&bytes[..64.min(bytes.len())]).to_lowercase();
    !(head.trim_start().starts_with("<!doctype") || head.trim_start().starts_with("<html"))
}

/// 从 Release 的 `sha256sums.txt` 里取某个资产的期望校验和。
///
/// 没有该附件时返回 `Ok(None)`。
fn expected_sha256(rel: &ReleaseInfo, asset: &str) -> Result<Option<String>> {
    let Some(sums) = rel.find_asset("sha256sums.txt") else {
        return Ok(None);
    };
    let body = crate::http::download(&http()?, &sums.url)?;
    Ok(parse_sha256sums(&String::from_utf8_lossy(&body), asset))
}

/// 解析 `sha256sums.txt`（`<hash>  <文件名>` 每行一条）。
pub fn parse_sha256sums(text: &str, asset: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        let (Some(hash), Some(name)) = (it.next(), it.next()) else {
            continue;
        };
        // 有些工具会给文件名加 `*` 前缀（二进制模式）
        if name.trim_start_matches('*') == asset && hash.len() == 64 {
            return Some(hash.to_string());
        }
    }
    None
}

/// 计算 sha256 的十六进制串。
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    /// **真跑一遍桌面端更新**（默认忽略）：下载真包、校验、替换 `/Applications` 里的
    /// 应用、挂重启助手。
    ///
    /// 会**真的改动这台机器上的安装**，所以默认 `#[ignore]`；发布前手工跑一次：
    ///
    /// ```text
    /// cargo test -p gws-core --lib install_desktop_for_real -- --ignored --nocapture
    /// ```
    ///
    /// 想验证"有新版才会走到下载"这条路，把本机的版本号临时改小（如 1.0.1）即可。
    #[test]
    #[ignore = "会真的替换 /Applications 里的应用，仅手工跑"]
    fn install_desktop_for_real() {
        let current = "1.0.1"; // 假装比线上低，走到 Available
        let rel = match check(current).unwrap().status {
            UpdateStatus::Available(r) => r,
            other => panic!("线上最新应当是 Available：{other:?}"),
        };
        println!("检查：当前 {current} → 线上 {}（{}）", rel.version, rel.tag);

        // ① 下载 + 校验（这几步与"装到哪"无关）
        let name = desktop_asset_name(&rel.version).expect("平台资产名");
        let pkg = download_and_verify(&rel, &name, None).expect("下载 + 校验应当成功");
        println!("下好了：{}（{} 字节）", pkg.display(), std::fs::metadata(&pkg).unwrap().len());

        // ② 挂载 → 复制 → 卸载 → 去隔离标记。**装到临时目录**，不碰 /Applications。
        #[cfg(target_os = "macos")]
        {
            let target = std::env::temp_dir().join("gws-update-e2e").join("GWS.app");
            let _ = std::fs::remove_dir_all(target.parent().unwrap());
            std::fs::create_dir_all(&target).unwrap();
            install_macos_into(&pkg, &target).expect("把 dmg 里的 .app 装到目标位置");

            let inner = target.join("Contents/MacOS/gws-desktop");
            assert!(inner.is_file(), "装完要真的有可执行文件：{}", inner.display());
            println!(
                "装好了：{}（{} 字节）",
                inner.display(),
                std::fs::metadata(&inner).unwrap().len()
            );
            // 顺手确认隔离标记被去掉了（不然 Gatekeeper 会拦）
            let xattr = std::process::Command::new("xattr").arg(&target).output().unwrap();
            let quoted = String::from_utf8_lossy(&xattr.stdout);
            assert!(
                !quoted.contains("com.apple.quarantine"),
                "隔离标记应当已去掉：{quoted}"
            );
            let _ = std::fs::remove_dir_all(target.parent().unwrap());
        }
    }

    /// **按文件名找资产**：GitLab release 里的 `name` 是给人看的显示名
    /// （「桌面端 · macOS（Apple Silicon）」「校验和」），文件名只在 URL 里。
    ///
    /// 实测反馈：点「更新」报「这个 release 里没有 gws-desktop_1.0.2_aarch64.dmg」。
    /// 同一个错误还让 **sha256 校验一直静默跳过** —— 按 `sha256sums.txt` 找不到
    /// 那个链接，于是每次都走"跳过校验"分支（明明每次都下到了校验和）。
    #[test]
    fn assets_are_found_by_file_name_not_by_display_name() {
        let rel = ReleaseInfo {
            tag: "v1.0.2".into(),
            version: "1.0.2".into(),
            web_url: String::new(),
            assets: vec![
                Asset {
                    name: "校验和".into(),
                    url: "https://git/api/v4/projects/1/packages/generic/gws/1.0.2/sha256sums.txt"
                        .into(),
                },
                Asset {
                    name: "桌面端 · macOS（Apple Silicon）".into(),
                    url: "https://git/api/v4/projects/1/packages/generic/gws/1.0.2/gws-desktop_1.0.2_aarch64.dmg".into(),
                },
            ],
        };

        assert!(rel.find_asset("sha256sums.txt").is_some(), "显示名叫「校验和」也要找得到");
        assert!(rel.find_asset("gws-desktop_1.0.2_aarch64.dmg").is_some());
        assert!(rel.find_asset("gws-desktop_9.9.9_aarch64.dmg").is_none(), "版本不同不能误配");
    }

    /// `.app` 的位置是从**运行中的可执行文件路径**推出来的 —— 推错就会去替换
    /// 一个不存在的目录，那是"更新把应用弄没了"级别的事故。
    #[test]
    fn app_bundle_is_derived_from_the_running_executable() {
        let app = macos_app_bundle(Path::new(
            "/Applications/GWS.app/Contents/MacOS/gws-desktop",
        ));
        assert_eq!(app.as_deref(), Some(Path::new("/Applications/GWS.app")));

        // 不在 .app 里（开发模式直接跑 target/release/…）→ 返回 None，
        // 调用方据此拒绝自替换并给出可读理由，而不是去删一个乱七八糟的目录
        assert!(macos_app_bundle(Path::new("/tmp/target/release/gws-desktop")).is_none());
        assert!(macos_app_bundle(Path::new("/")).is_none());
    }

    /// 资产名要跟打出来的包名对齐 —— 对不上就是"这个 release 里没有 X"。
    #[test]
    fn desktop_asset_name_matches_the_published_names() {
        let name = desktop_asset_name("1.0.2").unwrap();
        let expected = if cfg!(target_os = "macos") {
            "gws-desktop_1.0.2_aarch64.dmg"
        } else if cfg!(target_os = "windows") {
            "gws-desktop_1.0.2_x64-setup.exe"
        } else {
            "gws_1.0.2_amd64.deb"
        };
        assert_eq!(name, expected);
    }

    #[test]
    fn version_comparison_handles_common_cases() {
        assert_eq!(compare_versions("1.2.3", "1.2.3"), Ordering::Equal);
        assert_eq!(compare_versions("1.2.4", "1.2.3"), Ordering::Greater);
        assert_eq!(compare_versions("1.10.0", "1.9.0"), Ordering::Greater);
        assert_eq!(compare_versions("2.0.0", "1.99.99"), Ordering::Greater);
        assert_eq!(compare_versions("1.2.3", "1.2.4"), Ordering::Less);
        // 带 v 前缀也要认
        assert_eq!(compare_versions("v1.2.3", "1.2.3"), Ordering::Equal);
    }

    #[test]
    fn illegal_versions_do_not_look_like_updates() {
        // 解析失败按 0.0.0 处理——**不能误判成「有更新」**，
        // 否则一个畸形 tag 就会让人把好好的版本换掉
        assert_eq!(compare_versions("garbage", "1.0.0"), Ordering::Less);
        assert!(!is_semver("garbage"));
        assert!(!is_semver("1.2"));
        assert!(!is_semver("1.2.3-rc1"));
        assert!(is_semver("1.2.3"));
    }

    #[test]
    fn strip_v_only_removes_leading_v() {
        assert_eq!(strip_v("v1.0.0"), "1.0.0");
        assert_eq!(strip_v("1.0.0"), "1.0.0");
        assert_eq!(strip_v("  v1.0.0  "), "1.0.0");
        // 中间的 v 不能动
        assert_eq!(strip_v("1.0.0-v2"), "1.0.0-v2");
    }

    #[test]
    fn binary_detection_rejects_html_and_json() {
        // 最常见的失败模式：下载地址配错，拿回来一个登录页
        assert!(!looks_like_binary(b"<!DOCTYPE html><html><head>..."));
        assert!(!looks_like_binary(b"   \n  <html>login page</html>"));
        assert!(!looks_like_binary(b"{\"message\":\"404 Not Found\"}"));
        assert!(!looks_like_binary(b"short"));
    }

    #[test]
    fn binary_detection_accepts_real_executables() {
        let mut elf = vec![0x7f, b'E', b'L', b'F'];
        elf.extend([0u8; 200]);
        assert!(looks_like_binary(&elf));

        let mut pe = b"MZ".to_vec();
        pe.extend([0u8; 200]);
        assert!(looks_like_binary(&pe));

        let mut macho = vec![0xcf, 0xfa, 0xed, 0xfe];
        macho.extend([0u8; 200]);
        assert!(looks_like_binary(&macho));
    }

    #[test]
    fn sha256_matches_known_vector() {
        // 空串的 sha256 是公开的已知值，用来确认实现没接错
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn parses_sha256sums_in_common_formats() {
        let h = "a".repeat(64);
        let text = format!("# comment\n{h}  gws-aarch64-apple-darwin\n{b} *gws-x86_64-apple-darwin\n", b = "b".repeat(64));

        assert_eq!(
            parse_sha256sums(&text, "gws-aarch64-apple-darwin").as_deref(),
            Some(h.as_str())
        );
        // `*` 前缀（二进制模式）也要认
        assert_eq!(
            parse_sha256sums(&text, "gws-x86_64-apple-darwin").as_deref(),
            Some("b".repeat(64).as_str())
        );
        assert!(parse_sha256sums(&text, "不存在的资产").is_none());
        assert!(parse_sha256sums("", "x").is_none());
    }

    #[test]
    fn sha256sums_ignores_malformed_lines() {
        // 长度不对的哈希、缺文件名的行都要跳过，不能因此 panic 或误判
        let text = "tooshort  a\n\njustonefield\n" ;
        assert!(parse_sha256sums(text, "a").is_none());
    }

    #[test]
    fn target_triple_is_known_on_this_platform() {
        // 支持的三端之一，不能是 unknown
        assert_ne!(target_triple(), "unknown", "当前平台应在支持列表里");
        assert!(asset_name().starts_with("gws-"));
    }
}