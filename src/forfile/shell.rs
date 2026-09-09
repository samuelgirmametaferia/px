//! Shell detector: find external commands scripts actually use, and any
//! interpreters their shebangs reference.
//!
//! Parsing shell "properly" is a tar pit; this is a deliberately
//! conservative extractor — it only proposes a command when several signals
//! agree (command position, not a builtin/keyword, not a variable
//! assignment, not a case pattern, not a function defined in the script,
//! not ALL-CAPS env-style, and it has a lowercase letter). False negatives
//! are fine; false positives (the `Windows_NT`/`fetched` garbage of early
//! versions) are not.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::error::PxResult;
use crate::forfile::detector::{
    Detected, Detector, Ecosystem, SystemDep, extension, file_name, first_line,
};
use crate::recipe::schema::Recipe;

/// Shell builtins/keywords/common-px-noise that are never package deps.
const BUILTINS: &[&str] = &[
    ":",
    ".",
    "[",
    "[[",
    "alias",
    "bg",
    "bind",
    "break",
    "builtin",
    "case",
    "cd",
    "command",
    "compgen",
    "complete",
    "continue",
    "declare",
    "dirs",
    "disown",
    "do",
    "done",
    "echo",
    "elif",
    "else",
    "esac",
    "eval",
    "exec",
    "exit",
    "export",
    "fc",
    "fg",
    "fi",
    "for",
    "function",
    "getopts",
    "hash",
    "help",
    "history",
    "if",
    "in",
    "jobs",
    "kill",
    "let",
    "local",
    "logout",
    "mapfile",
    "popd",
    "printf",
    "pushd",
    "pwd",
    "read",
    "readarray",
    "readonly",
    "return",
    "select",
    "set",
    "shift",
    "shopt",
    "source",
    "suspend",
    "test",
    "then",
    "time",
    "times",
    "trap",
    "type",
    "typeset",
    "ulimit",
    "umask",
    "unalias",
    "unset",
    "until",
    "wait",
    "while",
    "true",
    "false",
    "clear",
    "which",
    "whoami",
    "who",
    "id",
    "env",
    "nohup",
    "xargs",
    "tee",
    "sort",
    "uniq",
    "grep",
    "sed",
    "awk",
    "cut",
    "tr",
    "head",
    "tail",
    "cat",
    "ls",
    "cp",
    "mv",
    "rm",
    "mkdir",
    "rmdir",
    "touch",
    "chmod",
    "chown",
    "ln",
    "dirname",
    "basename",
    "date",
    "sleep",
    "find",
    "wc",
    "du",
    "df",
    "ps",
    "killall",
    "pkill",
    "tar",
    "gzip",
    "gunzip",
    "bzip2",
    "xz",
    "ssh",
    "scp",
    "rsync",
    "curl",
    "wget",
    "git",
    "make",
    "man",
    "less",
    "more",
    "vi",
    "vim",
    "nano",
    "sudo",
    "su",
    "apt",
    "dpkg",
    "pacman",
    "dnf",
    "rpm",
    "zypper",
    "systemctl",
    "journalctl",
    "ip",
    "ifconfig",
    "netstat",
    "ping",
    "traceroute",
    "dig",
    "nslookup",
    "hostname",
    "uname",
    "uptime",
    "free",
    "mount",
    "umount",
    "seq",
    "expr",
    "numfmt",
    "stat",
    "file",
    "md5sum",
    "sha256sum",
    "base64",
    "openssl",
    "python",
    "python3",
    "pip",
    "node",
    "npm",
    "npx",
    "go",
    "cargo",
    "rustc",
    "gcc",
    "g++",
    "cc",
    "java",
    "javac",
    "ruby",
    "gem",
    "perl",
    "php",
    "print",
    "readlink",
    "realpath",
    "watch",
    "yes",
    "screen",
    "tmux",
    "top",
    "htop",
    "zip",
    "unzip",
    "7z",
    "coproc",
];

/// Common English words that show up at command position in scripts (echo
/// arguments, log messages, case bodies) but are never program names. Small,
/// honest denylist — the layered filters above it do the heavy lifting.
const WORD_NOISE: &[&str] = &[
    "on",
    "off",
    "in",
    "out",
    "ok",
    "no",
    "yes",
    "all",
    "none",
    "some",
    "fall",
    "fail",
    "failed",
    "success",
    "error",
    "warning",
    "info",
    "debug",
    "trace",
    "version",
    "check",
    "checked",
    "checking",
    "cached",
    "fetched",
    "fetching",
    "expected",
    "probing",
    "refusing",
    "skipping",
    "skips",
    "sandboxes",
    "sandbox",
    "asset",
    "assets",
    "exe",
    "bin",
    "lib",
    "src",
    "opt",
    "tmp",
    "var",
    "etc",
    "usr",
    "home",
    "root",
    "cache",
    "impeccable",
    "download",
    "downloads",
    "path",
    "file",
    "files",
    "dir",
    "name",
    "value",
    "type",
    "mode",
    "data",
    "text",
    "line",
    "lines",
    "code",
    "usage",
    "help",
    "please",
    "done",
    "todo",
    "note",
    "notes",
    "only",
    "also",
    "then",
    "when",
    "with",
    "without",
    "from",
    "into",
    "onto",
    "over",
    "under",
    "after",
    "before",
    "amd64",
    "x86_64",
    "aarch64",
    "arm64",
    "arm",
    "i386",
    "darwin",
    "linux",
    "unix",
    "cygwin",
    "mingw",
    "msys",
    "windows",
    "macos",
    "osx",
    "posix",
];

pub struct ShellDetector;

impl ShellDetector {
    fn is_shell_script(&self, file: &Path) -> bool {
        let ext = extension(file);
        if ext == "sh" || ext == "bash" || ext == "zsh" {
            return true;
        }
        // Extensionless executable with a sh shebang.
        if ext.is_empty()
            && let Some(line) = first_line(file)
                && line.starts_with("#!") && (line.contains("/sh") || line.contains("bash")) {
                    return true;
                }
        false
    }
}

/// Function names defined anywhere in the script (`foo() {`, `function foo`).
fn defined_functions(text: &str) -> BTreeSet<String> {
    let re = regex::Regex::new(r"(?m)^\s*(?:function\s+)?([\w-]+)\s*\(\s*\)").unwrap();
    re.captures_iter(text)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

/// True for lines that are `case` branch patterns (`foo|bar)` or `*)`).
fn is_case_pattern(line: &str) -> bool {
    let t = line.trim_start();
    let Some(before) = t.split(')').next() else {
        return false;
    };
    !before.is_empty()
        && before
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "|*_-.".contains(c))
        && !before.contains(' ')
}

impl Detector for ShellDetector {
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Shell
    }

    fn matches(&self, _path: &Path, files: &[PathBuf]) -> bool {
        files.iter().any(|f| self.is_shell_script(f))
    }

    fn analyze(&self, path: &Path, files: &[PathBuf], recipe: &Recipe) -> PxResult<Detected> {
        let eco = recipe.ecosystem("shell").cloned().unwrap_or_default();
        let mut commands: BTreeSet<String> = BTreeSet::new();
        let mut interpreters: BTreeSet<String> = BTreeSet::new();
        let mut evidence: Vec<String> = Vec::new();

        // Word at "command position": start of line or after ; | & $( `etc.
        // (?m) makes ^ anchor at every line start.
        let cmd_start = regex::Regex::new(
            r"(?m)(?:^|[;|&]\s*|\$\(\s*|`\s*|\bthen\s+|\bdo\s+|\belse\s+|\belif\s+)([a-zA-Z][\w.+-]*)",
        )
        .unwrap();
        // Assignment lines: `FOO=...`, `local FOO=...`, `export FOO=...`.
        let assignment =
            regex::Regex::new(r"^\s*(local|export|declare|readonly|typeset)\s|^\s*[\w.\[\]-]+=")
                .unwrap();

        for file in files {
            if !self.is_shell_script(file) {
                // Still peek at shebangs of any script (python3, ruby, ...)
                if extension(file).is_empty()
                    && let Some(line) = first_line(file)
                    && let Some(interp) = shebang_interpreter(&line)
                {
                    interpreters.insert(interp);
                }
                continue;
            }
            let Ok(text) = std::fs::read_to_string(file) else {
                continue;
            };
            if let Some(line) = first_line(file)
                && let Some(interp) = shebang_interpreter(&line)
            {
                interpreters.insert(interp);
            }
            let functions = defined_functions(&text);

            for line in text.lines() {
                // Case branch patterns and pure assignments are not commands.
                if is_case_pattern(line) || assignment.is_match(line) {
                    continue;
                }
                if let Some(caps) = cmd_start.captures(line)
                    && let Some(m) = caps.get(1)
                {
                    let word = m.as_str();
                    // Layered filters — every one must pass.
                    let plausible = !BUILTINS.contains(&word)
                            && !WORD_NOISE.contains(&word.to_lowercase().as_str())
                            && !functions.contains(word)
                            // env-var style: CDPATH, Windows_NT, PATH_2_X
                            && !(word.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                                && !word.chars().any(|c| c.is_ascii_lowercase()))
                            && word.chars().any(|c| c.is_ascii_lowercase())
                            && word.len() <= 24;
                    if plausible {
                        commands.insert(word.to_string());
                    }
                }
            }
            if evidence.len() < 8 {
                evidence.push(
                    file.strip_prefix(path)
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| file_name(file)),
                );
            }
        }

        let mut detected = Detected::default();

        // Interpreters referenced by shebangs (python3, ruby, ...) map to the
        // matching ecosystem's tools when that ecosystem is enabled.
        for interp in &interpreters {
            if let Some(pkg) = eco.overrides.get(interp) {
                detected.system.push(SystemDep {
                    import: interp.clone(),
                    candidates: vec![pkg.clone()],
                    ecosystem: Ecosystem::Shell,
                });
            }
        }

        for cmd in &commands {
            // Present on this machine? Then no need to propose anything —
            // unless the recipe has an explicit override anyway.
            let on_path = which::which(cmd).is_ok();
            if let Some(pkg) = eco.overrides.get(cmd) {
                detected.system.push(SystemDep {
                    import: format!("command: {cmd}"),
                    candidates: vec![pkg.clone()],
                    ecosystem: Ecosystem::Shell,
                });
            } else if !on_path {
                // Not on PATH and not a builtin → propose the same-named
                // package; the resolver validates it exists before install.
                detected.system.push(SystemDep {
                    import: format!("command: {cmd}"),
                    candidates: vec![cmd.clone()],
                    ecosystem: Ecosystem::Shell,
                });
            }
        }

        detected.evidence = evidence;
        Ok(detected)
    }
}

fn shebang_interpreter(line: &str) -> Option<String> {
    let rest = line.strip_prefix("#!")?;
    let rest = rest.trim();
    // /usr/bin/env python3 → python3; /bin/bash → bash
    let mut parts = rest.split_whitespace();
    let mut interp = parts.next()?;
    if interp.ends_with("/env") {
        interp = parts.next()?;
    }
    let interp = interp.rsplit('/').next()?;
    if interp
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        Some(interp.to_string())
    } else {
        None
    }
}
