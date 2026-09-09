//! `px info <pkg>` — merged info from every source that knows the package.

use crate::app::App;
use crate::error::PxResult;
use crate::ui::spinner::Spinners;

pub async fn run(app: App, name: &str) -> PxResult<()> {
    let style = &app.style;
    println!("{}", style.banner());

    let providers = app.providers();
    let spinners = Spinners::new(!app.cli.no_color);

    let mut handles = Vec::new();
    for p in &providers {
        let p = std::sync::Arc::clone(p);
        let name = name.to_string();
        let pb = spinners.add(&format!("asking {}", style.bold(p.label())));
        handles.push(tokio::spawn(async move {
            let hit = p.info(&name).await.ok().flatten();
            (p, hit, pb)
        }));
    }

    let mut any = false;
    for h in handles {
        let (p, hit, pb) = h.await.expect("info task panicked");
        let label = p.label().to_string();
        match hit {
            Some(hit) => {
                any = true;
                crate::ui::spinner::finish_ok(
                    &pb,
                    format!("{label}: {} {}", hit.name, hit.version),
                );
                if let Some(d) = &hit.description {
                    println!("    {}", style.dim(d));
                }
            }
            None => {
                crate::ui::spinner::finish_warn(&pb, format!("{label}: not found"));
            }
        }
    }

    if !any {
        return Err(crate::error::PxError::NotFound(format!(
            "no source knows '{name}'"
        )));
    }
    Ok(())
}
