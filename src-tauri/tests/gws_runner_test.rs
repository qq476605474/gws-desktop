use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use gws_desk_lib::gws_runner::{
    cleanup_if_done, find_gws, lock_runs, replay_output, respond_confirm, run_gws_once,
    spawn_stream, PendingEvent, RunShared,
};
use tauri::{AppHandle, Emitter, Listener, Manager, Runtime};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gws_runner_test_{}_{}", name, std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_mock(dir: &Path, name: &str, script: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
    }
    path
}

/// 50ms 轮询等待条件成立，超时带上下文 panic。
fn wait_until<T>(what: &str, timeout: Duration, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(v) = probe() {
            return v;
        }
        assert!(Instant::now() < deadline, "等待 {what} 超时（{timeout:?}）");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[derive(serde::Deserialize)]
struct OutputPayload {
    chunk: String,
}

#[derive(serde::Deserialize)]
struct ExitPayload {
    code: Option<i32>,
}

#[derive(serde::Deserialize)]
struct ConfirmPayload {
    question: String,
}

/// 订阅 run 的三个事件并聚合为一份日志（mock runtime 下 emit 同步送达监听器）。
#[derive(Default)]
struct EventLog {
    output: String,
    /// None = 未收到 exit；Some(None) = 收到 exit 且 code 为 null（被信号杀死）
    exit_code: Option<Option<i32>>,
    confirm: Option<String>,
}

/// 前端 exec 的等价物：订阅 gws-output/gws-exit/gws-confirm 后才 replay_output。
fn attach<R: Runtime>(handle: &AppHandle<R>, run_id: u32) -> Arc<Mutex<EventLog>> {
    let log = Arc::new(Mutex::new(EventLog::default()));
    let out = log.clone();
    handle.listen_any(format!("gws-output:{run_id}"), move |e| {
        let p: OutputPayload = serde_json::from_str(e.payload()).unwrap();
        out.lock().unwrap().output.push_str(&p.chunk);
    });
    let exit = log.clone();
    handle.listen_any(format!("gws-exit:{run_id}"), move |e| {
        let p: ExitPayload = serde_json::from_str(e.payload()).unwrap();
        exit.lock().unwrap().exit_code = Some(p.code);
    });
    let confirm = log.clone();
    handle.listen_any(format!("gws-confirm:{run_id}"), move |e| {
        let p: ConfirmPayload = serde_json::from_str(e.payload()).unwrap();
        confirm.lock().unwrap().confirm = Some(p.question);
    });
    log
}

#[test]
fn run_once_captures_output_and_code() {
    let dir = temp_dir("run_once_capture");
    let exe = write_mock(&dir, "mock.sh", "#!/bin/bash\necho hello\necho err >&2\nexit 3\n");
    let res = run_gws_once(&exe, &[], &dir);
    assert_eq!(res.code, Some(3));
    assert!(res.output.contains("hello"));
    assert!(res.output.contains("err"));
    let stdout_pos = res.output.find("hello").unwrap();
    let stderr_pos = res.output.find("err").unwrap();
    assert!(stdout_pos < stderr_pos, "stdout 应拼在 stderr 前: {}", res.output);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn run_once_passes_args_and_cwd() {
    let dir = temp_dir("run_once_args");
    let exe = write_mock(&dir, "mock.sh", "#!/bin/bash\necho \"cwd=$(pwd -P) args=$*\"\n");
    let args: Vec<String> = vec!["a".into(), "b".into()];
    let res = run_gws_once(&exe, &args, &dir);
    assert_eq!(res.code, Some(0));
    assert!(res.output.contains("args=a b"), "output: {}", res.output);
    let canonical = fs::canonicalize(&dir).unwrap();
    assert!(
        res.output.contains(&format!("cwd={}", canonical.display())),
        "output: {}, expect cwd={}",
        res.output,
        canonical.display()
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn find_gws_returns_path_or_none_without_panicking() {
    // 不篡改进程级 PATH（与并行测试相互干扰）：只验证真实环境下查找不 panic、类型正确。
    // PATH 解析的命中逻辑由 spawn_stream 的 e2e 用显式 exe 路径覆盖。
    let found: Option<PathBuf> = find_gws();
    if let Some(path) = &found {
        assert!(path.is_file(), "find_gws 返回的应是文件: {}", path.display());
    }
}

#[test]
fn record_buffers_until_start_then_direct() {
    let mut st = RunShared { child: None, stdin: None, started: false, finished: false, pending: Vec::new() };
    assert!(st.record(PendingEvent::Output("a".into())).is_none());
    assert!(st.record(PendingEvent::Confirm("q".into())).is_none());
    assert_eq!(st.pending.len(), 2);

    let drained = st.start();
    assert_eq!(drained.len(), 2);
    assert!(matches!(&drained[0], PendingEvent::Output(s) if s == "a"), "顺序应保持: {drained:?}");
    assert!(matches!(&drained[1], PendingEvent::Confirm(q) if q == "q"), "顺序应保持: {drained:?}");

    // start 后进入直发模式：record 返回 Some 由调用方 emit
    assert!(st.record(PendingEvent::Exit(Some(0))).is_some());
    assert!(st.start().is_empty(), "重复 start 不应再吐出事件");
}

#[test]
fn respond_confirm_yes_writes_stdin() {
    let dir = temp_dir("confirm_yes");
    let exe = write_mock(&dir, "mock.sh", "#!/bin/bash\nread line\necho \"got:$line\"\n");
    let mut child = Command::new(&exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    lock_runs().insert(
        90001,
        RunShared { child: Some(child), stdin: Some(stdin), started: false, finished: false, pending: Vec::new() },
    );

    respond_confirm(90001, true).unwrap();

    let mut out = String::new();
    stdout.read_to_string(&mut out).unwrap();
    assert!(out.contains("got:y"), "output: {}", out);

    let mut child = lock_runs().get_mut(&90001).unwrap().child.take().unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0));
    lock_runs().remove(&90001);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn respond_confirm_no_kills_process() {
    assert_eq!(respond_confirm(99999, true).unwrap_err(), "run 不存在");

    let dir = temp_dir("confirm_no");
    // read 是 bash 内建：阻塞等 stdin，不依赖 PATH 查找外部 sleep
    let exe = write_mock(&dir, "mock.sh", "#!/bin/bash\nread line\n");
    let mut child = Command::new(&exe).stdin(Stdio::piped()).spawn().unwrap();
    let stdin = child.stdin.take().unwrap();
    lock_runs().insert(
        90002,
        RunShared { child: Some(child), stdin: Some(stdin), started: false, finished: false, pending: Vec::new() },
    );

    respond_confirm(90002, false).unwrap();

    let mut child = lock_runs().get_mut(&90002).unwrap().child.take().unwrap();
    let status = child.wait().unwrap();
    assert!(status.code().is_none(), "被信号杀死的进程 code 应为 None: {:?}", status);
    lock_runs().remove(&90002);
    fs::remove_dir_all(&dir).unwrap();

    // child 已被 waiter take 走（None）时 kill 视为成功
    lock_runs().insert(
        90004,
        RunShared { child: None, stdin: None, started: false, finished: false, pending: Vec::new() },
    );
    respond_confirm(90004, false).unwrap();
    lock_runs().remove(&90004);
}

#[test]
fn finished_and_started_run_is_removed() {
    let mut st = RunShared { child: None, stdin: None, started: false, finished: true, pending: Vec::new() };
    st.pending.push(PendingEvent::Exit(Some(0)));
    lock_runs().insert(90003, st);

    // replay_output 核心：切直发并取回缓存 → started && finished → 清理
    let drained = {
        let mut runs = lock_runs();
        let drained = runs.get_mut(&90003).map(|st| st.start()).unwrap_or_default();
        cleanup_if_done(&mut runs, 90003);
        drained
    };
    assert_eq!(drained.len(), 1);
    assert!(matches!(drained[0], PendingEvent::Exit(Some(0))));

    assert!(!lock_runs().contains_key(&90003), "started+finished 的 run 应被移除");

    // 对照：finished 但前端从未订阅（未 start）→ 保留等待回放
    let st = RunShared { child: None, stdin: None, started: false, finished: true, pending: Vec::new() };
    lock_runs().insert(90005, st);
    {
        let mut runs = lock_runs();
        cleanup_if_done(&mut runs, 90005);
        assert!(runs.contains_key(&90005), "未 start 的 run 不应被清理");
        runs.remove(&90005);
    }
}

/// 最小试验：验证 mock runtime 下 AppHandle::emit 能同步送达 listen_any 注册的监听器
/// （e2e 事件断言的可行性前提）。
#[test]
fn mock_runtime_events_reach_rust_listeners() {
    let app = tauri::test::mock_app();
    let handle = app.app_handle();
    let got = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = got.clone();
    handle.listen_any("exp-event", move |e| sink.lock().unwrap().push(e.payload().to_string()));
    handle.emit("exp-event", "ping").unwrap();
    assert_eq!(got.lock().unwrap().as_slice(), ["\"ping\""]);
}

#[test]
fn stream_e2e_output_exit_replay() {
    let app = tauri::test::mock_app();
    let handle = app.app_handle();
    let dir = temp_dir("e2e_output");
    // 5400 字节的中文行（> 4096 读块）验证读线程跨块 UTF-8 重组端到端无损
    let big_line = "中".repeat(1800);
    let script = format!(
        "#!/bin/bash\necho hello\nprintf '中文输出行\\n'\nprintf '%s\\n' '{}'\nexit 2\n",
        big_line
    );
    let exe = write_mock(&dir, "mock.sh", &script);

    // run_gws_stream 契约：spawn + 编排完成后立即返回 runId，不等进程退出
    let started = Instant::now();
    let run_id = spawn_stream(handle, &exe, &[], dir.to_str().unwrap(), None).unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "run_gws_stream 应立即返回，实际耗时 {:?}",
        started.elapsed()
    );

    // 前端契约：先订阅三个事件，再 replay_output 补发订阅前的缓存事件
    let log = attach(handle, run_id);
    replay_output(handle.clone(), run_id).unwrap();

    let exit = wait_until("exit 事件", Duration::from_secs(10), || {
        log.lock().unwrap().exit_code
    });
    assert_eq!(exit, Some(2));

    let out = log.lock().unwrap().output.clone();
    assert!(out.contains("hello\n"), "output: {out}");
    assert!(out.contains("中文输出行\n"), "output: {out}");
    assert!(out.contains(&big_line), ">4KB 中文长行应完整送达（实际收到 {} 字节）", out.len());
    assert!(!out.contains('\u{FFFD}'), "跨块劈裂不应产生替换字符: {out:?}");

    // exit 已直发送达且 started → run 应被清理
    assert!(!lock_runs().contains_key(&run_id), "exit 送达后 run 应被清理");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn stream_e2e_confirm_kill() {
    let app = tauri::test::mock_app();
    let handle = app.app_handle();
    let dir = temp_dir("e2e_confirm");
    // read 是 bash 内建：阻塞等 stdin，无 stdout 输出 → 触发 watchdog 确认
    let exe = write_mock(&dir, "mock.sh", "#!/bin/bash\nread line\necho \"got:$line\"\n");

    let run_id = spawn_stream(handle, &exe, &[], dir.to_str().unwrap(), None).unwrap();
    let log = attach(handle, run_id);
    replay_output(handle.clone(), run_id).unwrap();

    // watchdog：静默 1.5s、每 250ms 轮询 → ~2s 内应发出 gws-confirm
    let question = wait_until("confirm 事件", Duration::from_secs(5), || {
        log.lock().unwrap().confirm.clone()
    });
    assert!(question.contains("可能正在等待输入"), "question: {question}");
    assert!(question.contains("1.5s"), "默认超时文案应含 1.5s: {question}");

    // 拒绝 → 后端 kill 子进程
    respond_confirm(run_id, false).unwrap();

    let exit = wait_until("exit 事件", Duration::from_secs(10), || {
        log.lock().unwrap().exit_code
    });
    assert_eq!(exit, None, "被 kill 的进程 exit code 应为 null");

    assert!(!lock_runs().contains_key(&run_id), "exit 送达后 run 应被清理");
    fs::remove_dir_all(&dir).unwrap();
}

/// 自定义长超时：1.8s 静默 + 5s 超时 → 不误发 confirm。
/// 判别力：1.8s > 默认 1500ms——若 confirm_timeout_ms 参数未生效（回落默认），
/// 本测试必误发 confirm 而失败（旧版 0.2s 静默对默认阈值同样安全，参数被忽略也绿）。
/// 裕量：距默认 1500ms +300ms、距参数 5000ms -3.2s。
#[test]
fn stream_e2e_long_custom_timeout_no_false_confirm() {
    let app = tauri::test::mock_app();
    let handle = app.app_handle();
    let dir = temp_dir("e2e_timeout_long");
    let exe = write_mock(&dir, "mock.sh", "#!/bin/bash\nsleep 1.8\necho done\n");

    let run_id = spawn_stream(handle, &exe, &[], dir.to_str().unwrap(), Some(5000)).unwrap();
    let log = attach(handle, run_id);
    replay_output(handle.clone(), run_id).unwrap();

    let exit = wait_until("exit 事件", Duration::from_secs(10), || {
        log.lock().unwrap().exit_code
    });
    assert_eq!(exit, Some(0));
    // exit 送达后 Confirm 不可能再入队（与 Exit 的临界区互斥）→ 此刻无 confirm 即永无
    assert!(
        log.lock().unwrap().confirm.is_none(),
        "1.8s 静默 + 5s 超时不应误发 confirm（若见 confirm 说明超时参数未生效），question: {:?}",
        log.lock().unwrap().confirm
    );
    assert!(log.lock().unwrap().output.contains("done"));

    assert!(!lock_runs().contains_key(&run_id), "exit 送达后 run 应被清理");
    fs::remove_dir_all(&dir).unwrap();
}

/// 自定义短超时：静默 1s + Some(100)（被 clamp 到下限 250ms）→ 发出 confirm 且文案带 "0.25s"。
/// 顺带端到端验证 clamp 下限生效（未 clamp 的 100ms 与 clamp 后的 250ms 轮询粒度下同拍触发，
/// 但文案 0.25s 能证明生效的是 clamp 后的值）。
#[test]
fn stream_e2e_short_custom_timeout_fires_confirm() {
    let app = tauri::test::mock_app();
    let handle = app.app_handle();
    let dir = temp_dir("e2e_timeout_short");
    // sleep 1s：静默期跨越 watchdog 首次轮询（250ms），保证 confirm 先于退出确定性发出
    let exe = write_mock(&dir, "mock.sh", "#!/bin/bash\nsleep 1\necho done\n");

    let run_id = spawn_stream(handle, &exe, &[], dir.to_str().unwrap(), Some(100)).unwrap();
    let log = attach(handle, run_id);
    replay_output(handle.clone(), run_id).unwrap();

    let question = wait_until("confirm 事件", Duration::from_secs(5), || {
        log.lock().unwrap().confirm.clone()
    });
    assert!(question.contains("可能正在等待输入"), "question: {question}");
    assert!(question.contains("0.25s"), "超时文案应为 clamp 后的 0.25s: {question}");

    // 进程未被杀：自然退出 code 0（confirm 只是提示，不影响运行）
    let exit = wait_until("exit 事件", Duration::from_secs(10), || {
        log.lock().unwrap().exit_code
    });
    assert_eq!(exit, Some(0));
    assert!(!lock_runs().contains_key(&run_id), "exit 送达后 run 应被清理");
    fs::remove_dir_all(&dir).unwrap();
}
