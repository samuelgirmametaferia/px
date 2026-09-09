# Manual test checklist

Run these on a real machine before calling a milestone done. Everything
under "live" needs an Arch-family box (this repo was built on CachyOS);
everything else runs anywhere.

## Bootstrap

```
cargo build --release
alias px=./target/release/px    # or cargo run --release -- ...
```

## Core flows (live, needs pacman)

- [ ] `px doctor` — detects CachyOS via `ID_LIKE=arch`; repo via pacman,
      aur via paru or yay (or "inactive — needs paru or yay" without them)
- [ ] `px search neovim` — spinner per source, table with both repo and AUR
      hits, no duplicates
- [ ] `px search firefxo` — "did you mean: firefox…?"
- [ ] `px info neovim` — every source that knows it reports version/desc
- [ ] `px --dry-run install ripgrep sl` — ripgrep skipped if installed,
      prints `[dry-run] /usr/bin/sudo pacman -S --needed sl`
- [ ] `px install sl` (real) — sudo preflight once, confirm plan, pacman
      runs with inherited stdio, `px list` shows it afterwards
- [ ] `px install zigdown-bin` (AUR-only) — resolves to aur, installs via
      `yay -S --needed --noconfirm` (or paru)
- [ ] `px install nonexistentxyz123` — not found, no crash, exit 1

## install for (live)

- [ ] `px --dry-run --global install for fixtures/python`
      → python-flask, numpy, python-opencv (cv2), python-pillow (PIL)
- [ ] `px --dry-run --global install for fixtures/c`
      → openssl, curl, zlib, sqlite + gcc/make/pkgconf
- [ ] `px --dry-run --global install for fixtures/shell`
      → ffmpeg, jq, inotify-tools (curl denied as common)
- [ ] `px --dry-run --global install for fixtures/node` — npm deps stay local
- [ ] `px --local install for <real python project>` — creates .venv, pip
      installs, no sudo prompt
- [ ] `px -i` then a path → same flow; `px -i -> fixtures/python` too

## Distro simulation (no special machine needed)

- [ ] `px --recipe recipes/debian.toml --dry-run install ffmpeg`
      → `[dry-run] /usr/bin/sudo apt-get install -y ffmpeg`
- [ ] `px --recipe recipes/fedora.toml --dry-run install ffmpeg jq`
      → `[dry-run] /usr/bin/sudo dnf install -y ffmpeg jq` (batched)
- [ ] `px --recipe recipes/debian.toml --dry-run --global install for fixtures/c`
      → libssl-dev, libcurl4-openssl-dev, zlib1g-dev, libsqlite3-dev

## Source builds (live)

- [ ] `px install <github-only-project>` — shows repos with stars, asks the
      dedicated question; declining prints "skipped source build"
- [ ] Confirm on a cargo project — clone into ~/.cache/px/builds, cargo
      build + install; `px list` records it as github
- [ ] `px --dry-run install charmbracelet-glow` — "would offer to clone and
      build (needs confirmation)"

## Edge cases

- [ ] `NO_COLOR=1 px search vim | cat` — no ANSI codes in the pipe
- [ ] `px` in a non-tty (CI) — prompts skipped, sensible errors, exit != 0
- [ ] `sudo px search vim` — refuses to run as root
- [ ] fish shell with `sudo` aliased — px still runs /usr/bin/sudo
- [ ] `px cache clean --all` then `px doctor` — caches empty, still works
      (bundled recipe fallback)
- [ ] Network off: `px doctor` works, `px search` still works (pacman is
      local), install works without recipe fetch

## Automated

```
cargo test          # 32 tests: recipes, command_gen (3 distros), parsers,
                    # detectors (7 ecosystems), fuzzy
cargo clippy --all-targets   # zero warnings
cargo fmt --check
```

## Universal install method matrix (verified 2026-09-09)

Every install method, verified end-to-end through `px install`:

| method | test package | result |
|---|---|---|
| script (pinned hash + sandbox) | pxfake-e2e (tests/fakepkg.rs, local HTTP) | ✔ install + binary runs; hash mismatch STOPS install |
| cargo | agent-code (avala-ai) | ✔ sandboxed build, `agent 0.30.0` |
| npm | nodemon (remy/nodemon) | ✔ cross-verified identity (npm repo == GitHub repo), `3.1.14` |
| go | pxtest-go (samuelgirmametaferia/pxtest-go) | ✔ `go install ...@latest`, binary runs |
| pipx | when-changed (pypi) | ✔ binary at ~/.local/bin/when-changed |
| release binary | pxtest-go v1.0.0 release | asset selection ✔ (correct tar.gz + checksums.txt detected from a real release); download blocked by this network's broken github-release CDN route — curl fails identically |
| gem / brew | (tools not installed) | ✔ correctly filtered out of candidates |
| source build | exercised in fallthroughs | ✔ correctly refused for JS-only repos (nodemon) |

Also verified live: the fakepkg hostile-installer scan (rc-persistence +
base64-into-shell = Dangerous), dead-record tombstones, library-crate
fallthrough (crates.io `is-even` is a library → npm namesake with its own
repo link = a different verified project), and `px remove <binary-name>`
routing to `cargo uninstall <crate-name>` via the install DB.
