# px

**the package-manager front-end for every Linux distro.**

px installs anything by driving the package managers your system **already
has** — pacman, apt, dnf, paru, yay — defined entirely as data in per-distro
**recipes**. px never reimplements package management, and it never talks to
repository APIs directly. Adding a distro is a TOML file, not a code change.

```
px install neovim google-chrome        # repo + AUR in parallel, best source wins
px install for ./my-project            # analyze a project, install what it needs
px -i                                  # interactive
px -i -> ./my-project                  # same thing, arrow syntax
px -i for ./my-project                 # same thing, again
px search firefxo                      # "did you mean: firefox?"
px doctor                              # what px sees on this machine
```

## How it works

1. **Detect** — read `/etc/os-release` (falls back to command probes) and pick
   the matching recipe. CachyOS → `arch`, Mint → `debian`, etc.
2. **Resolve** — query every active source *in parallel* (official repos, AUR
   via paru/yay if you have one, …) and pick the exact hit from the
   highest-priority source. No exact hit → fuzzy-ranked candidates →
   did-you-mean. Nothing anywhere → search GitHub for a source build.
3. **Confirm** — one itemized plan, one confirmation. Source builds always get
   their own dedicated "do you want me to download and build it for you?"
   (even under `--yes`).
4. **Install** — through the distro's own tools, elevated with sudo
   (preflighted once, so you're never prompted mid-flow).

### `install for <path>`

Point px at a project or script and it figures out the system packages:

| ecosystem | what it reads | what it maps to |
|---|---|---|
| Python | imports, requirements.txt, pyproject.toml | `python-{name}` (arch) / `python3-{name}` (deb/fedora) |
| C/C++ | `#include`, CMakeLists/meson/Makefile | `header_map` → e.g. `openssl` / `libssl-dev` / `openssl-devel` |
| Shell | commands in scripts, shebangs | missing binaries → packages |
| Node | package.json, imports | npm deps stay **local** (npm's own job) |
| Go | go.mod, imports | modules stay local; CGO → gcc |
| Java | pom.xml, build.gradle | jdk + maven/gradle |
| Ruby | Gemfile, requires | gems stay local (bundler) |

`--local` bootstraps a language environment instead (venv, npm install,
bundle) — nothing leaves the project, no sudo. Every proposed mapping is
validated against your real package sources before it reaches the plan;
unmappable imports are shown as a footnote, never guessed.

## Recipes are data

`recipes/arch.toml`, `recipes/debian.toml`, `recipes/fedora.toml` and
`recipes/opensuse.toml` ship with px (embedded in the binary as an offline
fallback, fetched from this repo's raw.githubusercontent URLs with a 24h
cache). The only differences between distros are strings:

```toml
# arch                          # debian                        # fedora
install = [                     install = [                     install = [
  "sudo", "pacman",               "sudo", "apt-get",              "sudo", "dnf",
  "-S", "--needed",               "install", "-y",               "install", "-y",
]                               ]                               ]

[ecosystems.python]             [ecosystems.python]             [ecosystems.python]
prefix = "python-{name}"        prefix = "python3-{name}"       prefix = "python3-{name}"
```

macOS (brew) and Windows (winget/scoop) will be recipes too — the schema
already supports `requires_elevation = false` and detection by command.

## Simulating other distros

`--recipe` + `--dry-run` lets you drive any distro's plan from any machine:

```
$ px --recipe recipes/debian.toml --dry-run install for fixtures/python
[dry-run] sudo apt-get install -y python3-flask
[dry-run] sudo apt-get install -y libssl-dev        # (c fixture)
...

$ px --recipe recipes/fedora.toml --dry-run install ffmpeg
[dry-run] sudo dnf install -y ffmpeg
```

Debian/Fedora command generation is pinned by tests (`tests/command_gen.rs`,
`tests/parsers.rs`) even though they're only live-testable on their own
distros.

## Commands

| command | what it does |
|---|---|
| `px install <pkgs...>` | parallel multi-source resolve + install |
| `px install for <path>` | project analysis (`--local` / `--global`) |
| `px -i` / `px -i -> <path>` / `px -i for <path>` | interactive forms |
| `px uninstall <pkgs...>` | remove packages through your distro's tools |
| `px suggest` | find unused packages + space hogs, offer to free them |
| `px status` | running px instance, interrupted installs, caches |
| `px search <term>` | search every source, fuzzy-ranked table |
| `px info <pkg>` | merged info from every source |
| `px list` | packages px installed (its own ledger) |
| `px doctor` | recipe, sources, tools, caches |
| `px tutorial` / `px --tutorial` | 2-minute interactive walkthrough |
| `px recipe list\|show` | inspect recipes |
| `px cache clean` | clean `~/.cache/px` |

Global flags: `-y/--yes`, `--dry-run`, `--local/--global`, `--no-source`,
`--refresh`, `--recipe <path>`, `--bar <style>`, `--no-color`, `-v/-vv`.

## Beyond packages

- **Apps with their own channels.** `px install claude-code` →
  `npm install -g @anthropic-ai/claude-code`; `px install bun` → the bun
  installer (shown verbatim, confirmed explicitly). Curated in each recipe's
  `[[apps]]` registry, with a generic npm-registry fallback.
- **Suspicious-package investigation.** Before installing an npm package,
  px checks public metadata — install scripts, typosquatting against
  popular names, single-publish red flags — and offers to investigate when
  anything looks off. The report is facts; the decision is yours.
- **Unused-package suggestions.** `px suggest` lists orphans (safe to
  remove, with sizes) and your biggest explicit packages, then uninstalls
  whatever you pick.
- **Resume.** Installs are journaled — a killed px offers to continue
  exactly where it stopped. `px status` shows running instances and
  pending work.
- **Update notices.** After installing, px passively mentions what else on
  your system has updates available.
- **Self-cleaning caches**, a **first-run spectrum animation**, **jokes
  during installs** (fresh from the internet, cached offline), and
  **five progress bar styles** (`--bar blocks|shades|rainbow|minimal|sparkles`).

## Safety rules

- px never runs as root; it elevates individual commands via sudo.
- Commands run via absolute paths (resolved with `which`) — your shell
  aliases can't change what px executes.
- Arguments are passed directly to child processes, never through a shell.
- Source builds from GitHub need their own explicit confirmation, always.
- Installer scripts (`curl | sh`) are shown verbatim before running.
- `--dry-run` prints the exact argv it would run and changes nothing.

## Building

```
cargo build --release
cargo test
```

Rust 1.85+ (edition 2024). No system dependencies beyond cargo.

## Status

v0.1.0 — Arch fully live-tested (incl. CachyOS detection and AUR through
paru/yay); Debian, Fedora and openSUSE recipes complete and contract-tested
but not yet live-tested on real machines. Source builds support cargo/go/
cmake/make/meson.
