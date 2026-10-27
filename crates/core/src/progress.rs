//! 长操作的进度上报。
//!
//! # 为什么放在 core 而不是各调用方自己搞
//!
//! 「拉 22 个仓库」「同步 22 个仓库的环境」「克隆文档库」这些操作要跑几十秒。
//! 界面必须能显示**当前在执行什么、完成了多少**——否则用户看到的就是一个
//! 不动的转圈，分不清在跑还是卡死（这正是旧版做得比本版好的地方）。
//!
//! 但 core 的硬约束是**不打印、不 emit**（设计方案 §1），所以这里只定义
//! 「怎么报告」，由调用方决定转成终端输出、Tauri 事件、还是丢弃。
//!
//! # 用法
//!
//! ```ignore
//! repo::add(&hub, &urls, Some(&|ev| eprintln!("{ev}")))?;   // CLI 打 stderr
//! repo::add(&hub, &urls, None)?;                            // 测试与不需要进度处
//! ```

use std::fmt;

/// 一次进度事件。
///
/// 可序列化是为了让桌面端能把原样推给前端渲染，不用在中间层再翻译一遍。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum ProgressEvent {
    /// 开始一项总数为 `total` 的操作。
    Start { total: usize, label: String },
    /// 完成第 `done` 项（1-based）。
    ///
    /// `ok=false` 时是失败项——**失败也要报**，否则「22 个里错了 3 个」这段时间
    /// 界面会显示成一切正常。
    Step { done: usize, total: usize, item: String, ok: bool, detail: String },
    /// **同一件事的进度刷新**（不是新的一项）。
    ///
    /// 界面应当**替换**上一条 Tick，而不是往后追加 —— 下载一个 6 MB 的包，
    /// 每 64 KB 追加一条就是近百行刷屏（实测反馈：进度日志一直在追加）。
    ///
    /// **只给一句完整的话**（如「下载 x.dmg  3.1 MB / 5.8 MB」），不带 done/total：
    /// 这条不是"第几项"，头部那个「x / y」计数不该被它带偏。
    /// 与 [`ProgressEvent::Step`] 的区别就是「这是新的一行，还是同一行的新数字」。
    Tick { item: String },
    /// 收尾。
    Done { label: String },
}

impl fmt::Display for ProgressEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProgressEvent::Start { total, label } => write!(f, "{label}（共 {total} 项）"),
            ProgressEvent::Tick { item } => write!(f, "{item}"),
            ProgressEvent::Step { done, total, item, ok, detail } => {
                let mark = if *ok { "✓" } else { "✗" };
                if detail.is_empty() {
                    write!(f, "[{done}/{total}] {mark} {item}")
                } else {
                    write!(f, "[{done}/{total}] {mark} {item}  {detail}")
                }
            }
            ProgressEvent::Done { label } => write!(f, "{label} 完成"),
        }
    }
}

/// 进度回调。`None` = 不上报。
///
/// 用 `Option<&dyn Fn>` 而不是 trait：调用点极少（五六个慢操作），
/// 传闭包比定义一堆实现类型轻得多，也不需要为「不关心进度」单独造一个类型。
pub type Progress<'a> = Option<&'a (dyn Fn(ProgressEvent) + Send + Sync)>;

/// 把 `Progress` 包成一个可多次调用的上报器。
///
/// 直接写 `if let Some(p) = progress { p(...) }` 在每个循环里很啰嗦，
/// 而且容易漏掉 Start/Done。
pub struct Reporter<'a> {
    cb: Progress<'a>,
    label: String,
    total: usize,
    done: usize,
}

impl<'a> Reporter<'a> {
    /// 开始一项操作。`total` 未知时传 0。
    pub fn start(cb: Progress<'a>, total: usize, label: impl Into<String>) -> Self {
        let label = label.into();
        if let Some(f) = cb {
            f(ProgressEvent::Start { total, label: label.clone() });
        }
        Self { cb, label, total, done: 0 }
    }

    /// 记一项完成。
    pub fn step(&mut self, item: impl Into<String>, ok: bool, detail: impl Into<String>) {
        self.done += 1;
        if let Some(f) = self.cb {
            f(ProgressEvent::Step {
                done: self.done,
                total: self.total,
                item: item.into(),
                ok,
                detail: detail.into(),
            });
        }
    }

    /// 成功一项。
    pub fn ok(&mut self, item: impl Into<String>) {
        self.step(item, true, "");
    }

    /// 失败一项（**仍要报出来**）。
    pub fn fail(&mut self, item: impl Into<String>, reason: impl Into<String>) {
        self.step(item, false, reason);
    }

    pub fn finish(self) {
        if let Some(f) = self.cb {
            f(ProgressEvent::Done { label: self.label });
        }
    }
}

/// 打印到 stderr 的回调，供 CLI 使用。
///
/// 走 stderr 而不是 stdout：stdout 是命令的**结果**（可被重定向/解析），
/// 进度是过程信息，两者混在一起就没法把结果单独管道出去了。
pub fn stderr_reporter(ev: ProgressEvent) {
    eprintln!("{ev}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn reporter_emits_start_steps_and_done_in_order() {
        let seen: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let cb = |ev: ProgressEvent| seen.lock().unwrap().push(ev.to_string());

        let mut r = Reporter::start(Some(&cb), 2, "拉取仓库");
        r.ok("a");
        r.fail("b", "网络超时");
        r.finish();

        let got = seen.lock().unwrap().clone();
        assert_eq!(got.len(), 4, "{got:?}");
        assert!(got[0].contains("共 2 项"), "{}", got[0]);
        assert!(got[1].contains("[1/2] ✓ a"), "{}", got[1]);
        // 失败项必须带 ✗ 与原因——否则界面会显示成一切正常
        assert!(got[2].contains("[2/2] ✗ b"), "{}", got[2]);
        assert!(got[2].contains("网络超时"), "{}", got[2]);
        assert!(got[3].contains("完成"), "{}", got[3]);
    }

    #[test]
    fn no_callback_is_a_cheap_noop() {
        // 测试与不关心进度的调用方走这条路径，不能 panic
        let mut r = Reporter::start(None, 1, "x");
        r.ok("a");
        r.finish();
    }

    #[test]
    fn unknown_total_is_allowed() {
        let mut r = Reporter::start(None, 0, "扫描");
        r.ok("a");
        r.finish();
    }
}