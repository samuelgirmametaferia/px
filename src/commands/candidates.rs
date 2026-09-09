//! `px candidates` — the repos the GitHub discovery system has found.
//! Reads the `candidates` branch the registry workflow publishes after
//! every discovery run (registry-candidates.jsonl), with graceful handling
//! when discovery hasn't run or found nothing yet.

use crate::app::App;
use crate::error::{PxError, PxResult};

const CANDIDATES_URL: &str = "https://raw.githubusercontent.com/samuelgirmametaferia/px/candidates/candidates.jsonl";

#[derive(serde::Deserialize)]
struct Candidate {
    canonical_id: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    expected_binaries: Vec<String>,
}

pub async fn run(app: App) -> PxResult<()> {
    let style = &app.style;
    println!("{}", style.banner());

    // raw.githubusercontent first, then the GitHub API (raw has flaky
    // CDN edges; the API is a different route that stays up)
    let mut text = None;
    match app.client.get(CANDIDATES_URL).send().await {
        Ok(r) if r.status().is_success() => text = Some(r.text().await.unwrap_or_default()),
        Ok(r) if r.status() == 404 => {}
        _ => {}
    }
    let text = match text {
        Some(t) => t,
        None => {
            let api = "https://api.github.com/repos/samuelgirmametaferia/px/contents/registry-candidates.jsonl?ref=candidates";
            match app
                .client
                .get(api)
                .header("Accept", "application/vnd.github.raw")
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
                Ok(r) if r.status() == 404 => {
                    println!(
                        "  {} discovery hasn't published candidates yet — the registry workflow",
                        style.warn("⚠")
                    );
                    println!(
                        "  {} runs every 6h (or dispatch it: github.com/samuelgirmametaferia/px/actions)",
                        style.dim("·")
                    );
                    return Ok(());
                }
                Ok(r) => {
                    return Err(PxError::User(format!(
                        "candidates fetch returned HTTP {}",
                        r.status()
                    )));
                }
                Err(e) => return Err(PxError::Network(e)),
            }
        }
    };

    // last-discovery marker rides on the same branch
    let last = app
        .client
        .get("https://api.github.com/repos/samuelgirmametaferia/px/contents/.last-discovery?ref=candidates")
        .header("Accept", "application/vnd.github.raw")
        .send()
        .await;
    if let Ok(r) = last
        && r.status().is_success()
    {
        let ts = r.text().await.unwrap_or_default();
        println!(
            "  {} discovery run: {}",
            style.dim("·"),
            style.dim(ts.trim())
        );
    }

    let cands: Vec<Candidate> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    if cands.is_empty() {
        println!(
            "  {} discovery found nothing in the last run",
            style.warn("○")
        );
        return Ok(());
    }

    println!(
        "\n  {} {} repo(s) the discovery system found:\n",
        style.ok("✓"),
        cands.len()
    );
    for c in &cands {
        let bins = if c.expected_binaries.is_empty() {
            String::new()
        } else {
            style.dim(&format!("  bin: {}", c.expected_binaries.join(", ")))
        };
        println!(
            "  {} {} {}",
            style.src("◈"),
            style.bold(&c.canonical_id),
            bins
        );
        if !c.description.is_empty() {
            println!("      {}", style.dim(&c.description));
        }
    }
    println!(
        "\n  {} candidates are validated by the sandbox job before entering the registry",
        style.dim("·")
    );
    Ok(())
}
