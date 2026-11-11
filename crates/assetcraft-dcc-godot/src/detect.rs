//! Finding Godot on the machine.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
    Other,
}

pub fn current_os() -> Os {
    if cfg!(target_os = "linux") {
        Os::Linux
    } else if cfg!(target_os = "macos") {
        Os::MacOs
    } else if cfg!(target_os = "windows") {
        Os::Windows
    } else {
        Os::Other
    }
}

pub fn static_candidates(
    os: Os,
    home: Option<&Path>,
    program_files: Option<&Path>,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    match os {
        Os::Linux => {
            out.push(PathBuf::from("/usr/bin/godot"));
            out.push(PathBuf::from("/usr/bin/godot4"));
            out.push(PathBuf::from("/usr/local/bin/godot"));
            out.push(PathBuf::from("/usr/local/bin/godot4"));
            out.push(PathBuf::from("/snap/bin/godot"));
            out.push(PathBuf::from(
                "/var/lib/flatpak/exports/bin/org.godotengine.Godot",
            ));
            if let Some(home) = home {
                out.push(home.join(".local/bin/godot"));
                out.push(home.join(".local/bin/godot4"));
                out.push(home.join(".local/share/flatpak/exports/bin/org.godotengine.Godot"));
            }
            out.push(PathBuf::from("/opt/godot/godot"));
            out.push(PathBuf::from("/opt/godot/Godot"));
        }
        Os::MacOs => {
            out.push(PathBuf::from(
                "/Applications/Godot.app/Contents/MacOS/Godot",
            ));
            out.push(PathBuf::from(
                "/Applications/Godot_v4.app/Contents/MacOS/Godot",
            ));
            if let Some(home) = home {
                out.push(home.join("Applications/Godot.app/Contents/MacOS/Godot"));
            }
        }
        Os::Windows => {
            if let Some(pf) = program_files {
                out.push(pf.join("Godot").join("Godot.exe"));
                out.push(pf.join("Godot").join("Godot_v4.exe"));
            }
        }
        Os::Other => {}
    }
    out
}

pub fn versioned_windows_candidates(foundation: &Path, versions: &[String]) -> Vec<PathBuf> {
    let mut sorted: Vec<&String> = versions.iter().collect();
    sorted.sort_by_key(|v| std::cmp::Reverse(version_key(v)));
    sorted
        .into_iter()
        .map(|v| foundation.join(v).join("Godot.exe"))
        .collect()
}

fn version_key(version: &str) -> Vec<u32> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse().ok())
        .collect()
}

fn subdirectories(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

fn which(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let mut names = vec![binary.to_string()];
    if cfg!(windows) {
        names.push(format!("{binary}.exe"));
    }
    for directory in std::env::split_paths(&path) {
        for name in &names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn find() -> Option<PathBuf> {
    let os = current_os();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let program_files = std::env::var_os("ProgramFiles")
        .or_else(|| std::env::var_os("PROGRAMFILES"))
        .map(PathBuf::from);

    let mut candidates = static_candidates(os, home.as_deref(), program_files.as_deref());
    if os == Os::Windows {
        if let Some(pf) = &program_files {
            let foundation = pf.join("Godot");
            let versions = subdirectories(&foundation);
            let mut versioned = versioned_windows_candidates(&foundation, &versions);
            versioned.append(&mut candidates);
            candidates = versioned;
        }
    }
    if let Some(found) = candidates.into_iter().find(|p| p.is_file()) {
        return Some(found);
    }
    which("godot").or_else(|| which("godot4"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_candidates_cover_package_and_flatpak() {
        let home = PathBuf::from("/home/u");
        let c = static_candidates(Os::Linux, Some(&home), None);
        assert!(c.contains(&PathBuf::from("/usr/bin/godot")));
        assert!(c.contains(&PathBuf::from("/snap/bin/godot")));
        assert!(c.contains(&PathBuf::from(
            "/var/lib/flatpak/exports/bin/org.godotengine.Godot"
        )));
        assert!(c.contains(&home.join(".local/bin/godot")));
    }
    #[test]
    fn macos_candidates_cover_applications() {
        let home = PathBuf::from("/Users/u");
        let c = static_candidates(Os::MacOs, Some(&home), None);
        assert!(c.contains(&PathBuf::from(
            "/Applications/Godot.app/Contents/MacOS/Godot"
        )));
        assert!(c.contains(&home.join("Applications/Godot.app/Contents/MacOS/Godot")));
    }
    #[test]
    fn windows_candidates_need_program_files() {
        let pf = PathBuf::from("C:\\Program Files");
        let c = static_candidates(Os::Windows, None, Some(&pf));
        assert!(c.contains(&pf.join("Godot").join("Godot.exe")));
    }
    #[test]
    fn an_unknown_platform_has_no_candidates() {
        assert!(static_candidates(Os::Other, None, None).is_empty());
    }
}
