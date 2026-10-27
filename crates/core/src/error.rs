//! 统一错误类型。
//!
//! 旧版 bash 的教训（设计方案 §8 第 11 条）：`gws rm` 最后一步 `rm -rf` 失败时
//! 没有任何检查，紧接着无条件打印「✓ 工作区已删除」，导致用户以为删干净了、
//! 实际留下空目录。**本版所有可能失败的操作都必须返回 `Result`，调用方必须处理。**

use std::path::PathBuf;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 当前目录及各级上级都没有 hub 标记文件。
    #[error("不在任何 hub 目录内（当前目录及上级均无标记文件）。创建: gws init <目录>")]
    HubNotFound,

    #[error("hub 不存在或不是有效 hub: {0}")]
    InvalidHub(PathBuf),

    /// 目录嵌套 hub 会让「向上查找标记文件」产生歧义，init 时必须拒绝。
    /// 旧版行为见设计方案 §8 第 10 条。
    #[error("目标位于已有 hub 内部，不能嵌套: {0}")]
    NestedHub(PathBuf),

    #[error("仓库不存在: {0}")]
    RepoNotFound(String),

    #[error("工作区不存在: {0}")]
    WorkspaceNotFound(String),

    #[error("工作区已存在: {0}")]
    WorkspaceExists(String),

    /// 创建工作区前的预检未通过。
    ///
    /// 带结构化明细而不是一句错误文本——界面需要按类别分别展示（仓库不存在 /
    /// 分支已被他人占用 / 本地残留），并给不同的下一步建议。
    #[error("{0}")]
    Precheck(Box<crate::workspace::lifecycle::PrecheckFailure>),

    #[error("环境不存在: {0}")]
    EnvNotFound(String),

    /// 名称含空格/斜杠、以 `.` 开头或含 `..`（防路径穿越）。
    #[error("{what} 名称非法: '{name}'")]
    InvalidName { what: String, name: String },

    /// git 命令以非零退出。保留 stderr，否则只剩一个无从排查的退出码。
    #[error("git 命令失败: git {args}\n{stderr}")]
    Git { args: String, stderr: String },

    /// 目标路径存在但删除后仍然存在——旧版正是在这里静默失败的。
    #[error("删除后路径仍然存在（可能有文件被占用或无权限）: {0}")]
    RemoveFailed(PathBuf),

    #[error("GitLab API 失败: {0}")]
    GitLab(String),

    #[error("配置文件解析失败 {path}: {source}")]
    Config {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("IO 失败 {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{0}")]
    Other(String),
}

impl Error {
    /// 附带路径的 IO 错误：`std::io::Error` 本身不含路径，定位不到是哪个文件。
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io { path: path.into(), source }
    }

    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }
}