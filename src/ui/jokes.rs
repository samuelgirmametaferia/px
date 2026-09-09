//! Jokes during installs. Fresh ones from the internet (icanhazdadjoke),
//! cached to disk; bundled fallbacks when offline. Cycling one into the
//! spinner keeps long installs company.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const BUNDLED: &[&str] = &[
    "why do programmers prefer dark mode? because light attracts bugs.",
    "there are only two hard things in package management: naming, cache invalidation, and off-by-one errors.",
    "i would tell you a UDP joke but you might not get it.",
    "a SQL query walks into a bar, approaches two tables and asks: may i join you?",
    "why did the developer go broke? because he used up all his cache.",
    "sudo make me a sandwich.",
    "it works on my machine — the four most expensive words in engineering.",
    "why do package managers never panic? they always resolve their dependencies.",
    "99 little bugs in the code, 99 little bugs… patch one down, run it around… 127 little bugs in the code.",
    "i told my computer i needed a break, and it said: no problem, i'll go to sleep.",
    "there's no place like 127.0.0.1.",
    "why was the javascript developer sad? because he didn't Node how to Express himself.",
];

static JOKES: OnceLock<Vec<String>> = OnceLock::new();
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// Load jokes: disk cache of fetched ones + bundled fallbacks.
pub fn init(client: &reqwest::Client) {
    let mut jokes: Vec<String> = BUNDLED.iter().map(|s| s.to_string()).collect();

    // Fetched jokes are cached for a day; refresh opportunistically.
    let cache_key = "jokes:dadjoke";
    if let Some(cached) = crate::cache::get("fun", cache_key, Duration::from_secs(86400)) {
        for line in cached.lines().filter(|l| !l.is_empty()) {
            jokes.push(line.to_string());
        }
    }

    // Kick off a background refresh (best effort, never blocks installs).
    let client = client.clone();
    std::thread::spawn(move || {
        // A tiny one-off runtime: jokes::init runs inside the main async
        // context, but this thread is plain sync.
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        rt.block_on(async {
            let mut fetched: Vec<String> = Vec::new();
            // grab a few; each request is tiny
            for _ in 0..3 {
                let Ok(resp) = client
                    .get("https://icanhazdadjoke.com")
                    .header("Accept", "text/plain")
                    .header("User-Agent", "px package manager")
                    .timeout(Duration::from_secs(3))
                    .send()
                    .await
                else {
                    break;
                };
                if let Ok(joke) = resp.text().await {
                    let joke = joke.trim().to_string();
                    if !joke.is_empty() {
                        fetched.push(joke);
                    }
                }
            }
            if !fetched.is_empty() {
                crate::cache::put("fun", cache_key, &fetched.join("\n"));
            }
        });
    });

    let _ = JOKES.set(jokes);
}

/// Cycle to the next joke (round-robin).
pub fn next() -> String {
    let jokes = JOKES.get_or_init(|| BUNDLED.iter().map(|s| s.to_string()).collect());
    let i = NEXT.fetch_add(1, Ordering::Relaxed) % jokes.len();
    jokes[i].clone()
}

/// All jokes (for the tutorial / doctor fun).
pub fn all() -> Vec<String> {
    JOKES
        .get_or_init(|| BUNDLED.iter().map(|s| s.to_string()).collect())
        .clone()
}
