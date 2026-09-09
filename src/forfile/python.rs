//! Python detector: imports in .py files + requirements/pyproject manifests.

use std::path::{Path, PathBuf};

use crate::error::PxResult;
use crate::forfile::detector::{
    Detected, Detector, Ecosystem, LocalDep, SystemDep, extension, file_name,
};
use crate::recipe::schema::Recipe;

/// Python stdlib modules we must never propose packages for. (Curated list —
/// a superset check keeps it short: anything here stays local to python.)
const STDLIB: &[&str] = &[
    "abc",
    "argparse",
    "array",
    "ast",
    "asyncio",
    "base64",
    "binascii",
    "bisect",
    "calendar",
    "cmath",
    "collections",
    "concurrent",
    "configparser",
    "contextlib",
    "contextvars",
    "copy",
    "copyreg",
    "csv",
    "ctypes",
    "dataclasses",
    "datetime",
    "decimal",
    "difflib",
    "dis",
    "email",
    "enum",
    "errno",
    "faulthandler",
    "filecmp",
    "fileinput",
    "fnmatch",
    "fractions",
    "ftplib",
    "functools",
    "gc",
    "getopt",
    "getpass",
    "gettext",
    "glob",
    "graphlib",
    "gzip",
    "hashlib",
    "heapq",
    "hmac",
    "html",
    "http",
    "imaplib",
    "importlib",
    "inspect",
    "io",
    "ipaddress",
    "itertools",
    "json",
    "keyword",
    "linecache",
    "locale",
    "logging",
    "lzma",
    "mailbox",
    "math",
    "mimetypes",
    "multiprocessing",
    "operator",
    "os",
    "pathlib",
    "pdb",
    "pickle",
    "pkgutil",
    "platform",
    "plistlib",
    "poplib",
    "posixpath",
    "pprint",
    "profile",
    "pstats",
    "pty",
    "pwd",
    "py_compile",
    "pyclbr",
    "queue",
    "quopri",
    "random",
    "re",
    "readline",
    "reprlib",
    "resource",
    "secrets",
    "select",
    "selectors",
    "shelve",
    "shlex",
    "shutil",
    "signal",
    "site",
    "smtplib",
    "socket",
    "socketserver",
    "sqlite3",
    "ssl",
    "statistics",
    "string",
    "stringprep",
    "struct",
    "subprocess",
    "symtable",
    "sys",
    "sysconfig",
    "tarfile",
    "tempfile",
    "textwrap",
    "threading",
    "time",
    "timeit",
    "tkinter",
    "token",
    "traceback",
    "tracemalloc",
    "typing",
    "types",
    "typing_extensions",
    "unicodedata",
    "unittest",
    "urllib",
    "uuid",
    "venv",
    "warnings",
    "wave",
    "weakref",
    "webbrowser",
    "wsgiref",
    "xml",
    "xmlrpc",
    "zipapp",
    "zipfile",
    "zipimport",
    "zlib",
    "__future__",
    "typing_extensions",
    "antigravity",
    "this",
];

pub struct PythonDetector;

impl Detector for PythonDetector {
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Python
    }

    fn matches(&self, _path: &Path, files: &[PathBuf]) -> bool {
        files.iter().any(|f| {
            let name = file_name(f);
            extension(f) == "py"
                || name == "pyproject.toml"
                || name == "requirements.txt"
                || name.starts_with("requirements") && name.ends_with(".txt")
                || name == "Pipfile"
                || name == "setup.py"
        })
    }

    fn analyze(&self, path: &Path, files: &[PathBuf], recipe: &Recipe) -> PxResult<Detected> {
        let mut imports: Vec<String> = Vec::new();
        let mut evidence: Vec<String> = Vec::new();
        let mut manifest_deps: Vec<String> = Vec::new();

        let import_re =
            regex::Regex::new(r"^\s*(?:import\s+([\w.]+)|from\s+([\w.]+)\s+import)").unwrap();
        for file in files {
            let name = file_name(file);
            let ext = extension(file);
            let Ok(text) = std::fs::read_to_string(file) else {
                continue;
            };

            if ext == "py" || name == "setup.py" {
                for line in text.lines() {
                    if let Some(caps) = import_re.captures(line) {
                        let target = caps
                            .get(1)
                            .or_else(|| caps.get(2))
                            .map(|m| m.as_str().to_string())
                            .unwrap_or_default();
                        if let Some(first) = target.split('.').next()
                            && !first.is_empty()
                            && !imports.contains(&first.to_string())
                        {
                            imports.push(first.to_string());
                        }
                    }
                }
                if !text.trim().is_empty() && evidence.len() < 8 {
                    evidence.push(rel_name(path, file));
                }
            } else if name == "requirements.txt"
                || (name.starts_with("requirements") && name.ends_with(".txt"))
            {
                for line in text.lines() {
                    let dep = line
                        .split('#')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .split(['=', '<', '>', '!', '[', ';'])
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if !dep.is_empty() && !manifest_deps.contains(&dep) {
                        manifest_deps.push(dep);
                    }
                }
                evidence.push(name);
            } else if name == "pyproject.toml"
                && let Ok(doc) = text.parse::<toml::Table>()
            {
                let deps = doc
                    .get("project")
                    .and_then(|p| p.as_table())
                    .and_then(|p| p.get("dependencies"))
                    .and_then(|d| d.as_array());
                if let Some(deps) = deps {
                    for dep in deps {
                        if let Some(s) = dep.as_str() {
                            let name = s
                                .split(['=', '<', '>', '!', '[', ' ', ';'])
                                .next()
                                .unwrap_or("")
                                .trim()
                                .to_string();
                            if !name.is_empty() && !manifest_deps.contains(&name) {
                                manifest_deps.push(name);
                            }
                        }
                    }
                }
                evidence.push(name);
            }
        }

        // Merge: manifest deps are authoritative; source imports fill gaps.
        let mut all = manifest_deps.clone();
        for imp in &imports {
            if !all.contains(imp) {
                all.push(imp.clone());
            }
        }

        let eco = recipe.ecosystem("python").cloned().unwrap_or_default();
        let mut detected = Detected {
            tools: eco.tools.clone(),
            ..Default::default()
        };

        for name in &all {
            let norm = name.replace('_', "-");
            if STDLIB.contains(&name.as_str()) {
                continue;
            }
            // Candidates in preference order. The `{name}` placeholder in the
            // recipe prefix is substituted (arch: python-{name}); the bare
            // name catches distros that don't prefix (arch's `numpy`).
            if let Some(pkg) = eco.overrides.get(name) {
                detected.system.push(SystemDep {
                    import: name.clone(),
                    candidates: vec![pkg.clone()],
                    ecosystem: Ecosystem::Python,
                });
            } else if let Some(prefix) = &eco.prefix {
                detected.system.push(SystemDep {
                    import: name.clone(),
                    candidates: vec![prefix.replace("{name}", &norm), norm.clone()],
                    ecosystem: Ecosystem::Python,
                });
                detected.local.push(LocalDep {
                    name: name.clone(),
                    ecosystem: Ecosystem::Python,
                });
            } else {
                detected.unmapped.push(format!("python:{name}"));
            }
        }

        if !eco.runtime_deps.is_empty() && !detected.system.is_empty() {
            // Native builds need these only when there's something to install.
            for dep in &eco.runtime_deps {
                if !detected.tools.contains(dep) {
                    detected.tools.push(dep.clone());
                }
            }
        }

        detected.evidence = evidence;
        Ok(detected)
    }
}

fn rel_name(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| file_name(file))
}
