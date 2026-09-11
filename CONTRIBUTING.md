# Contributing to px

Useful contributions include reproducible install failures, distro recipe fixes,
missing package mappings, and clearer examples. Start small: one behavior or
recipe per pull request makes review easier.

## Report a problem

Include your distro, architecture (`uname -m`), `px --version`, the command you
ran, and the error output. `px doctor` helps identify missing tools. Remove
private paths, repository URLs, and credentials before sharing logs.

For unexpected resolution, include the software project's official URL and the
package or executable you expected. Avoid rerunning an installation solely to
collect logs; existing output or a dry run is usually enough.

## Work locally

```sh
git clone https://github.com/samuelgirmametaferia/px
cd px
cargo build --bin px
cargo test --all-targets
cargo fmt --check
cargo clippy --all-targets -- -D warnings
python3 scripts/test-installer.py
```

Detector fixtures live in `fixtures/` and regression tests in `tests/`.
Add a small fixture when correcting dependency detection. Installer tests use
fake downloads and temporary directories; they do not install software globally.

## Add or fix a distro recipe

Recipes in `recipes/` define package-manager arguments, output parsers, and
language-to-package mappings. Compare an existing recipe first, then cover your
change in `tests/recipes.rs`, `tests/command_gen.rs`, or `tests/parsers.rs`.
Use fixtures captured from the relevant package manager where possible.

A dry run can preview the generated install command:

```sh
cargo run --bin px -- --recipe recipes/debian.toml --dry-run install ffmpeg
```

Include the distro version and how you validated the result in your pull
request. The manual checklist is in [docs/manual-tests.md](docs/manual-tests.md).
