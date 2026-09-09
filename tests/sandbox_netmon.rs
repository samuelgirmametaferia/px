//! Sandbox + network-monitor tests. The sandbox test asserts the bwrap
//! argument SHAPE (the live behavior — /usr read-only — was verified on
//! this machine; see docs/manual-tests.md).

#[test]
fn sandbox_wrap_argv_shape() {
    let argv = vec!["npm".to_string(), "install".to_string(), "-g".to_string()];
    let wrapped = px::sandbox::wrap_argv(&argv, &["/tmp/build".to_string()]);
    let joined = wrapped.join(" ");
    // system read-only, fresh proc/dev/tmp, isolated namespaces
    assert!(joined.contains("--ro-bind / /"), "{joined}");
    assert!(joined.contains("--proc /proc"), "{joined}");
    assert!(joined.contains("--dev /dev"), "{joined}");
    assert!(joined.contains("--tmpfs /tmp"), "{joined}");
    assert!(joined.contains("--unshare-ipc"), "{joined}");
    assert!(joined.contains("--unshare-pid"), "{joined}");
    // home writable, extra dir writable
    assert!(
        joined.contains("--bind /home"),
        "home must be writable: {joined}"
    );
    assert!(joined.contains("--bind /tmp/build /tmp/build"), "{joined}");
    // the command itself is the tail
    assert!(joined.ends_with("npm install -g"), "{joined}");
}

#[test]
fn sandbox_mode_parses() {
    assert_eq!(
        px::sandbox::SandboxMode::parse("auto"),
        Some(px::sandbox::SandboxMode::Auto)
    );
    assert_eq!(
        px::sandbox::SandboxMode::parse("on"),
        Some(px::sandbox::SandboxMode::On)
    );
    assert_eq!(
        px::sandbox::SandboxMode::parse("off"),
        Some(px::sandbox::SandboxMode::Off)
    );
    assert_eq!(px::sandbox::SandboxMode::parse("bogus"), None);
}

#[cfg(target_os = "linux")]
#[test]
fn netmon_reads_interface_counters() {
    let rx = px::netmon::total_rx_bytes().expect("/proc/net/dev must parse");
    // loopback excluded, so this is real interface bytes (may be 0 on an
    // idle airgapped box, but the parse must succeed and be sane)
    assert!(rx < u64::MAX / 2);
}

#[cfg(target_os = "linux")]
#[test]
fn netmon_monitor_advances_on_traffic() {
    // End-to-end-ish: run the monitor against a tiny download and confirm
    // the bar position moves. Skipped silently when offline.
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let baseline = match px::netmon::total_rx_bytes() {
            Some(b) => b,
            None => return,
        };
        let bar = indicatif::ProgressBar::new(512 * 1024);
        let monitor = px::netmon::spawn_monitor(bar.clone(), Some(512 * 1024), baseline);
        // generate some RX (tiny request); ignore failure when offline
        let _ = reqwest::Client::new()
            .get("https://registry.npmjs.org/express")
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        monitor.abort();
        assert!(
            bar.position() > 0,
            "monitor should credit received bytes (position={})",
            bar.position()
        );
    });
}
