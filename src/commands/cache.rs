//! `px cache clean`.

use crate::app::App;
use crate::error::{PxError, PxResult};

pub fn clean(app: App, search: bool, recipes: bool, builds: bool, all: bool) -> PxResult<()> {
    let style = &app.style;
    let mut cleaned: Vec<&str> = Vec::new();
    if all {
        crate::cache::clean(None).map_err(|e| PxError::User(e.to_string()))?;
        cleaned.push("everything");
    } else {
        for (flag, ns) in [(search, "search"), (recipes, "recipes"), (builds, "builds")] {
            if flag {
                crate::cache::clean(Some(ns)).map_err(|e| PxError::User(e.to_string()))?;
                cleaned.push(ns);
            }
        }
    }
    if cleaned.is_empty() {
        return Err(PxError::User(
            "specify --search, --recipes, --builds or --all".into(),
        ));
    }
    println!("{} cleaned {}", style.ok("✔"), cleaned.join(", "));
    Ok(())
}
