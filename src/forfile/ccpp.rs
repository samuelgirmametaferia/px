//! C/C++ detector: #include scanning + header→package mapping.

use std::path::{Path, PathBuf};

use crate::error::PxResult;
use crate::forfile::detector::{Detected, Detector, Ecosystem, SystemDep, extension, file_name};
use crate::recipe::schema::Recipe;

/// glibc / compiler-bundled headers that are never package deps.
const DEFAULT_HEADERS: &[&str] = &[
    "stdio",
    "stdlib",
    "string",
    "math",
    "unistd",
    "fcntl",
    "assert",
    "ctype",
    "errno",
    "float",
    "limits",
    "locale",
    "setjmp",
    "signal",
    "stdarg",
    "stddef",
    "stdint",
    "stdio_ext",
    "time",
    "wchar",
    "wctype",
    "complex",
    "inttypes",
    "iso646",
    "stdalign",
    "stdatomic",
    "stdbit",
    "stdckdint",
    "stdbool",
    "stdnoreturn",
    "tgmath",
    "uchar",
    "sys",
    "linux",
    "arpa",
    "net",
    "netinet",
    "scsi",
    "sound",
    "video",
    "asm",
    "bits",
    "c++",
    "cassert",
    "cctype",
    "cerrno",
    "cfloat",
    "climits",
    "clocale",
    "cmath",
    "csignal",
    "cstdarg",
    "cstddef",
    "cstdint",
    "cstdio",
    "cstdlib",
    "cstring",
    "ctime",
    "cwchar",
    "cwctype",
    "algorithm",
    "array",
    "atomic",
    "bitset",
    "chrono",
    "codecvt",
    "complex",
    "condition_variable",
    "deque",
    "exception",
    "forward_list",
    "fstream",
    "functional",
    "future",
    "initializer_list",
    "iomanip",
    "ios",
    "iosfwd",
    "iostream",
    "istream",
    "iterator",
    "limits",
    "list",
    "locale",
    "map",
    "memory",
    "mutex",
    "new",
    "numeric",
    "ostream",
    "queue",
    "random",
    "ratio",
    "regex",
    "scoped_allocator",
    "set",
    "shared_mutex",
    "sstream",
    "stack",
    "stdexcept",
    "streambuf",
    "string",
    "string_view",
    "strstream",
    "system_error",
    "thread",
    "tuple",
    "type_traits",
    "typeindex",
    "typeinfo",
    "unordered_map",
    "unordered_set",
    "utility",
    "valarray",
    "variant",
    "vector",
    "version",
];

pub struct CCppDetector;

fn is_c_file(file: &Path) -> bool {
    matches!(
        extension(file).as_str(),
        "c" | "h" | "cpp" | "hpp" | "cc" | "hh" | "cxx" | "hxx"
    )
}

impl Detector for CCppDetector {
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::CCpp
    }

    fn matches(&self, _path: &Path, files: &[PathBuf]) -> bool {
        files.iter().any(|f| {
            is_c_file(f)
                || matches!(
                    file_name(f).as_str(),
                    "CMakeLists.txt" | "meson.build" | "Makefile" | "configure.ac"
                )
        })
    }

    fn analyze(&self, path: &Path, files: &[PathBuf], recipe: &Recipe) -> PxResult<Detected> {
        let eco = recipe.ecosystem("c").cloned().unwrap_or_default();
        let include_re = regex::Regex::new(r#"(?m)^\s*#\s*include\s*[<"]([^">]+)"#).unwrap();

        let mut headers: Vec<String> = Vec::new();
        let mut evidence: Vec<String> = Vec::new();
        let mut has_cmake = false;
        let mut has_meson = false;

        for file in files {
            let name = file_name(file);
            if is_c_file(file) {
                let Ok(text) = std::fs::read_to_string(file) else {
                    continue;
                };
                for caps in include_re.captures_iter(&text) {
                    if let Some(m) = caps.get(1) {
                        let path = m.as_str();
                        // First path segment: openssl/ssl.h → openssl
                        let first = path.split('/').next().unwrap_or(path);
                        // Strip .h for bare headers: stdio.h → stdio
                        let first = first.strip_suffix(".h").unwrap_or(first);
                        if !first.is_empty() && !headers.contains(&first.to_string()) {
                            headers.push(first.to_string());
                        }
                    }
                }
                if evidence.len() < 8 {
                    evidence.push(
                        file.strip_prefix(path)
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or(name.clone()),
                    );
                }
            } else if name == "CMakeLists.txt" {
                has_cmake = true;
            } else if name == "meson.build" {
                has_meson = true;
            }
        }

        let mut detected = Detected {
            tools: eco.tools.clone(),
            evidence,
            ..Default::default()
        };
        if has_cmake {
            detected.tools.push("cmake".into());
        }
        if has_meson {
            detected.tools.push("meson".into());
        }
        if files
            .iter()
            .any(|f| file_name(f) == "Makefile" || file_name(f) == "configure.ac")
        {
            // make is already in tools; autotools needs autoconf
            if files.iter().any(|f| file_name(f) == "configure.ac") {
                detected.tools.push("autoconf".into());
                detected.tools.push("automake".into());
            }
        }

        for header in &headers {
            if DEFAULT_HEADERS.contains(&header.as_str()) {
                continue;
            }
            // Longest-prefix match in the recipe's header_map: the header
            // dir must start with the key ("openssl/ssl.h" → key "openssl").
            let mapped = eco
                .header_map
                .iter()
                .filter(|(k, _)| header.starts_with(k.as_str()))
                .max_by_key(|(k, _)| k.len())
                .map(|(_, v)| v.clone());
            match mapped {
                Some(pkg) => detected.system.push(SystemDep {
                    import: format!("{header}.h"),
                    candidates: vec![pkg],
                    ecosystem: Ecosystem::CCpp,
                }),
                None => detected.unmapped.push(format!("header: {header}.h")),
            }
        }

        Ok(detected)
    }
}
