use clap::Parser;
use px::cli::Cli;

/// Preprocess argv so `px -i for x` / `px -i -> x` / `px -i` work even though
/// clap can't parse `->` or short flags before subcommands.
///   px -i                    → px install (interactive prompt)
///   px -i for <path>         → px install for <path>
///   px -i -> <path>          → px install for <path>
///   px -i <pkg...>           → px install <pkg...>
/// Global flags may precede -i (px --dry-run -i -> x). A `-i` that follows a
/// subcommand is left alone.
fn normalize_argv(args: Vec<String>) -> Vec<String> {
    const SUBCOMMANDS: &[&str] = &[
        "install", "search", "info", "list", "doctor", "recipe", "cache",
    ];
    // Find a standalone `-i` that appears before any subcommand token.
    let mut pos = None;
    for (i, tok) in args.iter().enumerate().skip(1) {
        if SUBCOMMANDS.contains(&tok.as_str()) {
            break; // subcommand context: -i belongs to it (or is invalid there)
        }
        if tok == "-i" {
            pos = Some(i);
            break;
        }
    }
    let Some(pos) = pos else {
        return args;
    };

    let mut out: Vec<String> = vec!["px".into()];
    let before = &args[1..pos];
    let rest = &args[pos + 1..];
    out.extend(before.iter().cloned());

    // `px -i -> <path>` / `px -i for <path>` → install for <path>
    if let Some(first) = rest.first()
        && (first == "->" || first == "→" || first == "for")
        && rest.len() >= 2
    {
        out.push("install".into());
        out.push("for".into());
        out.push(rest[1].clone());
        return out;
    }

    // `px -i <pkgs...>` → install <pkgs...>; bare `px -i` → install (prompt)
    out.push("install".into());
    out.extend(rest.iter().cloned());
    out
}

fn main() {
    let raw: Vec<String> = std::env::args().collect();
    let args = normalize_argv(raw);
    let cli = Cli::parse_from(args);

    // Tracing verbosity.
    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            match cli.verbose {
                0 => tracing_subscriber::EnvFilter::new("warn"),
                1 => tracing_subscriber::EnvFilter::new("info"),
                _ => tracing_subscriber::EnvFilter::new("debug"),
            }
        });
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let code = runtime.block_on(async {
        let app = match px::app::App::init(cli).await {
            Ok(app) => app,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        match app.run().await {
            Ok(()) => 0,
            Err(px::error::PxError::Cancelled) => 130,
            Err(e) => {
                eprintln!("{e}");
                1
            }
        }
    });
    std::process::exit(code);
}
