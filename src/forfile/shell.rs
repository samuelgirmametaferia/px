//! Shell detector: find external commands scripts actually use, and any
//! interpreters their shebangs reference.

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
    "popd",
    "printf",
    "pushd",
    "pwd",
    "read",
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
    "fstab",
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
    "echo",
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
    "rsync",
];

/// Commands in the denylist that ARE worth proposing (they're commonly not
/// installed by default) — the recipe's overrides decide their package.
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
            && line.starts_with("#!")
            && (line.contains("/sh") || line.contains("bash"))
        {
            return true;
        }
        false
    }
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

        // Word at "command position": start of line or after ; | && || $( `etc.
        // (?m) makes ^ anchor at every line start.
        let cmd_start = regex::Regex::new(
            r"(?m)(?:^|[;|&]\s*|\$\(\s*|`\s*|\bthen\s+|\bdo\s+|\belse\s+|\belif\s+)([a-zA-Z][\w.+-]*)",
        )
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
            for caps in cmd_start.captures_iter(&text) {
                if let Some(m) = caps.get(1) {
                    commands.insert(m.as_str().to_string());
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
            if BUILTINS.contains(&cmd.as_str()) {
                continue;
            }
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
