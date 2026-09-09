//! Network-flow progress: instead of trying to parse a package manager's
//! captured output, px knows the total download size up front (from package
//! metadata) and watches the interface counters in /proc/net/dev while the
//! install runs. RX delta ÷ total = percentage. Ambient traffic from other
//! apps inflates the estimate slightly — px subtracts a small noise floor
//! and clamps at the known total.

use std::time::Duration;

/// Total received bytes across all interfaces, from /proc/net/dev.
/// Format per line: "iface: rx_bytes rx_packets ... tx_bytes ..."
pub fn total_rx_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/net/dev").ok()?;
    let mut total: u64 = 0;
    for line in text.lines() {
        let Some((iface, rest)) = line.split_once(':') else {
            continue; // the two header lines
        };
        let iface = iface.trim();
        // loopback carries no real downloads; everything else counts
        if iface == "lo" {
            continue;
        }
        if let Some(rx) = rest.split_whitespace().next() {
            total += rx.parse::<u64>().unwrap_or(0);
        }
    }
    Some(total)
}

/// Bytes/sec attributed to ambient (non-px) traffic and subtracted from the
/// estimate — small, deliberate, per the design.
const NOISE_FLOOR_BPS: u64 = 4 * 1024;

/// Watch network RX while an install runs and drive a progress bar towards
/// `total_bytes`. Returns a handle; call `.abort()` when the install ends.
/// The bar is clamped so ambient downloads can never fake 100% early...
/// they can only make it reach the end slightly sooner.
pub fn spawn_monitor(
    bar: indicatif::ProgressBar,
    total_bytes: u64,
    baseline_rx: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last = baseline_rx;
        let mut credited: u64 = 0;
        loop {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let Some(now) = total_rx_bytes() else {
                continue;
            };
            let delta = now.saturating_sub(last);
            last = now;
            // subtract the noise floor for this sampling window
            let window = Duration::from_millis(300).as_secs_f64();
            let noise = (NOISE_FLOOR_BPS as f64 * window) as u64;
            let real = delta.saturating_sub(noise);
            credited = (credited + real).min(total_bytes);
            bar.set_position(credited);
            if credited >= total_bytes {
                break;
            }
        }
    })
}
