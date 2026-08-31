use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Runtime};

const GWS_INSTALL_HINT: &str = if cfg!(windows) {
    "gws 未安装。gws 是 bash 脚本：请先安装 Git for Windows（自带 Git Bash），\
     在「开始菜单 → Git → Git Bash」里执行：\
     curl -fsSL https://raw.githubusercontent.com/qq476605474/gws/main/gws -o ~/.local/bin/gws，\
     并把 %USERPROFILE%\\.local\\bin 加入用户 PATH。本客户端会自动借助 Git 的 bash 解释执行"
} else {
    "gws 未安装。安装: curl -fsSL https://raw.githubusercontent.com/qq476605474/gws/main/gws -o ~/.local/bin/gws && chmod +x ~/.local/bin/gws"
};

const CONFIRM_SILENCE: Duration = Duration::from_millis(1500);
const WATCHDOG_INTERVAL: Duration = Duration::from_millis(250);

/// confirm_timeout_ms 的取值边界：下限 250ms = watchdog 轮询周期——更小的阈值与轮询
/// 粒度无意义，且 Some(0) 会让持续输出型命令在首个轮询点就被误弹"等待确认"；
/// 上限 1 小时防调用方误传巨大值把确认等待撑到不可用。None 保持默认 1500ms。
fn clamped_confirm_timeout(confirm_timeout_ms: Option<u64>) -> Duration {
    const MIN_MS: u64 = 250;
    const MAX_MS: u64 = 3_600_000;
    confirm_timeout_ms
        .map(|v| v.clamp(MIN_MS, MAX_MS))
        .map(Duration::from_millis)
        .unwrap_or(CONFIRM_SILENCE)
}

/// 确认超时的展示文案：整秒显示 "30s"，非整秒保留一位小数 "1.5s"。
fn confirm_secs_text(silence: Duration) -> String {
    if silence.as_millis() % 1000 == 0 {
        format!("{}s", silence.as_millis() / 1000)
    } else {
        format!("{}s", silence.as_secs_f64())
    }
}

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

#[derive(Clone, Debug)]
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

/// 取锁并从中毒状态恢复：某个后台线程 panic 不应拖垮全局事件流。
/// lock_runs 与 StreamMeta 内部锁统一使用此策略。
fn lock_ignoring_poison<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn lock_runs() -> MutexGuard<'static, HashMap<u32, RunShared>> {
    lock_ignoring_poison(&RUNS)
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
        *lock_ignoring_poison(&self.last_output) = Instant::now();
    }

    fn silent_for(&self) -> Duration {
        lock_ignoring_poison(&self.last_output).elapsed()
    }

    fn reader_done(&self) {
        *lock_ignoring_poison(&self.readers) -= 1;
        self.readers_done.notify_all();
    }

    /// 不持 RUNS 锁等待：若持锁等待会阻塞 respond_confirm 等所有命令（死锁）。
    fn wait_readers_done(&self) {
        let mut n = lock_ignoring_poison(&self.readers);
        while *n > 0 {
            n = self.readers_done.wait(n).unwrap_or_else(|e| e.into_inner());
        }
    }
}

fn emit_event<R: Runtime>(app: &AppHandle<R>, run_id: u32, ev: &PendingEvent) {
    let name = ev.event_name(run_id);
    let _ = match ev {
        PendingEvent::Output(chunk) => app.emit(&name, OutputPayload { chunk: chunk.as_str() }),
        PendingEvent::Confirm(question) => app.emit(&name, ConfirmPayload { question: question.as_str() }),
        PendingEvent::Exit(code) => app.emit(&name, ExitPayload { code: *code }),
    };
}

/// 已持 RUNS 锁的事件入口（push_event 的无锁内核）。
/// waiter/watchdog 用它把「检查 run 状态 + 记录事件」组合进同一次临界区，
/// 保证 Confirm 不可能越过 Exit 之后入队（竞态原子化）。
fn push_event_locked<R: Runtime>(
    runs: &mut HashMap<u32, RunShared>,
    app: &AppHandle<R>,
    run_id: u32,
    ev: PendingEvent,
) {
    if let Some(st) = runs.get_mut(&run_id) {
        if let Some(ev) = st.record(ev) {
            emit_event(app, run_id, &ev);
        }
    }
}

/// 读者线程的事件入口。
/// 持锁 emit：跨线程事件按 record 顺序送达，且回放与直发不会交错；
/// emit 只向事件循环投递消息，无阻塞 IO，持锁安全。
fn push_event<R: Runtime>(app: &AppHandle<R>, run_id: u32, ev: PendingEvent) {
    let mut runs = lock_runs();
    push_event_locked(&mut runs, app, run_id, ev);
}

/// 追加 buf 到 carry，返回可安全解码的前缀；尾部不完整的多字节序列留在 carry 等下一块；
/// 确定无效的字节序列替换为 U+FFFD 逐字节消费（保证收敛）。
/// 管道 read 按任意字节边界返回块，对每块独立 from_utf8_lossy 会把跨块边界的
/// 多字节字符（如中文）劈成 U+FFFD —— 突发 >4KB 输出时中文必然乱码。
fn take_complete_utf8(carry: &mut Vec<u8>, buf: &[u8]) -> String {
    carry.extend_from_slice(buf);
    let mut out = String::new();
    loop {
        match std::str::from_utf8(carry) {
            Ok(s) => {
                out.push_str(s);
                carry.clear();
                return out;
            }
            Err(e) => {
                let valid = e.valid_up_to();
                // from_utf8 报错时 [0, valid) 必为合法 UTF-8
                out.push_str(std::str::from_utf8(&carry[..valid]).unwrap());
                match e.error_len() {
                    // 确定无效：该字节替换为 U+FFFD 并消费它，继续解码后续字节
                    Some(_) => {
                        out.push('\u{FFFD}');
                        carry.drain(..valid + 1);
                    }
                    // 尾部不完整的多字节序列：留在 carry 等下一块补全
                    None => {
                        carry.drain(..valid);
                        return out;
                    }
                }
            }
        }
    }
}

/// EOF 冲刷：残留的不完整序列已无后续字节可补全，按 lossy 语义收尾。
fn flush_carry(carry: &mut Vec<u8>) -> String {
    let rest = String::from_utf8_lossy(carry).into_owned();
    carry.clear();
    rest
}

/// gws 的启动形态：program 加 prefix_args 再接业务参数，可附带要补进 PATH 的目录。
/// unix：program=gws 脚本本体，prefix 为空；
/// Windows：原生可执行（gws.exe）prefix 为空；.cmd/.bat 须经 `cmd /C` 转发；
/// 无扩展名脚本是 bash 脚本 → program=bash.exe、prefix=[脚本路径]，
/// 并把 Git 的 usr/bin 等补进 PATH——GUI 进程的系统 PATH 不含这些目录，
/// 脚本里的 mktemp/basename 等 coreutils 会找不到（--noprofile 不执行 /etc/profile）。
#[derive(Debug, Clone)]
pub struct GwsLaunch {
    pub program: PathBuf,
    pub prefix_args: Vec<String>,
    pub path_append: Vec<PathBuf>,
}

impl GwsLaunch {
    /// 直接执行 program（无前缀参数、无 PATH 追加）：集成测试构造 mock 启动形态也用它。
    pub fn direct(program: PathBuf) -> Self {
        Self { program, prefix_args: Vec::new(), path_append: Vec::new() }
    }

    /// 组装进程：program + prefix_args + args，已设工作目录；调用方自管 stdio。
    /// path_append 只追加不前置：用户 PATH 里已有的工具（自装 git 等）优先级不变。
    pub fn command(&self, args: &[String], cwd: &Path) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(self.prefix_args.iter()).args(args).current_dir(cwd);
        #[cfg(windows)]
        {
            // GUI 进程拉起控制台程序（bash.exe / cmd.exe）时，Windows 默认弹一个
            // 黑控制台窗口——CREATE_NO_WINDOW 抑制它；std 会自动补 CREATE_UNICODE_ENVIRONMENT
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        if !self.path_append.is_empty() {
            if let Some(cur) = std::env::var_os("PATH") {
                let mut paths: Vec<PathBuf> = std::env::split_paths(&cur).collect();
                for p in &self.path_append {
                    if !paths.contains(p) {
                        paths.push(p.clone());
                    }
                }
                if let Ok(joined) = std::env::join_paths(paths) {
                    cmd.env("PATH", joined);
                }
            }
        }
        cmd
    }
}

/// 同步一次性执行，stdout 在前、stderr 追加换行后拼接。
pub fn run_gws_once(launch: &GwsLaunch, args: &[String], cwd: &Path) -> RunResult {
    match launch.command(args, cwd).stdin(Stdio::null()).output() {
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

/// 文件是否以 shebang 开头：Windows 上无扩展名文件不能直接执行，
/// 凭此判定它是需要解释器的脚本（而非被改名放错位置的 exe）。
fn has_shebang(path: &Path) -> bool {
    std::fs::File::open(path)
        .and_then(|mut f| {
            let mut head = [0u8; 2];
            f.read_exact(&mut head).map(|_| head == *b"#!")
        })
        .unwrap_or(false)
}

/// 用户主目录：unix 用 HOME；Windows 上 GUI 进程常无 HOME，取 USERPROFILE。
fn home_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
    } else {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

/// 找 Git for Windows 的 bash.exe（WSL 的 bash 别名不在此列：路径含 Windows 且必须是 exe）。
/// 固定安装位优先于 PATH：PATH 里的 "bash" 可能是别的发行版/残留，官方安装位语义确定。
fn find_windows_bash() -> Option<PathBuf> {
    for dir in ["C:\\Program Files\\Git\\bin", "C:\\Program Files\\Git\\usr\\bin"] {
        let candidate = PathBuf::from(dir).join("bash.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join("bash.exe"))
            .find(|c| c.is_file() && c.to_string_lossy().to_lowercase().contains("windows"))
    })
}

/// 由 bash.exe 反推 Git 安装根目录：bin（或 usr\bin）逐级向上找同时含
/// usr\bin 与 mingw64 的目录——不依赖固定盘符/安装位置（用户可装在 D 盘等）。
fn git_root_from_bash(bash: &Path) -> Option<PathBuf> {
    let mut dir = bash.parent()?;
    loop {
        if dir.join("usr").join("bin").is_dir() && dir.join("mingw64").is_dir() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// 定位 gws 及其启动方式。
///
/// unix：PATH + 常见安装位找可执行的 `gws`，直接运行。
/// Windows 依次尝试：
///   1. `gws.exe`（PATH → %USERPROFILE%\.local\bin → %USERPROFILE%\bin）——直接执行；
///   2. `gws.cmd` / `gws.bat` —— 经 `cmd /C` 转发（Rust 不允许直接 spawn .bat/.cmd）；
///   3. 无扩展名 `gws`（curl 官方安装法的产物，bash 脚本）——找 Git 的 bash.exe 解释执行。
/// GUI 从 Dock/开始菜单启动不继承交互 shell 的 PATH（.zshrc 等未加载），
/// ~/.local/bin 这类用户安装位常不在其中——故 PATH 之外再按常见安装位兜底。
pub fn find_gws() -> Option<GwsLaunch> {
    if cfg!(windows) {
        return find_gws_windows();
    }
    let home = home_dir()?;
    let dirs: Vec<PathBuf> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .chain([
            home.join(".local/bin"),
            home.join("bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/opt/homebrew/bin"),
        ])
        .collect();
    dirs.iter().map(|d| d.join("gws")).find(|c| c.is_file()).map(GwsLaunch::direct)
}

#[cfg(windows)]
fn find_gws_windows() -> Option<GwsLaunch> {
    let home = home_dir()?;
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
    dirs.push(home.join(".local").join("bin"));
    dirs.push(home.join("bin"));
    for dir in &dirs {
        // 原生可执行优先：用户自备的 gws.exe 不被脚本版盖过
        let exe = dir.join("gws.exe");
        if exe.is_file() {
            return Some(GwsLaunch::direct(exe));
        }
        for ext in ["cmd", "bat"] {
            let f = dir.join(format!("gws.{ext}"));
            if f.is_file() {
                // Rust ≥1.58 禁止直接 spawn .bat/.cmd（防参数注入），须走 cmd /C
                return Some(GwsLaunch {
                    program: PathBuf::from("cmd"),
                    prefix_args: vec!["/C".into(), f.to_string_lossy().into_owned()],
                    path_append: Vec::new(),
                });
            }
        }
        let script = dir.join("gws");
        if script.is_file() && has_shebang(&script) {
            let bash = find_windows_bash()?;
            let root = git_root_from_bash(&bash);
            let mut path_append = Vec::new();
            if let Some(root) = &root {
                for sub in ["usr\\bin", "mingw64\\bin", "cmd"] {
                    let p = root.join(sub);
                    if p.is_dir() {
                        path_append.push(p);
                    }
                }
            }
            return Some(GwsLaunch {
                program: bash,
                // --noprofile --norc：不读 Git Bash 的 profile（GUI 进程的 PATH 是
                // 系统/用户 PATH，profile 改写反而引入 /usr/bin 风格的 unix 路径）
                prefix_args: vec![
                    "--noprofile".into(),
                    "--norc".into(),
                    script.to_string_lossy().into_owned(),
                ],
                path_append,
            });
        }
    }
    None
}

#[cfg(not(windows))]
fn find_gws_windows() -> Option<GwsLaunch> {
    None
}

/// (async)：阻塞至子进程退出，须在主线程外执行（sync fn + async 标记 → 线程池）。
#[tauri::command(async)]
pub fn run_gws(args: Vec<String>, cwd: String) -> Result<RunResult, String> {
    let launch = find_gws().ok_or_else(|| GWS_INSTALL_HINT.to_string())?;
    Ok(run_gws_once(&launch, &args, Path::new(&cwd)))
}

/// 泛型 Runtime：生产走 Wry，集成测试可用 mock runtime 的 AppHandle 直调。
/// confirm_timeout_ms：stdout 静默多久后发 gws-confirm（None → 默认 1500ms）。
/// 慢命令（sync 的静默 git 阶段、repo add 的 clone）静默远超默认值，
/// 由前端按命令传大值防假确认；真读 stdin 的命令（gws drop）保持默认。
#[tauri::command]
pub fn run_gws_stream<R: Runtime>(
    app: AppHandle<R>,
    args: Vec<String>,
    cwd: String,
    confirm_timeout_ms: Option<u64>,
) -> Result<u32, String> {
    let launch = find_gws().ok_or_else(|| GWS_INSTALL_HINT.to_string())?;
    spawn_stream(&app, &launch, &args, &cwd, confirm_timeout_ms)
}

/// spawn + 事件编排。与命令入口拆开：集成测试直接传 mock 可执行文件路径，
/// 不必篡改进程级 PATH（会与并行测试相互干扰）。
pub fn spawn_stream<R: Runtime>(
    app: &AppHandle<R>,
    launch: &GwsLaunch,
    args: &[String],
    cwd: &str,
    confirm_timeout_ms: Option<u64>,
) -> Result<u32, String> {
    // 各后台线程要求 'static：入口处克隆出所有权句柄（launch/cwd 是函数体内消费掉的借用）
    let app = app.clone();
    let mut child = launch
        .command(args, Path::new(cwd))
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

    // stderr 读线程：不重置 last_output（静默判据只看 stdout）。
    // carry 跨 read 块重组 UTF-8，块边界劈裂的多字节字符（中文）不产生 U+FFFD。
    if let Some(mut pipe) = stderr {
        let app = app.clone();
        let meta = meta.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut carry: Vec<u8> = Vec::new();
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let chunk = take_complete_utf8(&mut carry, &buf[..n]);
                        if !chunk.is_empty() {
                            push_event(&app, run_id, PendingEvent::Output(chunk));
                        }
                    }
                }
            }
            let rest = flush_carry(&mut carry);
            if !rest.is_empty() {
                push_event(&app, run_id, PendingEvent::Output(rest));
            }
            meta.reader_done();
        });
    }

    if let Some(mut pipe) = stdout {
        let app = app.clone();
        let meta = meta.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut carry: Vec<u8> = Vec::new();
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        meta.touch();
                        let chunk = take_complete_utf8(&mut carry, &buf[..n]);
                        if !chunk.is_empty() {
                            push_event(&app, run_id, PendingEvent::Output(chunk));
                        }
                    }
                }
            }
            let rest = flush_carry(&mut carry);
            if !rest.is_empty() {
                push_event(&app, run_id, PendingEvent::Output(rest));
            }
            meta.reader_done();
        });
    }

    // watchdog：子进程超时无 stdout 输出且未退出 → 猜测在等 stdin 确认，
    // 每次运行至多发一次 gws-confirm 事件。超时按 run 可配置（confirm_timeout_ms）：
    // 真读 stdin 的命令（gws drop）用默认 1.5s；慢命令由前端传大值防假确认。
    // 「确认未结束 + 记录 Confirm」在单次加锁内完成：否则 waiter 可能在
    // 检查与记录之间置 finished，Confirm 落到 Exit 之后（乱序或误发）。
    {
        let app = app.clone();
        let meta = meta.clone();
        let confirm_silence = clamped_confirm_timeout(confirm_timeout_ms);
        let secs_text = confirm_secs_text(confirm_silence);
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
            if meta.silent_for() < confirm_silence {
                continue;
            }
            // 单次加锁：确认未结束才记录 Confirm（与 waiter 的 Exit 临界区互斥）
            let mut runs = lock_runs();
            if runs.get(&run_id).is_some_and(|st| !st.finished) {
                let question = format!("gws 已 {secs_text} 无输出，可能正在等待输入。继续执行？");
                push_event_locked(&mut runs, &app, run_id, PendingEvent::Confirm(question));
            }
            return;
        });
    }

    // waiter：先等读者排空管道（此刻进程必已退出），再 take 出 Child wait——
    // take 早于 wait 是为了 wait 期间不持 RUNS 锁；而 Child 保留在 RUNS 到此刻，
    // 是为了 respond_confirm(no) 能 kill 尚在运行的进程。
    // 「记录 Exit + 置 finished + 清理判断」在单次加锁内完成（原子化）：
    // watchdog 的 Confirm 检查与之互斥，不可能再插到 Exit 之后。
    std::thread::spawn(move || {
        meta.wait_readers_done();
        let child = lock_runs().get_mut(&run_id).and_then(|st| st.child.take());
        let code = child.and_then(|mut c| c.wait().ok()).and_then(|s| s.code());
        // 单次加锁：记录 Exit + 置 finished + 清理判断原子完成
        let mut runs = lock_runs();
        push_event_locked(&mut runs, &app, run_id, PendingEvent::Exit(code));
        if let Some(st) = runs.get_mut(&run_id) {
            st.finished = true;
        }
        cleanup_if_done(&mut runs, run_id);
    });

    Ok(run_id)
}

/// 泛型 Runtime：生产走 Wry，集成测试可用 mock runtime 的 AppHandle 直调。
#[tauri::command]
pub fn replay_output<R: Runtime>(app: AppHandle<R>, run_id: u32) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_secs_text_formats_whole_and_fractional_seconds() {
        assert_eq!(confirm_secs_text(Duration::from_millis(1500)), "1.5s");
        assert_eq!(confirm_secs_text(Duration::from_millis(100)), "0.1s");
        assert_eq!(confirm_secs_text(Duration::from_millis(30000)), "30s");
        assert_eq!(confirm_secs_text(Duration::from_millis(5000)), "5s");
    }

    #[test]
    fn confirm_timeout_clamped_to_polling_bounds() {
        // None → 默认 1500ms 不变
        assert_eq!(clamped_confirm_timeout(None), CONFIRM_SILENCE);
        // 下限 250ms（watchdog 轮询周期）：Some(0) 不再对持续输出命令乱发 confirm
        assert_eq!(clamped_confirm_timeout(Some(0)), Duration::from_millis(250));
        assert_eq!(clamped_confirm_timeout(Some(100)), Duration::from_millis(250));
        assert_eq!(clamped_confirm_timeout(Some(249)), Duration::from_millis(250));
        // 区间内原样透传
        assert_eq!(clamped_confirm_timeout(Some(250)), Duration::from_millis(250));
        assert_eq!(clamped_confirm_timeout(Some(1500)), Duration::from_millis(1500));
        assert_eq!(clamped_confirm_timeout(Some(30_000)), Duration::from_millis(30_000));
        // 上限 1 小时
        assert_eq!(clamped_confirm_timeout(Some(3_600_000)), Duration::from_millis(3_600_000));
        assert_eq!(clamped_confirm_timeout(Some(u64::MAX)), Duration::from_millis(3_600_000));
    }

    #[test]
    fn ascii_passes_through_in_small_chunks() {
        // 两个小块拼成同一句：ASCII 无多字节序列，全部直通
        let mut carry = Vec::new();
        assert_eq!(take_complete_utf8(&mut carry, b"hel"), "hel");
        assert_eq!(take_complete_utf8(&mut carry, b"lo world"), "lo world");
        assert!(carry.is_empty(), "ASCII 不应残留 carry");
    }

    #[test]
    fn cjk_char_split_1_plus_2_and_2_plus_1() {
        let bytes = "中".as_bytes(); // 3 字节
        let mut carry = Vec::new();
        assert_eq!(take_complete_utf8(&mut carry, &bytes[..1]), "");
        assert_eq!(carry.len(), 1, "劈开的后 2 字节应留在 carry");
        assert_eq!(take_complete_utf8(&mut carry, &bytes[1..]), "中");
        assert!(carry.is_empty());

        let mut carry = Vec::new();
        assert_eq!(take_complete_utf8(&mut carry, &bytes[..2]), "");
        assert_eq!(carry.len(), 2, "劈开的后 1 字节应留在 carry");
        assert_eq!(take_complete_utf8(&mut carry, &bytes[2..]), "中");
        assert!(carry.is_empty());
    }

    #[test]
    fn consecutive_cjk_split_across_chunks() {
        // 三个「中」按 1/1/4/3 字节切块：连续字符在多块间反复劈裂仍能无损重组
        let bytes = "中中中".as_bytes();
        let mut carry = Vec::new();
        assert_eq!(take_complete_utf8(&mut carry, &bytes[..1]), "");
        assert_eq!(take_complete_utf8(&mut carry, &bytes[1..2]), "");
        assert_eq!(take_complete_utf8(&mut carry, &bytes[2..6]), "中中");
        assert_eq!(take_complete_utf8(&mut carry, &bytes[6..]), "中");
        assert!(carry.is_empty());
    }

    #[test]
    fn invalid_byte_becomes_replacement_and_keeps_going() {
        // 确定无效的 0xFF → U+FFFD，且不卡死：后续有效内容继续正常解码
        let mut carry = Vec::new();
        assert_eq!(take_complete_utf8(&mut carry, b"a\xFFb"), "a\u{FFFD}b");
        assert!(carry.is_empty(), "无效字节应被逐个消费");
        assert_eq!(take_complete_utf8(&mut carry, "中".as_bytes()), "中");
    }

    #[test]
    fn invalid_byte_between_cjk_chars() {
        let mut carry = Vec::new();
        let mut input = "中".as_bytes().to_vec();
        input.push(0xFF);
        input.extend_from_slice("文".as_bytes());
        assert_eq!(take_complete_utf8(&mut carry, &input), "中\u{FFFD}文");
        assert!(carry.is_empty());
    }

    #[test]
    fn eof_flush_emits_replacement_for_residual() {
        // EOF 时 carry 里只剩劈裂的半个「中」：按 lossy 冲刷为一个 U+FFFD
        let mut carry = Vec::new();
        assert_eq!(take_complete_utf8(&mut carry, &"中".as_bytes()[..2]), "");
        assert_eq!(flush_carry(&mut carry), "\u{FFFD}");
        assert!(carry.is_empty());
        // EOF 时无残留则冲刷出空串
        assert_eq!(flush_carry(&mut carry), "");
    }

    #[test]
    fn split_at_every_offset_roundtrips() {
        // 在每个字节边界劈一次：两块 + EOF 冲刷后必须无损还原
        let s = "a中文b\u{1F600}c"; // 混合 ASCII、3 字节中文、4 字节 emoji
        let bytes = s.as_bytes();
        for k in 0..=bytes.len() {
            let mut carry = Vec::new();
            let first = take_complete_utf8(&mut carry, &bytes[..k]);
            let second = take_complete_utf8(&mut carry, &bytes[k..]);
            let rest = flush_carry(&mut carry);
            assert_eq!(format!("{first}{second}{rest}"), s, "劈裂点 k={k}");
        }
    }

    #[test]
    fn six_thousand_cjk_across_4k_chunks_no_replacement() {
        // 复现路径：单次写 6000 个「中」（18000 字节）被 4096 字节读块切开，
        // 旧实现每块独立 lossy 会产生 U+FFFD；carry 重组后应无损
        let s = "中".repeat(6000);
        let bytes = s.as_bytes();
        let mut carry = Vec::new();
        let mut out = String::new();
        for chunk in bytes.chunks(4096) {
            out.push_str(&take_complete_utf8(&mut carry, chunk));
        }
        out.push_str(&flush_carry(&mut carry));
        assert_eq!(out, s);
        assert!(!out.contains('\u{FFFD}'), "不应出现替换字符");
    }

    #[test]
    fn has_shebang_only_for_script_files() {
        let dir = std::env::temp_dir().join(format!("gws_shebang_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("gws");
        std::fs::write(&script, "#!/usr/bin/env bash\n").unwrap();
        assert!(has_shebang(&script));
        let binary = dir.join("gws.bin");
        std::fs::write(&binary, b"MZ\x90\x00").unwrap();
        assert!(!has_shebang(&binary));
        assert!(!has_shebang(&dir.join("missing")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn git_root_from_bash_walks_up_both_layouts() {
        let dir = std::env::temp_dir().join(format!("gws_gitroot_test_{}", std::process::id()));
        let root = dir.join("Git");
        for sub in ["bin", "usr/bin", "mingw64", "cmd"] {
            std::fs::create_dir_all(root.join(sub)).unwrap();
        }
        let bash_bin = root.join("bin").join("bash.exe");
        std::fs::write(&bash_bin, b"").unwrap();
        assert_eq!(git_root_from_bash(&bash_bin), Some(root.clone()));
        // usr\bin\bash.exe 布局：多一层向上也能找到
        let bash_usr = root.join("usr").join("bin").join("bash.exe");
        std::fs::write(&bash_usr, b"").unwrap();
        assert_eq!(git_root_from_bash(&bash_usr), Some(root.clone()));
        // 不含 usr/bin + mingw64 的目录树：向上走到根也不会误判
        let stray = dir.join("stray").join("bash.exe");
        std::fs::create_dir_all(stray.parent().unwrap()).unwrap();
        std::fs::write(&stray, b"").unwrap();
        assert_eq!(git_root_from_bash(&stray), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn command_assembles_program_prefix_args_and_path() {
        let launch = GwsLaunch {
            program: PathBuf::from("bash.exe"),
            prefix_args: vec!["--noprofile".into(), "--norc".into(), "C:/gws".into()],
            // path_append 须用无盘符冒号的路径：unix 的列表分隔符就是 `:`，
            // 带 `C:` 的假路径在 macOS 上会被 join_paths 拒绝（Windows 真实路径无此问题）
            path_append: vec![PathBuf::from("/opt/git/usr/bin"), PathBuf::from("/opt/git/mingw64/bin")],
        };
        let args = vec!["sync".to_string(), "dev".to_string()];
        let cmd = launch.command(&args, Path::new("C:/hub"));
        assert_eq!(cmd.get_program(), "bash.exe");
        assert_eq!(
            cmd.get_args().collect::<Vec<_>>(),
            ["--noprofile", "--norc", "C:/gws", "sync", "dev"]
        );
        assert_eq!(cmd.get_current_dir(), Some(Path::new("C:/hub").as_ref()));
        // PATH 只追加不清空：原有条目仍在，追加目录在尾部
        let path_val = cmd
            .get_envs()
            .find(|(k, _)| *k == std::ffi::OsStr::new("PATH"))
            .and_then(|(_, v)| v)
            .expect("path_append 非空时应设置 PATH");
        let paths: Vec<PathBuf> = std::env::split_paths(path_val).collect();
        assert!(
            paths.contains(&PathBuf::from("/opt/git/usr/bin")),
            "追加目录应在 PATH 中: {path_val:?}"
        );
        let last = paths.last().unwrap();
        assert_eq!(*last, PathBuf::from("/opt/git/mingw64/bin"), "追加目录应在 PATH 尾部");
        // 空追加目录不应碰 PATH
        let plain = GwsLaunch::direct(PathBuf::from("gws"));
        let cmd2 = plain.command(&args, Path::new("C:/hub"));
        assert!(
            cmd2.get_envs().all(|(k, _)| k != std::ffi::OsStr::new("PATH")),
            "无追加目录时不应显式设置 PATH"
        );
    }
}
