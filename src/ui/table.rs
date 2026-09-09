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

/// Box-drawing panel: a titled section used in install plans.
///   ╭─ Packages to install ─────────────╮
///   │ ripgrep 14.1.0           (repo)   │
///   ╰───────────────────────────────────╯
pub fn panel(title: &str, lines: &[String], width: usize) -> String {
    let w = width.max(title.len() + 4).max(20);
    let mut out = String::new();
    let title_seg = format!("─ {title} ");
    out.push_str(&format!("╭{title_seg:<width$}╮\n", width = w + 1));
    for line in lines {
        // Truncate over-long lines (ANSI codes make measuring unreliable, so
        // we just cap content length before styling at call sites).
        out.push_str(&format!("│ {line:<width$} │\n", width = w - 1));
    }
    out.push_str(&format!("╰{:-<width$}╯\n", "", width = w + 1));
    out.pop(); // trailing newline
    out
}
