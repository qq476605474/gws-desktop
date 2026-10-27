//! 配置文件的结构版本校验。
//!
//! ## 为什么要在反序列化之前单独检查 `schema`
//!
//! 旧版本写的 `gws.json` / `.workspace.json` 字段与本版不同。如果直接丢给 serde，
//! 用户看到的会是「missing field \`title\`」——完全不知道发生了什么。
//!
//! 先单独把 `schema` 读出来比对，就能给出「这是旧结构，本程序不支持」这种
//! **能直接指导下一步**的错误。新旧两版对 hub 与工作区的约定差异很大，
//! 静默兼容只会让用户在不自知的损坏状态下工作。

use std::path::Path;

use crate::{Error, Result};

/// 校验 JSON 文本的 `schema` 字段是否等于期望值。
///
/// 通过后再交给 serde 做完整反序列化。
pub fn check_version(text: &str, path: &Path, expected: u32) -> Result<()> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|source| Error::Config { path: path.to_path_buf(), source })?;

    match value.get("schema").and_then(serde_json::Value::as_u64) {
        Some(v) if v == u64::from(expected) => Ok(()),
        Some(v) => Err(Error::other(format!(
            "{} 的结构版本不支持：文件是 {v}，本程序支持 {expected}。\n\
             这多半是**另一个版本**写下的文件 —— 不同版本对目录与字段的约定不同，\n\
             不自动接管。请另建新工作区，或手工核对后迁移。",
            path.display()
        ))),
        None => Err(Error::other(format!(
            "{} 缺少 schema 字段，无法判断结构版本（文件可能已损坏）",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> std::path::PathBuf {
        std::path::PathBuf::from("/tmp/x.json")
    }

    #[test]
    fn accepts_matching_version() {
        assert!(check_version(r#"{"schema":2,"x":1}"#, &path(), 2).is_ok());
    }

    #[test]
    fn rejects_mismatched_version_with_actionable_message() {
        let err = check_version(r#"{"schema":1,"name":"x","branch":"y"}"#, &path(), 2).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("版本不支持"), "{msg}");
        assert!(msg.contains("文件是 1"), "{msg}");
        assert!(msg.contains("支持 2"), "{msg}");
        // 必须告诉用户下一步怎么办，而不是只说"不支持"
        assert!(msg.contains("另建新工作区"), "{msg}");
    }

    #[test]
    fn rejects_missing_schema() {
        let err = check_version(r#"{"name":"x"}"#, &path(), 2).unwrap_err();
        assert!(err.to_string().contains("缺少 schema"), "{err}");
    }

    #[test]
    fn rejects_malformed_json_as_config_error() {
        let err = check_version("{ 不是 json", &path(), 2).unwrap_err();
        assert!(matches!(err, Error::Config { .. }), "{err}");
    }

    #[test]
    fn rejects_non_numeric_schema() {
        let err = check_version(r#"{"schema":"二"}"#, &path(), 2).unwrap_err();
        assert!(err.to_string().contains("缺少 schema"), "{err}");
    }
}