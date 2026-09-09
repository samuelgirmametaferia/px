pub mod jokes;
pub mod progress;
pub mod prompt;
pub mod spectrum;
pub mod spinner;
pub mod style;
pub mod table;

/// Terminal width in columns (80 when unknown — pipes, redirects, CI).
/// Everything that draws (tables, panels, bars) scales to this.
pub fn term_width() -> usize {
    console::Term::stdout()
        .size_checked()
        .map(|(_, cols)| cols as usize)
        .unwrap_or(80)
        .max(40)
}

/// True when colors should be emitted: a tty and no NO_COLOR / --no-color.
pub fn colors_enabled(no_color_flag: bool) -> bool {
    if no_color_flag || std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    // owo-colors respects tty detection via `if_supports_color`; we gate our
    // helpers the same way for consistency in pipes and CI.
    std::io::IsTerminal::is_terminal(&std::io::stdout())
}

/// True when px can show interactive prompts (stdin is a tty).
pub fn interactive() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdin())
        && std::io::IsTerminal::is_terminal(&std::io::stdout())
}
