//! 名称校验。
//!
//! 这些名字最终都会变成**文件系统路径的一段**（`ws/<名>/`、`envs/<名>/`、
//! `repos/<名>/`），所以必须挡住路径穿越与非法字符。
//!
//! 旧版对应实现（`validate_ws_name`）：
//! 拒绝空、含空格或斜杠、以 `.` 开头、包含 `..`。
//! 中文等 Unicode 字符是**允许**的（工作区名常见中文）。

use crate::{Error, Result};

/// 校验一个路径组件（工作区名 / 环境标识 / 仓库名）。
///
/// `what` 用于错误文案，如「工作区」「环境标识」。
pub fn validate_component(name: &str, what: &str) -> Result<()> {
    let bad = |n: &str| -> Result<()> {
        Err(Error::InvalidName {
            what: what.to_string(),
            name: n.to_string(),
        })
    };

    if name.is_empty() {
        return bad(name);
    }
    // 空白、路径分隔符：会破坏路径结构
    if name.chars().any(char::is_whitespace) || name.contains('/') || name.contains('\\') {
        return bad(name);
    }
    // `.` 开头会生成隐藏目录；`..` 是路径穿越
    if name.starts_with('.') || name.contains("..") {
        return bad(name);
    }
    // NUL 无法出现在路径里
    if name.contains('\0') {
        return bad(name);
    }
    Ok(())
}

// ══ 分支名拼接 ═════════════════════════════════════════════════════════
//
// 自动拼接的日期和后缀里手写的日期会撞车：
// 用户在「需求名」里又写了一遍日期（`20261020-ca-mobile-...`），拼出来就成了
// `bugfix-20261020-20261020-ca-mobile-...`。真实数据里出现过，所以拼接前
// 把后缀开头重复的日期去掉。

/// 后缀开头是几项「日期 + `-`」？返回字节数。
fn leading_date_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let digits = |range: std::ops::Range<usize>| b[range].iter().all(u8::is_ascii_digit);
    let num = |range: std::ops::Range<usize>| {
        std::str::from_utf8(&b[range])
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
    };
    let month_ok = |range: std::ops::Range<usize>| matches!(num(range), Some(1..=12));
    let day_ok = |range: std::ops::Range<usize>| matches!(num(range), Some(1..=31));

    // YYYY-MM-DD-
    if b.len() >= 11
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'-'
        && digits(0..4)
        && month_ok(5..7)
        && day_ok(8..10)
    {
        return Some(11);
    }
    // YYYY-MM-
    if b.len() >= 8 && b[4] == b'-' && b[7] == b'-' && digits(0..4) && month_ok(5..7) {
        return Some(8);
    }
    // YYYYMMDD-
    if b.len() >= 9 && b[8] == b'-' && digits(0..8) && month_ok(4..6) && day_ok(6..8) {
        return Some(9);
    }
    // YYMMDD-
    if b.len() >= 7 && b[6] == b'-' && digits(0..6) && month_ok(2..4) && day_ok(4..6) {
        return Some(7);
    }
    None
}

/// 整段就是一个日期（没有后续需求名）。
fn is_bare_date(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    let num = |r: std::ops::Range<usize>| {
        std::str::from_utf8(&b[r])
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
    };
    let month_ok = |r: std::ops::Range<usize>| matches!(num(r), Some(1..=12));
    let day_ok = |r: std::ops::Range<usize>| matches!(num(r), Some(1..=31));

    matches!(b.len(), 8 | 6) && digits(0..b.len()) && {
        let (m, d) = if b.len() == 8 {
            (4..6, 6..8)
        } else {
            (2..4, 4..6)
        };
        month_ok(m) && day_ok(d)
    } || (b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && digits(0..4)
        && month_ok(5..7)
        && day_ok(8..10))
        || (b.len() == 7 && b[4] == b'-' && digits(0..4) && month_ok(5..7))
}

/// 去掉后缀开头重复出现的日期，返回 `(剩下的部分, 是否去掉过)`。
///
/// 认 `YYYYMMDD-`、`YYMMDD-`、`YYYY-MM-DD-`、`YYYY-MM-`，可连续去多个；
/// 整段就是一个日期时返回空串（拼接时只留 `前缀-日期`）。
pub fn strip_leading_dates(suffix: &str) -> (String, bool) {
    let mut rest = suffix.trim();
    let mut stripped = false;
    loop {
        if is_bare_date(rest) {
            rest = "";
            stripped = true;
            break;
        }
        if let Some(n) = leading_date_len(rest) {
            rest = &rest[n..];
            stripped = true;
            continue;
        }
        break;
    }
    (rest.trim().to_string(), stripped)
}

/// 拼分支名：`前缀-日期-后缀`，后缀开头重复的日期先去掉。
pub fn compose_branch(prefix: &str, date: &str, suffix: &str) -> String {
    let (suffix, _) = strip_leading_dates(suffix);
    [prefix.trim(), date.trim(), suffix.as_str()]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_names() {
        for ok in [
            "收银台改版",
            "feature-20261010-smart-reach",
            "pre-202609",
            "dev",
            "a",
        ] {
            assert!(validate_component(ok, "工作区").is_ok(), "应接受: {ok}");
        }
    }

    #[test]
    fn rejects_path_traversal() {
        for bad in ["..", "../etc", "a/../b", "..hidden"] {
            assert!(validate_component(bad, "工作区").is_err(), "应拒绝: {bad}");
        }
    }

    #[test]
    fn rejects_separators_and_whitespace() {
        for bad in ["a/b", "a\\b", "a b", "a\tb", "a\nb"] {
            assert!(validate_component(bad, "工作区").is_err(), "应拒绝: {bad}");
        }
    }

    #[test]
    fn rejects_empty_and_hidden() {
        assert!(validate_component("", "工作区").is_err());
        assert!(validate_component(".git", "工作区").is_err());
    }

    #[test]
    fn error_message_carries_what_and_name() {
        let err = validate_component("a/b", "环境标识").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("环境标识"), "{msg}");
        assert!(msg.contains("a/b"), "{msg}");
    }

    #[test]
    fn strips_repeated_dates_in_every_supported_form() {
        // 真实坏分支：后缀里已经写了日期，工具又拼了一遍
        let cases = [
            ("20261020-ca-mobile-vault", "ca-mobile-vault"),
            ("261020-ca-mobile-vault", "ca-mobile-vault"),
            ("2026-10-20-ca-mobile-vault", "ca-mobile-vault"),
            ("2026-10-ca-mobile-vault", "ca-mobile-vault"),
        ];
        for (input, want) in cases {
            let (got, stripped) = strip_leading_dates(input);
            assert_eq!(got, want, "{input}");
            assert!(stripped, "{input} 应识别为日期");
        }
    }

    #[test]
    fn strips_multiple_leading_dates_and_bare_date() {
        assert_eq!(
            strip_leading_dates("20261020-2026-10-20-ca"),
            ("ca".into(), true)
        );
        // 只有日期：拼接时只留 `前缀-日期`，不要出现两个日期
        assert_eq!(strip_leading_dates("20261020"), ("".into(), true));
        assert_eq!(strip_leading_dates("2026-10-20"), ("".into(), true));
        assert_eq!(strip_leading_dates("261020"), ("".into(), true));
    }

    #[test]
    fn leaves_non_date_suffixes_untouched() {
        for ok in [
            "ca-mobile-vault",
            "2026-release",
            "20261340-ca",
            "v2.3-bugfix",
            "2.3-fix",
        ] {
            assert_eq!(strip_leading_dates(ok), (ok.to_string(), false), "{ok}");
        }
    }

    #[test]
    fn compose_branch_does_not_repeat_the_date() {
        assert_eq!(
            compose_branch("bugfix", "20261021", "20261020-ca-mobile-vault"),
            "bugfix-20261021-ca-mobile-vault"
        );
        assert_eq!(
            compose_branch("feature", "20261021", ""),
            "feature-20261021"
        );
        assert_eq!(
            compose_branch("feature", "20261021", "20261020"),
            "feature-20261021"
        );
        assert_eq!(
            compose_branch("feature", "20261021", "  ca  "),
            "feature-20261021-ca"
        );
    }
}