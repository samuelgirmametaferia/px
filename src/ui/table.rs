use comfy_table::{Table, presets::UTF8_FULL};

use crate::backend::PackageHit;

/// Shared table preset for search results and install plans.
pub fn hits_table(style: &crate::ui::style::Style, hits: &[PackageHit]) -> Table {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec!["package", "version", "source", "description"]);
    for hit in hits {
        table.add_row(vec![
            style.by_source(&hit.source, &hit.name),
            style.dim(&hit.version),
            style.by_source(&hit.source, &hit.source),
            hit.description
                .as_deref()
                .unwrap_or("")
                .chars()
                .take(60)
                .collect::<String>(),
        ]);
    }
    table
}

/// Visible length of a string (ANSI escape sequences count as zero).
pub fn visible_len(s: &str) -> usize {
    let mut len = 0usize;
    let mut in_esc = false;
    for c in s.chars() {
        if in_esc {
            if c == 'm' {
                in_esc = false;
            }
            continue;
        }
        if c == '\x1b' {
            in_esc = true;
            continue;
        }
        len += 1;
    }
    len
}

/// Pad a (possibly styled) string with spaces to a visible width.
fn pad_to(line: &str, width: usize) -> String {
    let pad = width.saturating_sub(visible_len(line));
    format!("{line}{}", " ".repeat(pad))
}

/// Box-drawing panel: a titled section used in install plans.
///   ╭─ install plan ────────────────────╮
///   │ ripgrep 14.1.0           (repo)   │
///   ╰───────────────────────────────────╯
/// Widths are measured by VISIBLE length so styled lines stay aligned.
pub fn panel(title: &str, lines: &[String], width: usize) -> String {
    let w = width.max(visible_len(title) + 4).max(20);
    let mut out = String::new();
    let title_seg = format!("─ {title} ");
    out.push_str(&format!("╭{title_seg:<width$}╮\n", width = w + 1));
    for line in lines {
        out.push_str(&format!("│ {} │\n", pad_to(line, w - 1)));
    }
    out.push_str(&format!("╰{:-<width$}╯\n", "", width = w + 1));
    out.pop(); // trailing newline
    out
}
