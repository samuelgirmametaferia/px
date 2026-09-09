use comfy_table::{Cell, Color, ContentArrangement, Table, presets::UTF8_FULL};

use crate::backend::PackageHit;

/// Map a source id to its palette color (the same mapping ui::style uses).
fn source_color(source: &str) -> Option<Color> {
    match source {
        "repo" | "pacman" | "apt" | "dnf" | "zypper" => Some(Color::Green),
        "aur" | "paru" | "yay" => Some(Color::Magenta),
        "github" | "source" | "app" => Some(Color::Yellow),
        _ => None,
    }
}

fn styled_cell(text: &str, color: Option<Color>, colors_on: bool) -> Cell {
    if colors_on {
        let mut cell = Cell::new(text);
        if let Some(c) = color {
            cell = cell.fg(c);
        }
        cell
    } else {
        Cell::new(text)
    }
}

/// Search results table. Scales to the terminal: comfy-table's dynamic
/// arrangement gives columns their natural width when there's room and
/// WRAPS content when there isn't — full descriptions are always shown,
/// never chopped to a fixed character count. Colors go through
/// comfy-table's own cell styling: raw ANSI in cells breaks its width
/// math and misaligns every column.
pub fn hits_table(style: &crate::ui::style::Style, hits: &[PackageHit]) -> Table {
    let colors_on = style.colors_on();
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_width(crate::ui::term_width() as u16);
    table.set_header(vec!["package", "version", "source", "description"]);
    for hit in hits {
        let color = source_color(&hit.source);
        table.add_row(vec![
            styled_cell(&hit.name, color, colors_on),
            styled_cell(&hit.version, Some(Color::DarkGrey), colors_on),
            styled_cell(&hit.source, color, colors_on),
            styled_cell(
                &hit.description.clone().unwrap_or_default(),
                None,
                colors_on,
            ),
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
    let mut w = width.max(visible_len(&format!("─ {title} "))).max(20);
    // ... and the terminal: a box wider than the screen wraps and breaks.
    w = w.min(crate::ui::term_width().saturating_sub(4));
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
