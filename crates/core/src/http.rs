//! 极小的 HTTP 封装。
//!
//! # 为什么要单独一层
//!
//! 有两处需要「GET 一个 JSON」和「带进度下载」：GitLab 客户端
//! （[`crate::gitlab::Client`]）与服务**自更新**（[`crate::update`]，走 GitHub
//! Releases）。但它们打的是**完全不同的服务** —— 基址、错误类型、鉴权方式都不一样。
//! 让自更新去复用 GitLab 客户端在语义上是错的（「拿 GitLab 客户端访问 GitHub」），
//! 所以把这两件事抽出来共用，而不是硬塞进任何一个客户端里。
//!
//! 不做 trait 抽象：一共就两种用法，两个函数比一层抽象清楚。

use std::io::Read as _;
use std::time::Duration;

use crate::{Error, Result};

/// 整体超时。偶发慢响应不至于把命令挂死。
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// **连接**超时，单独设短一些。
///
/// 连不上时（网络不通、DNS 挂了、防火墙丢包）应该在几秒内失败并报错，
/// 而不是让人对着转圈等满 30 秒 —— 30 秒和「卡死」在使用者眼里没区别。
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// 建一个带超时的客户端。`user_agent` 带上产品名与版本，便于服务端识别来源。
pub fn client(user_agent: &str) -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(user_agent.to_string())
        .build()
        .map_err(|e| Error::other(format!("创建 HTTP 客户端失败: {e}")))
}

/// GET 一个 JSON（绝对 URL）。
pub fn get_json<T: serde::de::DeserializeOwned>(
    http: &reqwest::blocking::Client,
    url: &str,
) -> Result<T> {
    let resp = http
        .get(url)
        .send()
        .map_err(|e| Error::other(format!("请求失败 {url}: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(Error::other(format!("请求返回 {status}: {}", snippet(&text))));
    }
    resp.json::<T>()
        .map_err(|e| Error::other(format!("解析响应失败: {e}")))
}

/// 下载一个文件（绝对 URL）。
pub fn download(http: &reqwest::blocking::Client, url: &str) -> Result<Vec<u8>> {
    let resp = http
        .get(url)
        .send()
        .map_err(|e| Error::other(format!("下载失败: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(Error::other(format!("下载返回 {status}: {}", snippet(&text))));
    }
    resp.bytes()
        .map(|b| b.to_vec())
        .map_err(|e| Error::other(format!("读取下载内容失败: {e}")))
}

/// 分块下载，**带进度回调**（安装包几 MB，几秒就能下完，但有进度才看得出在跑）。
///
/// 回调参数是 `(已下载字节, 总字节)`；服务端没给 `Content-Length` 时总数为 `None`。
pub fn download_streaming(
    http: &reqwest::blocking::Client,
    url: &str,
    mut on_chunk: impl FnMut(u64, Option<u64>),
) -> Result<Vec<u8>> {
    let resp = http
        .get(url)
        .send()
        .map_err(|e| Error::other(format!("下载失败: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(Error::other(format!("下载返回 {status}: {}", snippet(&text))));
    }
    let total = resp.content_length();
    let mut out = Vec::with_capacity(total.unwrap_or(0) as usize);
    let mut buf = [0u8; 64 * 1024];
    let mut resp = resp;
    let mut last = 0u64;
    loop {
        let n = resp
            .read(&mut buf)
            .map_err(|e| Error::other(format!("读取下载内容失败: {e}")))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        let done = out.len() as u64;
        if should_report(done, total, last) {
            last = done;
            on_chunk(done, total);
        }
    }
    Ok(out)
}

/// 截断响应正文，别把一整页 HTML 错误页塞进报错里。
pub(crate) fn snippet(text: &str) -> String {
    let t = text.trim();
    let mut s: String = t.chars().take(300).collect();
    if t.chars().count() > 300 {
        s.push('…');
    }
    s
}

/// 下载进度**要不要报**：按比例节流。
///
/// 每 64 KB 报一次的话，一个 6 MB 的包会往上抛近百个回调，界面日志刷屏
/// （实测反馈：进度日志一直在追加）。这里按**每 5%** 报一次，最多 20 条；
/// 服务端没给总长度（`total=None`）时按每 512 KB 报一次。
///
/// `last` 是上次报的位置。
fn should_report(done: u64, total: Option<u64>, last: u64) -> bool {
    match total {
        Some(t) if t > 0 => done.saturating_sub(last) * 20 >= t,
        _ => done.saturating_sub(last) >= 512 * 1024,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_progress_is_throttled_to_about_twenty_reports() {
        let total = Some(5_800_000u64);
        let mut last = 0u64;
        let mut reports = 0;
        let mut done = 0u64;
        while done < 5_800_000 {
            done += 64 * 1024; // 下载就是 64 KB 一块一块来的
            if should_report(done, total, last) {
                last = done;
                reports += 1;
            }
        }
        // 5.8 MB / 64 KB ≈ 92 块 → 不节流就是 92 条；节流后最多 20 条
        assert!(reports <= 20, "报得太多了：{reports} 条（实测反馈就是嫌这个）");
        assert!(reports >= 15, "也不能报得太稀：{reports} 条");
    }

    #[test]
    fn without_content_length_it_reports_every_half_megabyte() {
        let mut last = 0u64;
        assert!(!should_report(100_000, None, last), "小步不该报");
        assert!(should_report(512 * 1024, None, last), "半 MB 要报");
        last = 512 * 1024;
        assert!(!should_report(600_000, None, last));
        assert!(should_report(1024 * 1024, None, last));
    }

    #[test]
    fn zero_total_does_not_divide_by_zero() {
        assert!(!should_report(1, Some(0), 0));
    }

    #[test]
    fn snippet_truncates_long_bodies() {
        let long = "x".repeat(1000);
        let s = snippet(&long);
        assert!(s.chars().count() <= 301, "应截断到 300 字符 + 省略号");
        assert!(s.ends_with('…'));
        assert_eq!(snippet("  short  "), "short");
    }
}