//! `px list` — packages px itself installed.

use crate::app::App;
use crate::error::PxResult;

pub fn run(app: App) -> PxResult<()> {
    let style = &app.style;
    let ledger = crate::ledger::Ledger::load();
    if ledger.entries.is_empty() {
        println!("{}", style.dim("px hasn't installed anything yet"));
        return Ok(());
    }
    let mut table = comfy_table::Table::new();
    table.load_preset(comfy_table::presets::UTF8_FULL);
    table.set_header(vec!["package", "source", "installed at"]);
    let mut entries = ledger.entries.clone();
    entries.sort_by_key(|e| std::cmp::Reverse(e.installed_at));
    for e in entries {
        table.add_row(vec![
            style.by_source(&e.source, &e.name),
            style.dim(&e.source),
            style.dim(&e.installed_at.to_rfc3339()),
        ]);
    }
    println!("{table}");
    Ok(())
}
