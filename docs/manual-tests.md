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
