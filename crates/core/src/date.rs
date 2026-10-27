//! 日期工具。整个项目对日期的需求只有「本地时区的今天」和两种格式化，
//! 集中在这里，将来换实现只动一个文件。
//!
//! **必须用本地时区。** 分支名里的日期是约定好的「发车日期」，需求文档目录名也用
//! 它。用 UTC 会让清晨（CST 08:00 之前）创建的需求落到前一天，跟人手敲的
//! `date +%Y%m%d` 对不上。

use chrono::Local;

/// 本地时区的今天：`YYYY-MM-DD`。
pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// 本地时区的今天，紧凑形态：`YYYYMMDD`。
pub fn today_compact() -> String {
    Local::now().format("%Y%m%d").to_string()
}

/// 按位数取日期：`8` → `20261010`，`6` → `261010`。
///
/// 真实仓库里两种都有人用（`feature-20250926-xxx` 是 8 位，
/// `feature-260926-xxx` 是 6 位），所以位数做成 hub 配置项而不是写死。
pub fn today_with_digits(digits: u8) -> String {
    let full = today_compact();
    let n = (digits as usize).min(full.len());
    full[full.len() - n..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn today_has_expected_shape() {
        let d = today();
        assert_eq!(d.len(), 10, "{d}");
        assert_eq!(d.matches('-').count(), 2, "{d}");
        assert!(d.chars().filter(|c| c.is_ascii_digit()).count() == 8, "{d}");
    }

    #[test]
    fn compact_is_eight_digits() {
        let d = today_compact();
        assert_eq!(d.len(), 8, "{d}");
        assert!(d.chars().all(|c| c.is_ascii_digit()), "{d}");
    }

    #[test]
    fn digits_eight_returns_full_and_six_returns_short() {
        let full = today_compact();
        assert_eq!(today_with_digits(8), full);
        assert_eq!(today_with_digits(6), full[2..]);
    }

    #[test]
    fn digits_is_clamped_to_available_length() {
        // 传了荒谬的位数也不能 panic（配置校验会先拦住，这里是兜底）
        assert_eq!(today_with_digits(0), "");
        assert_eq!(today_with_digits(99), today_compact());
    }

    #[test]
    fn compact_and_dashed_agree_on_the_day() {
        // 同一次调用周期内两者必须是同一天
        let dashed = today();
        let compact = today_compact();
        assert_eq!(dashed.replace('-', ""), compact);
    }
}