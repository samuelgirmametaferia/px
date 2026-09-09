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

/// Watch network RX while an install runs and drive a progress bar.
///
/// - `Some(total)`: position = credited bytes, clamped at the known total
///   → a real percentage.
/// - `None`: total unknown (AUR builds report no download size) → position
///   is an open-ended byte counter, so the bar visibly climbs during the
///   download instead of freezing until completion.
///
/// Ambient traffic is discounted by the noise floor either way. Returns a
/// handle; call `.abort()` when the install ends. `credited` is updated
/// live so callers can report the downloaded total in their summary.
pub fn spawn_monitor(
    bar: indicatif::ProgressBar,
    total_bytes: Option<u64>,
    baseline_rx: u64,
    credited: std::sync::Arc<std::sync::atomic::AtomicU64>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last = baseline_rx;
        let mut seen: u64 = 0;
        let mut samples: u32 = 0;
        let total = total_bytes.unwrap_or(u64::MAX);
        loop {
            tokio::time::sleep(Duration::from_millis(300)).await;
            samples += 1;
            // long installs deserve fresh jokes: rotate every ~15s
            if samples % 50 == 0 {
                bar.set_message(crate::ui::jokes::next());
            }
            let Some(now) = total_rx_bytes() else {
                continue;
            };
            let delta = now.saturating_sub(last);
            last = now;
            // subtract the noise floor for this sampling window
            let window = Duration::from_millis(300).as_secs_f64();
            let noise = (NOISE_FLOOR_BPS as f64 * window) as u64;
            let real = delta.saturating_sub(noise);
            seen = (seen + real).min(total);
            credited.store(seen, std::sync::atomic::Ordering::Relaxed);
            bar.set_position(seen);
            if total_bytes.is_some() && seen >= total {
                break;
            }
        }
    })
}
