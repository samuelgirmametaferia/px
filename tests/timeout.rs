//! Hang prevention: every query has a hard ceiling, and children die with
//! px. These tests pin that behavior with actually-slow commands.

use px::error::PxError;
use px::exec::{ExecOutput, Executor, RealExecutor, RunOpts};
use std::time::{Duration, Instant};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn query_timeout_kills_the_child_and_returns() {
    let exec = RealExecutor::new();
    let start = Instant::now();
    let result = rt().block_on(exec.run(
        &["sleep".to_string(), "30".to_string()],
        RunOpts {
            timeout: Some(Duration::from_secs(1)),
            ..Default::default()
        },
    ));
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "timeout must return promptly, took {elapsed:?}"
    );
    match result {
        Err(PxError::Timeout(msg)) => {
            assert!(msg.contains("timed out"), "message: {msg}");
        }
        other => panic!("expected Timeout, got {other:?}"),
    }
}

#[test]
fn no_timeout_means_no_ceiling() {
    // a fast command with no timeout works normally
    let exec = RealExecutor::new();
    let out = rt()
        .block_on(exec.run(
            &["echo".to_string(), "hello".to_string()],
            RunOpts::default(),
        ))
        .unwrap();
    assert_eq!(out.stdout.trim(), "hello");
}

#[test]
fn dry_run_returns_instantly() {
    let exec = RealExecutor::new();
    let start = Instant::now();
    let out = rt()
        .block_on(exec.run(
            &["sleep".to_string(), "60".to_string()],
            RunOpts {
                dry_run: true,
                ..Default::default()
            },
        ))
        .unwrap();
    assert!(out.success());
    assert!(start.elapsed() < Duration::from_millis(500));
}

#[test]
fn child_is_killed_when_the_future_is_dropped() {
    // kill_on_drop: abandoning the wait must not leak a sleeping child.
    let exec = RealExecutor::new();
    let start = Instant::now();
    // simulate: start the run, drop it after 300ms via timeout(300ms)
    let handle = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let argv: Vec<String> = vec!["sleep".to_string(), "60".to_string()];
    handle.block_on(async {
        let fut = exec.run(
            &argv,
            RunOpts {
                // no px-level timeout — the outer timeout drops the future,
                // which must kill the child via kill_on_drop
                ..Default::default()
            },
        );
        let _ = tokio::time::timeout(Duration::from_millis(300), fut).await;
    });
    assert!(start.elapsed() < Duration::from_secs(2));
    // give the child a moment to die, then confirm no `sleep 60` remains
    std::thread::sleep(Duration::from_millis(300));
    let leaked = std::process::Command::new("pgrep")
        .args(["-f", "sleep 60"])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    assert!(!leaked, "a killed run must not leave `sleep 60` behind");
}

// ExecOutput must stay constructible for MockExecutor users.
#[test]
fn exec_output_shape() {
    let out = ExecOutput {
        status: 0,
        stdout: String::new(),
        stderr: String::new(),
    };
    assert!(out.success());
}
