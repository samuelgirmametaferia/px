use owo_colors::OwoColorize;

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

    /// The px banner. Multicolor, as promised.
    pub fn banner(&self) -> String {
        if !self.on {
            return "px".into();
        }
        format!(
            "{}{}  {}",
            "p".bold().bright_magenta(),
            "x".bold().bright_cyan(),
            "· the package manager front-end".dimmed()
        )
    }
}
