//! The first-run spectrum animation: a rainbow-gradient px banner sweep.
//! Runs once after install (tracked in px's state dir), only on a tty.

use std::io::Write;
use std::time::Duration;

const FRAMES: usize = 36;

pub fn state_marker() -> std::path::PathBuf {
    crate::state::state_dir().join("welcomed")
}

/// True when this is the first interactive px run on this machine.
pub fn should_run() -> bool {
    !state_marker().exists() && std::io::IsTerminal::is_terminal(&std::io::stdout())
}

/// True-color rainbow gradient across the px wordmark, animated as a
/// traveling wave. Degrades to the plain banner without truecolor.
pub fn play() {
    let banner = "  p x";
    let sub = "· the package manager front-end";
    let mut out = std::io::stdout();

    for frame in 0..FRAMES {
        let mut line = String::new();
        for (i, ch) in banner.chars().enumerate() {
            let hue = ((frame * 10 + i * 42) % 360) as u32;
            line.push_str(&format!("\x1b[1m{}\x1b[0m", rgb(hue, ch)));
        }
        let _ = write!(out, "\r\x1b[2K{line}  \x1b[2m{sub}\x1b[0m");
        let _ = out.flush();
        std::thread::sleep(Duration::from_millis(38));
    }
    // settle on a fixed pretty frame
    let _ = write!(out, "\r\x1b[2K");
    let _ = writeln!(
        out,
        "  {}  \x1b[2m{sub}\x1b[0m",
        banner
            .chars()
            .enumerate()
            .map(|(i, ch)| format!("\x1b[1m{}\x1b[0m", rgb((i * 70) as u32, ch)))
            .collect::<String>()
    );
    let _ = writeln!(
        out,
        "  \x1b[2mwelcome to px — run `px --tutorial` to learn it in 2 minutes\x1b[0m"
    );
    let _ = out.flush();

    let _ = std::fs::write(state_marker(), b"");
}

/// hue (0-359) → ansi truecolor char wrapper.
fn rgb(hue: u32, ch: char) -> String {
    let (r, g, b) = hsl_to_rgb(hue, 0.85, 0.6);
    format!("\x1b[38;2;{r};{g};{b}m{ch}\x1b[0m")
}

fn hsl_to_rgb(h: u32, s: f64, l: f64) -> (u8, u8, u8) {
    let h = h as f64 / 360.0;
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let to = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0) as u8
    };
    (to(h + 1.0 / 3.0), to(h), to(h - 1.0 / 3.0))
}
