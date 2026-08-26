use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const GWS_INSTALL_HINT: &str = "gws 未安装。安装: curl -fsSL https://raw.githubusercontent.com/qq476605474/gws/main/gws -o ~/.local/bin/gws && chmod +x ~/.local/bin/gws";

const CONFIRM_SILENCE: Duration = Duration::from_millis(1500);
const WATCHDOG_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Serialize, Clone)]
pub struct RunResult {
    pub code: Option<i32>,
    pub output: String,
}

#[derive(Serialize, Clone)]
struct OutputPayload<'a> {
    chunk: &'a str,
}

#[derive(Serialize, Clone)]
struct ConfirmPayload<'a> {
    question: &'a str,
}

#[derive(Serialize, Clone)]
struct ExitPayload {
    code: Option<i32>,
}

#[derive(Clone)]
pub enum PendingEvent {
    Output(String),
    Confirm(String),
    Exit(Option<i32>),
}

impl PendingEvent {
    pub fn event_name(&self, run_id: u32) -> String {
        match self {
            PendingEvent::Output(_) => format!("gws-output:{run_id}"),
            PendingEvent::Confirm(_) => format!("gws-confirm:{run_id}"),
            PendingEvent::Exit(_) => format!("gws-exit:{run_id}"),
        }
    }
}

pub struct RunShared {
    /// 保留到 waiter 线程观察到读者 EOF 才被 take 走：
    /// 期间 respond_confirm(no) 才能通过它 kill 还在运行的进程。
    pub child: Option<Child>,
    /// respond_confirm(yes) 时 take 走写 "y\n"；handle drop 时向子进程发 EOF。
    pub stdin: Option<ChildStdin>,
    /// 前端三个事件订阅完成（replay_output 已调用）。
    pub started: bool,
    /// exit 事件已入队（waiter 已观察到进程退出）。
    pub finished: bool,
    /// started 前缓存的事件，由 replay_output 按序补发。
    pub pending: Vec<PendingEvent>,
}

impl RunShared {
    /// 记录事件：已 started 返回 Some(ev) 由调用方立刻 emit；否则缓存返回 None。
    pub fn record(&mut self, ev: PendingEvent) -> Option<PendingEvent> {
        if self.started {
            Some(ev)
        } else {
            self.pending.push(ev);
            None
        }
    }

    /// 开启直发模式并返回缓存事件（按入队顺序）。
    pub fn start(&mut self) -> Vec<PendingEvent> {
        self.started = true;
        std::mem::take(&mut self.pending)
    }
}

/// run 生命周期结束（started 且 finished，事件已全部送达）则移除。
/// replay_output 与 waiter 线程共用，集成测试直接验证此语义。
pub fn cleanup_if_done(runs: &mut HashMap<u32, RunShared>, run_id: u32) {
    if runs.get(&run_id).is_some_and(|st| st.started && st.finished) {
        runs.remove(&run_id);
    }
}

static RUN_ID: AtomicU32 = AtomicU32::new(1);
/// 集成测试以固定 run_id（90001/90002/...）直接读写，与业务 run_id 空间隔离。
pub static RUNS: LazyLock<Mutex<HashMap<u32, RunShared>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn lock_runs() -> MutexGuard<'static, HashMap<u32, RunShared>> {
    // 某个后台线程 panic 中毒后继续取数据：单个 run 的异常不应拖垮全局事件流
    RUNS.lock().unwrap_or_else(|e| e.into_inner())
}

/// 流式 run 的跨线程元数据：
/// readers/readers_done —— waiter 等 stdout/stderr 读线程 EOF 后才发 exit 事件，
/// 否则管道内未读输出会在 exit 之后才到（乱序）或因 run 被清理而丢失；
/// last_output —— watchdog 的静默判据（只看 stdout）。
struct StreamMeta {
    readers: Mutex<u32>,
    readers_done: Condvar,
    last_output: Mutex<Instant>,
}

impl StreamMeta {
    fn new(readers: u32) -> Self {
        Self { readers: Mutex::new(readers), readers_done: Condvar::new(), last_output: Mutex::new(Instant::now()) }
    }

    fn touch(&self) {
        *self.last_output.lock().unwrap() = Instant::now();
    }

    fn silent_for(&self) -> Duration {
        self.last_output.lock().unwrap().elapsed()
    }

    fn reader_done(&self) {
        *self.readers.lock().unwrap() -= 1;
        self.readers_done.notify_all();
    }

    /// 不持 RUNS 锁等待：若持锁等待会阻塞 respond_confirm 等所有命令（死锁）。
    fn wait_readers_done(&self) {
        let mut n = self.readers.lock().unwrap();
        while *n > 0 {
            n = self.readers_done.wait(n).unwrap();
        }
    }
}

fn emit_event(app: &AppHandle, run_id: u32, ev: &PendingEvent) {
    let name = ev.event_name(run_id);
    let _ = match ev {
        PendingEvent::Output(chunk) => app.emit(&name, OutputPayload { chunk: chunk.as_str() }),
        PendingEvent::Confirm(question) => app.emit(&name, ConfirmPayload { question: question.as_str() }),
        PendingEvent::Exit(code) => app.emit(&name, ExitPayload { code: *code }),
    };
}

/// 读者/watchdog/waiter 共用的事件入口。
/// 持锁 emit：跨线程事件按 record 顺序送达，且回放与直发不会交错；
/// emit 只向事件循环投递消息，无阻塞 IO，持锁安全。
fn push_event(app: &AppHandle, run_id: u32, ev: PendingEvent) {
    let mut runs = lock_runs();
    if let Some(st) = runs.get_mut(&run_id) {
        if let Some(ev) = st.record(ev) {
            emit_event(app, run_id, &ev);
        }
    }
}

/// 同步一次性执行，stdout 在前、stderr 追加换行后拼接。
pub fn run_gws_once(exe: &Path, args: &[String], cwd: &Path) -> RunResult {
    match Command::new(exe).args(args).current_dir(cwd).stdin(Stdio::null()).output() {
        Ok(out) => {
            let mut output = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            if !stderr.is_empty() {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(&stderr);
            }
            RunResult { code: out.status.code(), output }
        }
        Err(e) => RunResult { code: None, output: format!("启动失败: {e}") },
    }
}

pub fn find_gws() -> Option<PathBuf> {
    let name = if cfg!(windows) { "gws.exe" } else { "gws" };
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path).map(|dir| dir.join(name)).find(|candidate| candidate.is_file())
    })
}

/// (async)：阻塞至子进程退出，须在主线程外执行（sync fn + async 标记 → 线程池）。
#[tauri::command(async)]
pub fn run_gws(args: Vec<String>, cwd: String) -> Result<RunResult, String> {
    let exe = find_gws().ok_or_else(|| GWS_INSTALL_HINT.to_string())?;
    Ok(run_gws_once(&exe, &args, Path::new(&cwd)))
}

#[tauri::command]
pub fn run_gws_stream(app: AppHandle, args: Vec<String>, cwd: String) -> Result<u32, String> {
    let exe = find_gws().ok_or_else(|| GWS_INSTALL_HINT.to_string())?;
    let mut child = Command::new(&exe)
        .args(&args)
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 gws 失败: {e}"))?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let run_id = RUN_ID.fetch_add(1, Ordering::SeqCst);
    lock_runs().insert(
        run_id,
        RunShared { child: Some(child), stdin, started: false, finished: false, pending: Vec::new() },
    );

    let meta = Arc::new(StreamMeta::new((stdout.is_some() as u32) + (stderr.is_some() as u32)));

    // stderr 读线程：不重置 last_output（静默判据只看 stdout）
    if let Some(mut pipe) = stderr {
        let app = app.clone();
        let meta = meta.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => push_event(
                        &app,
                        run_id,
                        PendingEvent::Output(String::from_utf8_lossy(&buf[..n]).to_string()),
                    ),
                }
            }
            meta.reader_done();
        });
    }

    if let Some(mut pipe) = stdout {
        let app = app.clone();
        let meta = meta.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        meta.touch();
                        push_event(
                            &app,
                            run_id,
                            PendingEvent::Output(String::from_utf8_lossy(&buf[..n]).to_string()),
                        );
                    }
                }
            }
            meta.reader_done();
        });
    }

    // watchdog：子进程 1.5s 无 stdout 输出且未退出 → 猜测在等 stdin 确认，
    // 每次运行至多发一次 gws-confirm 事件
    {
        let app = app.clone();
        let meta = meta.clone();
        let cwd = cwd.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(WATCHDOG_INTERVAL);
            let over = {
                let runs = lock_runs();
                match runs.get(&run_id) {
                    None => true,
                    Some(st) => st.finished,
                }
            };
            if over {
                return;
            }
            if meta.silent_for() >= CONFIRM_SILENCE {
                push_event(
                    &app,
                    run_id,
                    PendingEvent::Confirm(format!("gws 在 {cwd} 等待确认（1.5s 无输出）。确认继续？")),
                );
                return;
            }
        });
    }

    // waiter：先等读者排空管道（此刻进程必已退出），再 take 出 Child wait——
    // take 早于 wait 是为了 wait 期间不持 RUNS 锁；而 Child 保留在 RUNS 到此刻，
    // 是为了 respond_confirm(no) 能 kill 尚在运行的进程。
    std::thread::spawn(move || {
        meta.wait_readers_done();
        let child = lock_runs().get_mut(&run_id).and_then(|st| st.child.take());
        let code = child.and_then(|mut c| c.wait().ok()).and_then(|s| s.code());
        push_event(&app, run_id, PendingEvent::Exit(code));
        let mut runs = lock_runs();
        if let Some(st) = runs.get_mut(&run_id) {
            st.finished = true;
        }
        cleanup_if_done(&mut runs, run_id);
    });

    Ok(run_id)
}

#[tauri::command]
pub fn replay_output(app: AppHandle, run_id: u32) -> Result<(), String> {
    let mut runs = lock_runs();
    // run 不存在 = 事件已全部直发送达或早已清理，幂等成功
    let drained = runs.get_mut(&run_id).map(|st| st.start()).unwrap_or_default();
    // 持锁 emit：与 push_event 直发路径互斥，回放事件不与直发事件交错
    for ev in &drained {
        emit_event(&app, run_id, ev);
    }
    cleanup_if_done(&mut runs, run_id);
    Ok(())
}

#[tauri::command]
pub fn respond_confirm(run_id: u32, yes: bool) -> Result<(), String> {
    let stdin = {
        let mut runs = lock_runs();
        let st = runs.get_mut(&run_id).ok_or_else(|| "run 不存在".to_string())?;
        if yes {
            st.stdin.take()
        } else {
            // child 已被 waiter take 走（进程已在退出）时无对象可 kill，视为成功
            if let Some(child) = st.child.as_mut() {
                let _ = child.kill();
            }
            None
        }
    };
    // 锁已释放再写：写管道可能阻塞（缓冲满），不能持锁做 IO
    if let Some(mut stdin) = stdin {
        let _ = stdin.write_all(b"y\n");
        let _ = stdin.flush();
    }
    Ok(())
}
