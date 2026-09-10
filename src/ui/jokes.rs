//! Jokes during installs. Primary source: the px joke dataset
//! (github.com/samuelgirmametaferia/pxJokeData — the r/Jokes collection).
//! A SECTION of the compressed TSV is range-fetched (~256KB, never the
//! whole file), partially decompressed, and the humorous entries (label 1)
//! become the local pool. icanhazdadjoke supplements; bundled fallbacks
//! cover offline first runs. One joke rotates into every install and the
//! bar message every ~15s.

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

static JOKES: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());
static NEXT: AtomicUsize = AtomicUsize::new(0);
static INITIALIZED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The rotation index persists on disk — px is a fresh process every run,
/// and an in-memory counter resets to the SAME first joke every single
/// time (the "why do programmers prefer dark mode?" forever bug).
fn rotation_file() -> std::path::PathBuf {
    crate::cache::cache_root().join("joke-index")
}

fn load_rotation() -> usize {
    std::fs::read_to_string(rotation_file())
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn store_rotation(i: usize) {
    let f = rotation_file();
    if let Some(parent) = f.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(f, i.to_string());
}

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
            // primary: a section of the px joke dataset
            if let Some(pool) = fetch_dataset_section(&client).await {
                // RwLock: the pool merges into the live set (OnceLock's
                // set-once semantics silently dropped fetched jokes)
                let mut guard = JOKES.write().unwrap();
                for j in pool {
                    if !guard.contains(&j) {
                        guard.push(j);
                    }
                }
                let keep: Vec<String> = guard.iter().skip(BUNDLED.len()).cloned().collect();
                drop(guard);
                if !keep.is_empty() {
                    crate::cache::put("fun", cache_key, &keep.join("\n"));
                }
                return;
            }
            // fallback: icanhazdadjoke
            let mut fetched: Vec<String> = Vec::new();
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

    let mut guard = JOKES.write().unwrap();
    if guard.is_empty() {
        *guard = jokes;
        INITIALIZED.store(true, Ordering::Release);
    }
}

/// Cycle to the next joke — round-robin ACROSS RUNS (disk-persisted
/// index) and within a run.
pub fn next() -> String {
    ensure_init();
    let jokes_len = JOKES.read().unwrap().len();

    // in-process calls advance the atomic; the first call of each process
    // seeds it from the persisted index so runs continue the rotation
    let i = if NEXT.load(Ordering::Relaxed) == 0 {
        let start = load_rotation();
        NEXT.store(start + 1, Ordering::Relaxed);
        store_rotation(start + 1);
        start % jokes_len.max(1)
    } else {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        store_rotation(n + 1);
        n % jokes_len.max(1)
    };
    JOKES
        .read()
        .unwrap()
        .get(i)
        .cloned()
        .unwrap_or_else(|| BUNDLED[0].to_string())
}

fn ensure_init() {
    if !INITIALIZED.load(Ordering::Acquire) {
        let mut guard = JOKES.write().unwrap();
        if guard.is_empty() {
            *guard = BUNDLED.iter().map(|s| s.to_string()).collect();
            INITIALIZED.store(true, Ordering::Release);
        }
    }
}

/// All jokes (for the tutorial / doctor fun).
pub fn all() -> Vec<String> {
    ensure_init();
    JOKES.read().unwrap().clone()
}

const JOKE_DATA_REPO: &str = "samuelgirmametaferia/pxJokeData";
const JOKE_SECTION_BYTES: usize = 256 * 1024; // a section, never the whole file

/// Range-fetch a 256KB section of one of the dataset's gzipped TSVs,
/// decompress what we can, and return the humorous (label 1) jokes.
/// Picks the file by day so different runs sample different sections.
async fn fetch_dataset_section(client: &reqwest::Client) -> Option<Vec<String>> {
    let files = ["dev.tsv.gz", "test.tsv.gz", "train.tsv.gz"];
    let day = chrono::Utc::now().format("%j").to_string();
    let pick = day
        .chars()
        .last()
        .and_then(|c| c.to_digit(10))
        .map(|d| (d % files.len() as u32) as usize)
        .unwrap_or(0);
    let url = format!(
        "https://raw.githubusercontent.com/{}/{}/data/{}",
        JOKE_DATA_REPO, "master", files[pick]
    );
    let resp = client
        .get(&url)
        .header("Range", format!("bytes=0-{JOKE_SECTION_BYTES}"))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    // 206 = partial content (range honored); 200 means the server ignored
    // the range and is sending the whole file — take it but stop reading
    // after the section size to avoid pulling megabytes
    let bytes: Vec<u8> = if resp.status().as_u16() == 206 {
        resp.bytes().await.ok()?.to_vec()
    } else {
        // the server ignored the range — stream chunks and stop at the cap
        let mut resp = resp;
        let mut buf: Vec<u8> = Vec::with_capacity(JOKE_SECTION_BYTES);
        while buf.len() < JOKE_SECTION_BYTES {
            match resp.chunk().await {
                Ok(Some(chunk)) => {
                    buf.extend_from_slice(&chunk);
                    if buf.len() >= JOKE_SECTION_BYTES {
                        buf.truncate(JOKE_SECTION_BYTES);
                        break;
                    }
                }
                _ => break,
            }
        }
        buf
    };
    if bytes.is_empty() {
        return None;
    }
    // partial gzip decompress: flate2 gives us everything decompressable
    use flate2::read::MultiGzDecoder;
    use std::io::Read;
    let mut text = String::new();
    let mut dec = MultiGzDecoder::new(&bytes[..]);
    let _ = dec.read_to_string(&mut text); // errors at truncation — fine
    let jokes: Vec<String> = text
        .lines()
        .filter_map(|l| {
            let (label, body) = l.split_once('\t')?;
            if label.trim() != "1" {
                return None; // only the humorous ones
            }
            let body = body.trim();
            // reasonable length for a spinner line; skip novels
            if body.len() < 20 || body.len() > 200 || body.contains('\n') {
                return None;
            }
            Some(body.to_string())
        })
        .collect();
    if jokes.is_empty() {
        None
    } else {
        tracing::debug!(
            "joke dataset: {} jokes from a section of {}",
            jokes.len(),
            files[pick]
        );
        Some(jokes)
    }
}
