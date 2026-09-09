//! `px list` — packages px itself installed.

use comfy_table::{Cell, Color, ContentArrangement, Table, presets::UTF8_FULL};

use crate::app::App;
use crate::error::PxResult;

pub fn run(app: App) -> PxResult<()> {
    let style = &app.style;
    let ledger = crate::ledger::Ledger::load();
    if ledger.entries.is_empty() {
        println!("{}", style.dim("px hasn't installed anything yet"));
        return Ok(());
    }
    let colors_on = style.colors_on();
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_width(crate::ui::term_width() as u16);
    table.set_header(vec!["package", "source", "installed at"]);
    let mut entries = ledger.entries.clone();
    entries.sort_by_key(|e| std::cmp::Reverse(e.installed_at));
    for e in entries {
        let mut name = Cell::new(&e.name);
        let mut source = Cell::new(&e.source);
        if colors_on {
            let color = match e.source.as_str() {
                "repo" | "pacman" | "apt" | "dnf" | "zypper" => Color::Green,
                "aur" | "paru" | "yay" => Color::Magenta,
                _ => Color::Yellow,
            };
            name = name.fg(color);
            source = source.fg(color);
        }
        table.add_row(vec![
            name,
            source,
            Cell::new(e.installed_at.to_rfc3339()).fg(Color::DarkGrey),
        ]);
    }
    println!("{table}");
    println!(
        "\n{} {} package(s) installed by px",
        style.dim("·"),
        ledger.entries.len()
    );
    Ok(())
}
