use owo_colors::OwoColorize;

/// Truecolor character at an HSL hue (sat 0.8, light 0.62) — the aurora.
fn truecolor(hue: f64, ch: char) -> String {
    let (r, g, b) = hsl_to_rgb(hue, 0.8, 0.62);
    format!("\x1b[1;38;2;{r};{g};{b}m{ch}\x1b[0m")
}

/// Dimmed variant for the tagline (light 0.55, sat 0.5).
fn dim_truecolor(hue: f64, ch: char) -> String {
    let (r, g, b) = hsl_to_rgb(hue, 0.5, 0.55);
    format!("\x1b[38;2;{r};{g};{b}m{ch}\x1b[0m")
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let h = h / 360.0;
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

/// The px palette, defined once. Every colored string in the binary goes
/// through one of these helpers so the look stays consistent and can be
/// disabled globally (NO_COLOR / --no-color / non-tty) in one place.
pub struct Style {
    on: bool,
}

impl Style {
    pub fn new(on: bool) -> Self {
        Style { on }
    }

    /// Whether to emit colors (drives comfy-table cell styling too).
    pub fn colors_on(&self) -> bool {
        self.on
    }

    fn wrap<'a>(&self, s: &'a str, f: impl Fn(&'a str) -> String) -> String {
        if self.on { f(s) } else { s.to_string() }
    }

    // ---- brand / structure --------------------------------------------
    pub fn brand(&self, s: &str) -> String {
        self.wrap(s, |s| s.bold().bright_cyan().to_string())
    }
    pub fn header(&self, s: &str) -> String {
        self.wrap(s, |s| s.bold().bright_cyan().to_string())
    }
    pub fn dim(&self, s: &str) -> String {
        self.wrap(s, |s| s.dimmed().to_string())
    }
    pub fn bold(&self, s: &str) -> String {
        self.wrap(s, |s| s.bold().to_string())
    }

    // ---- semantic colors ----------------------------------------------
    pub fn repo(&self, s: &str) -> String {
        self.wrap(s, |s| s.green().to_string())
    }
    pub fn aur(&self, s: &str) -> String {
        self.wrap(s, |s| s.magenta().to_string())
    }
    pub fn extra(&self, s: &str) -> String {
        self.wrap(s, |s| s.magenta().to_string())
    }
    pub fn src(&self, s: &str) -> String {
        self.wrap(s, |s| s.yellow().to_string())
    }
    pub fn ok(&self, s: &str) -> String {
        self.wrap(s, |s| s.green().to_string())
    }
    pub fn warn(&self, s: &str) -> String {
        self.wrap(s, |s| s.yellow().to_string())
    }
    pub fn err(&self, s: &str) -> String {
        self.wrap(s, |s| s.bold().red().to_string())
    }
    pub fn value(&self, s: &str) -> String {
        self.wrap(s, |s| s.bright_white().to_string())
    }

    /// Color a package name by the source it came from.
    pub fn by_source(&self, source: &str, s: &str) -> String {
        match source {
            "repo" | "pacman" | "apt" | "dnf" => self.repo(s),
            "aur" | "paru" | "yay" => self.aur(s),
            "github" | "source" => self.src(s),
            _ => self.extra(s),
        }
    }

    /// The px banner: an aurora gradient sweeping across the wordmark and
    /// tagline — every run, not just the first. NO_COLOR falls back to plain.
    /// When the first-run spectrum animation already played this run, stay
    /// quiet — one banner per invocation, not two.
    pub fn banner(&self) -> String {
        if crate::ui::spectrum::played_this_run() {
            return String::new();
        }
        if !self.on {
            return "px · the universal package manager".into();
        }
        let wordmark = "px";
        let tagline = "· the universal package manager";
        // aurora hues: violet → cyan → teal → green
        let hues = [280.0, 250.0, 200.0, 170.0, 140.0];
        let mut out = String::new();
        let wl = wordmark.len().max(1);
        for (i, ch) in wordmark.chars().enumerate() {
            let hue = hues[i * (hues.len() - 1) / (wl - 1).max(1)];
            out.push_str(&truecolor(hue, ch));
        }
        out.push_str("  ");
        let tl = tagline.len().max(1);
        for (i, ch) in tagline.chars().enumerate() {
            let hue = hues[(i + 1) * (hues.len() - 1) / tl.max(1)];
            out.push_str(&dim_truecolor(hue, ch));
        }
        out
    }
}
