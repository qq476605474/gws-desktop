use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use gws_desk_lib::gws_runner::{
    cleanup_if_done, find_gws, respond_confirm, run_gws_once, PendingEvent, RunShared, RUNS,
};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gws_runner_test_{}_{}", name, std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_mock(dir: &Path, name: &str, script: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, script).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
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
}

#[test]
fn find_gws_resolves_path() {
    let dir = temp_dir("find_gws");
    let gws = write_mock(&dir, "gws", "#!/bin/bash\nexit 0\n");
    let old_path = std::env::var_os("PATH");
    let mut paths = vec![dir.clone()];
    if let Some(p) = &old_path {
        paths.extend(std::env::split_paths(p));
    }
    std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
    let found = find_gws();
    match old_path {
        Some(p) => std::env::set_var("PATH", p),
        None => std::env::remove_var("PATH"),
    }
    assert_eq!(found, Some(gws));
}

#[test]
fn record_buffers_until_start_then_direct() {
    let mut st = RunShared { child: None, stdin: None, started: false, finished: false, pending: Vec::new() };
    assert!(st.record(PendingEvent::Output("a".into())).is_none());
    assert!(st.record(PendingEvent::Confirm("q".into())).is_none());
    assert_eq!(st.pending.len(), 2);

    let drained = st.start();
    assert_eq!(drained.len(), 2);
    assert!(matches!(&drained[0], PendingEvent::Output(s) if s == "a"), "顺序应保持: {:?}", drained.len());
    assert!(matches!(&drained[1], PendingEvent::Confirm(q) if q == "q"));

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

    RUNS.lock().unwrap().insert(
        90001,
        RunShared { child: Some(child), stdin: Some(stdin), started: false, finished: false, pending: Vec::new() },
    );

    respond_confirm(90001, true).unwrap();

    let mut out = String::new();
    stdout.read_to_string(&mut out).unwrap();
    assert!(out.contains("got:y"), "output: {}", out);

    let mut child = RUNS.lock().unwrap().get_mut(&90001).unwrap().child.take().unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0));
    RUNS.lock().unwrap().remove(&90001);
}

#[test]
fn respond_confirm_no_kills_process() {
    assert_eq!(respond_confirm(99999, true).unwrap_err(), "run 不存在");

    let dir = temp_dir("confirm_no");
    let exe = write_mock(&dir, "mock.sh", "#!/bin/bash\nsleep 30\n");
    let mut child = Command::new(&exe).stdin(Stdio::piped()).spawn().unwrap();
    let stdin = child.stdin.take().unwrap();
    RUNS.lock().unwrap().insert(
        90002,
        RunShared { child: Some(child), stdin: Some(stdin), started: false, finished: false, pending: Vec::new() },
    );

    respond_confirm(90002, false).unwrap();

    let mut child = RUNS.lock().unwrap().get_mut(&90002).unwrap().child.take().unwrap();
    let status = child.wait().unwrap();
    assert!(status.code().is_none(), "被信号杀死的进程 code 应为 None: {:?}", status);
    RUNS.lock().unwrap().remove(&90002);

    // child 已被 waiter take 走（None）时 kill 视为成功
    RUNS.lock().unwrap().insert(
        90004,
        RunShared { child: None, stdin: None, started: false, finished: false, pending: Vec::new() },
    );
    respond_confirm(90004, false).unwrap();
    RUNS.lock().unwrap().remove(&90004);
}

#[test]
fn finished_and_started_run_is_removed() {
    let mut st = RunShared { child: None, stdin: None, started: false, finished: true, pending: Vec::new() };
    st.pending.push(PendingEvent::Exit(Some(0)));
    RUNS.lock().unwrap().insert(90003, st);

    // replay_output 核心：切直发并取回缓存 → started && finished → 清理
    let drained = {
        let mut runs = RUNS.lock().unwrap();
        let drained = runs.get_mut(&90003).map(|st| st.start()).unwrap_or_default();
        cleanup_if_done(&mut runs, 90003);
        drained
    };
    assert_eq!(drained.len(), 1);
    assert!(matches!(drained[0], PendingEvent::Exit(Some(0))));
    assert!(!RUNS.lock().unwrap().contains_key(&90003), "started+finished 的 run 应被移除");

    // 对照：finished 但前端从未订阅（未 start）→ 保留等待回放
    let st = RunShared { child: None, stdin: None, started: false, finished: true, pending: Vec::new() };
    RUNS.lock().unwrap().insert(90005, st);
    {
        let mut runs = RUNS.lock().unwrap();
        cleanup_if_done(&mut runs, 90005);
        assert!(runs.contains_key(&90005), "未 start 的 run 不应被清理");
        runs.remove(&90005);
    }
}
