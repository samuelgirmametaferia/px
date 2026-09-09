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

/// Pad a (possibly styled) string with spaces to a visible width, or
/// truncate it (with an ANSI reset) if it's already longer.
fn pad_to(line: &str, width: usize) -> String {
    let len = visible_len(line);
    if len >= width {
        return truncate_visible(line, width);
    }
    format!("{line}{}", " ".repeat(width - len))
}

/// Cut a styled string at a visible width; appends a reset so any open
/// color can't bleed into the border.
fn truncate_visible(line: &str, width: usize) -> String {
    let mut out = String::new();
    let mut len = 0usize;
    let mut in_esc = false;
    for c in line.chars() {
        if in_esc {
            out.push(c);
            if c == 'm' {
                in_esc = false;
            }
            continue;
        }
        if c == '\x1b' {
            out.push(c);
            in_esc = true;
            continue;
        }
        if len >= width {
            return format!("{out}\x1b[0m");
        }
        out.push(c);
        len += 1;
    }
    out
}

/// Box-drawing panel: a titled section used in install plans.
///   ╭─ install plan ────────────────────╮
///   │ ripgrep 14.1.0           (repo)   │
///   ╰───────────────────────────────────╯
/// Widths are measured by VISIBLE length so styled lines stay aligned.
/// `width` is the content width; every row renders exactly w+4 cells.
pub fn panel(title: &str, lines: &[String], width: usize) -> String {
    // content width must fit the widest line AND the title segment
    let w = width
        .max(visible_len(&format!("─ {title} ")))
        .max(20);
    let inner = w + 2; // " content "
    let mut out = String::new();
    let title_seg = format!("─ {title} ");
    let dashes = inner.saturating_sub(visible_len(&title_seg));
    out.push_str(&format!("╭{title_seg}{}╮\n", "─".repeat(dashes)));
    for line in lines {
        out.push_str(&format!("│ {} │\n", pad_to(line, w)));
    }
    out.push_str(&format!("╰{}╯\n", "─".repeat(inner)));
    out.pop(); // trailing newline
    out
}
