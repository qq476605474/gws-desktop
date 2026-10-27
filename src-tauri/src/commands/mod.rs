//! Tauri 命令层：把 `gws-core` 暴露给前端。
//!
//! # 这一层只做三件事
//!
//! 1. 持有「当前 hub」这一份会话状态
//! 2. 把 core 的错误转成前端能**分类处理**的结构
//! 3. 参数/返回值的序列化边界
//!
//! **不含任何业务逻辑**——业务全在 core，CLI 与桌面端共用同一份实现。
//! 这也意味着桌面端与命令行永远不会行为不一致。

pub mod danger;
pub mod deploy;
pub mod doc;
pub mod gitlab;
pub mod hub;
pub mod mr;
pub mod paseo;
pub mod repo;
pub mod sync;
pub mod tools;
pub mod ws;

#[cfg(test)]
mod browse_tests;
#[cfg(test)]
mod tests;

use std::sync::Mutex;

use tauri::Emitter;

use gws_core::workspace::lifecycle::PrecheckFailure;
use gws_core::{Error, Hub};

/// 会话状态：当前打开的 hub。
///
/// 只存 hub 而**不是**每个工作区——工作区是从磁盘按需读的，缓存它们会引入
/// 「界面显示的是旧状态」这类难查的问题。
#[derive(Default)]
pub struct App {
    hub: Mutex<Option<Hub>>,
}

impl App {
    pub fn set_hub(&self, hub: Hub) {
        *self.hub.lock().unwrap() = Some(hub);
    }

    pub fn clear_hub(&self) {
        *self.hub.lock().unwrap() = None;
    }

    /// 取当前 hub 的副本。未打开时给出**可操作**的提示。
    pub fn hub(&self) -> Result<Hub, CommandError> {
        self.hub
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| CommandError::plain("还没有打开 hub。先在启动页选择或新建一个 hub。"))
    }

    /// 改配置用：取出来、改、再 `save_config` 写回，同时更新会话里的副本。
    pub fn update_hub<T>(
        &self,
        f: impl FnOnce(&mut Hub) -> Result<T, Error>,
    ) -> Result<T, CommandError> {
        let mut guard = self.hub.lock().unwrap();
        let hub = guard
            .as_mut()
            .ok_or_else(|| CommandError::plain("还没有打开 hub"))?;
        let out = f(hub)?;
        hub.save_config()?;
        Ok(out)
    }

    /// 只读地用一下 hub，不写回。
    pub fn with_hub<T>(&self, f: impl FnOnce(&Hub) -> Result<T, Error>) -> Result<T, CommandError> {
        let guard = self.hub.lock().unwrap();
        let hub = guard
            .as_ref()
            .ok_or_else(|| CommandError::plain("还没有打开 hub"))?;
        Ok(f(hub)?)
    }
}

// ══ 错误 ═══════════════════════════════════════════════════════════════

/// 前端能分类处理的错误。
///
/// 特别地，**创建工作区的预检失败带结构化明细**：界面要按类别分别展示
/// （仓库不存在 / 分支已被他人占用 / 本地残留），并给不同的下一步建议——
/// 把整段中文塞进一个弹窗就退化成旧版那种「看不懂就关掉」的体验了。
#[derive(Debug, Clone, serde::Serialize)]
pub struct CommandError {
    pub message: String,
    /// `precheck` 时前端可以渲染成分类清单。
    pub kind: &'static str,
    /// **Box 一下**：PrecheckFailure 有四个 Vec，直接内嵌会把 CommandError 撑到
    /// 近 200 字节，而 `Result<T, CommandError>` 是每个命令的返回类型——
    /// clippy 的 `result_large_err` 会报警，这里也确实没必要让每个 Ok 都背这么大的 Err。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precheck: Option<Box<PrecheckFailure>>,
}

impl CommandError {
    pub fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: "other",
            precheck: None,
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<Error> for CommandError {
    fn from(e: Error) -> Self {
        match e {
            Error::Precheck(f) => Self {
                message: f.to_string(),
                kind: "precheck",
                precheck: Some(f),
            },
            other => Self::plain(other.to_string()),
        }
    }
}

impl From<String> for CommandError {
    fn from(s: String) -> Self {
        Self::plain(s)
    }
}

pub type CmdResult<T> = Result<T, CommandError>;

// ══ 阻塞任务 ═══════════════════════════════════════════════════════════

/// 把一段**会阻塞**的代码放到阻塞线程池里跑。
///
/// # 为什么必须有这个（踩过的坑，别再改回去）
///
/// core 用的是 `reqwest::blocking`。它在 **debug 构建**下会走
/// `reqwest::blocking::wait::enter()`，那里嵌套新建一个 tokio runtime 再立刻销毁：
///
/// ```ignore
/// fn enter() {
///     #[cfg(debug_assertions)]
///     { let _enter = tokio::runtime::Builder::new_current_thread().build()
///           .expect("build shell runtime").enter(); }
/// }
/// ```
///
/// 而 `#[tauri::command(async)]` 的命令体是被 `async_runtime::spawn` 出来的，
/// **已经处在 tokio 运行时上下文里**——在异步上下文里 drop 一个 runtime 会直接
/// panic：
///
/// ```text
/// Cannot drop a runtime in a context where blocking is not allowed.
/// This happens when a runtime is dropped from within an asynchronous context.
/// ```
///
/// 命令体炸在 spawned 任务里，**没有任何响应回给前端**，于是 `invoke` 的 Promise
/// 永不 settle——界面上就是永远转圈。
///
/// 实测同时中招的三处：检查更新、GitLab 检测、仓库检索。而 CLI 一直是好的，
/// 因为 CLI 没有 tokio 运行时——**这就是它藏得深的原因**。
/// release 构建没有那个 `debug_assertions` 分支，所以只在 `tauri dev` 下复现。
///
/// # 为什么还要 catch_unwind
///
/// 上面这个是「为什么会 panic」；`catch_unwind` 挡的是「**任何** panic 都不该
/// 变成永久转圈」。命令体里任何一个 unwrap 炸了，前端都会卡死在 busy 遮罩里
/// 连界面都点不动——比报个错严重得多。宁可返回一条「内部错误」，也不要让
/// 人对着转圈猜。
pub async fn bg<T, F>(f: F) -> CmdResult<T>
where
    F: FnOnce() -> CmdResult<T> + Send + 'static,
    T: Send + 'static,
{
    let joined = tauri::async_runtime::spawn_blocking(move || {
        // AssertUnwindSafe：闭包捕获的都是命令入参（String / 拥有所有权的 Hub 等），
        // 一次性的，panic 后不会再有别人观察到它们的不一致状态。
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
    })
    .await;

    match joined {
        // 任务本身没能跑完（线程池关闭等）
        Err(e) => Err(CommandError::plain(format!("后台任务异常：{e}"))),
        Ok(Ok(v)) => v,
        Ok(Err(_)) => Err(CommandError::plain(
            "内部错误：后台任务崩溃了。这是程序缺陷，不是你的操作问题——\
             请把上一步操作告诉开发者，命令行（gws）同一操作通常能给出更清楚的报错。",
        )),
    }
}

// ══ 进度事件 ═══════════════════════════════════════════════════════════

/// 进度事件的固定名字。
///
/// **固定一个名字**，而不是每次操作生成一个新的：前端只订阅一次，靠事件里的
/// `label` 区分是哪次操作。若每次操作一个新名字，前端得先知道名字才能订阅，
/// 而名字要等命令返回——那恰恰是我们想避免的「什么都等命令返回」。
pub const PROGRESS_EVENT: &str = "gws://progress";

/// 把 core 的进度事件转成 Tauri 事件推给前端。
pub fn progress_cb(
    app: &tauri::AppHandle,
) -> impl Fn(gws_core::progress::ProgressEvent) + Send + Sync + 'static {
    let app = app.clone();
    move |ev| {
        // emit 失败（窗口已关）不是错误：进度丢一条无所谓
        let _ = app.emit(PROGRESS_EVENT, &ev);
    }
}
// ══ 前端传参 ═══════════════════════════════════════════════════════════

/// 模块选择的入参形态（设计方案 §6.4）。
///
/// 用 `Option<Vec<String>>` 而不是空数组表示「不限制」——空数组与「没选任何
/// 模块」在语义上要能区分开，否则界面上把勾选全取消会变成「操作全部模块」。
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct FilterInput {
    pub only: Option<Vec<String>>,
    pub exclude: Option<Vec<String>>,
}

impl From<FilterInput> for gws_core::workspace::ModuleFilter {
    fn from(f: FilterInput) -> Self {
        use std::collections::BTreeSet;
        Self {
            only: f.only.map(|v| v.into_iter().collect::<BTreeSet<_>>()),
            exclude: f.exclude.unwrap_or_default().into_iter().collect(),
        }
    }
}
