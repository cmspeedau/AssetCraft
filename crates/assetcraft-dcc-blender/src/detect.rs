//! Finding Blender on the machine.
//!
//! The exporter has to hand its script to a Blender binary, and there is no
//! single place it lives: a package manager puts it on `PATH`, an installer
//! drops an app bundle or a versioned folder, and the user may have several.
//! The candidate list is built from the locations each platform is known to use,
//! then checked for existence in order of preference.
//!
//! The pure parts — the candidate lists and the version ordering — are separated
//! from the environment lookups so they can be tested on any machine.

use std::path::{Path, PathBuf};

/// The platform whose install locations should be searched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    /// Linux, including the common package and Flatpak locations.
    Linux,
    /// macOS app bundles.
    MacOs,
    /// Windows, under `Program Files`.
    Windows,
    /// Anything else: no known locations.
    Other,
}

/// The platform this binary is running on.
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

/// Blender locations that do not depend on a version number, most preferred
/// first.
///
/// `home` and `program_files` are passed in rather than read from the
/// environment so the list can be tested for every platform on any machine.
pub fn static_candidates(
    os: Os,
    home: Option<&Path>,
    program_files: Option<&Path>,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    match os {
        Os::Linux => {
            out.push(PathBuf::from("/usr/bin/blender"));
            out.push(PathBuf::from("/usr/local/bin/blender"));
            out.push(PathBuf::from("/snap/bin/blender"));
            out.push(PathBuf::from(
                "/var/lib/flatpak/exports/bin/org.blender.Blender",
            ));
            if let Some(home) = home {
                out.push(home.join(".local/bin/blender"));
                out.push(home.join(".local/share/flatpak/exports/bin/org.blender.Blender"));
            }
            out.push(PathBuf::from("/opt/blender/blender"));
        }
        Os::MacOs => {
            out.push(PathBuf::from(
                "/Applications/Blender.app/Contents/MacOS/Blender",
            ));
            if let Some(home) = home {
                out.push(home.join("Applications/Blender.app/Contents/MacOS/Blender"));
            }
        }
        Os::Windows => {
            if let Some(program_files) = program_files {
                out.push(
                    program_files
                        .join("Blender Foundation")
                        .join("Blender")
                        .join("blender.exe"),
                );
            }
        }
        Os::Other => {}
    }
    out
}

/// Blender locations inside `Blender Foundation`, newest version first.
///
/// Windows installs Blender into a folder named for its version
/// (`Blender 4.2`), so the directory has to be listed and the versions sorted
/// rather than guessed at.
pub fn versioned_windows_candidates(foundation: &Path, versions: &[String]) -> Vec<PathBuf> {
    let mut sorted: Vec<&String> = versions.iter().collect();
    sorted.sort_by_key(|version| std::cmp::Reverse(version_key(version)));
    sorted
        .into_iter()
        .map(|version| foundation.join(version).join("blender.exe"))
        .collect()
}

/// A sortable key for a version string like `Blender 4.2` or `4.2.1`.
fn version_key(version: &str) -> Vec<u32> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

/// The first Blender found in the known locations, then on `PATH`.
pub fn find() -> Option<PathBuf> {
    let os = current_os();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let program_files = std::env::var_os("ProgramFiles")
        .or_else(|| std::env::var_os("PROGRAMFILES"))
        .map(PathBuf::from);

    let mut candidates = static_candidates(os, home.as_deref(), program_files.as_deref());
    if os == Os::Windows {
        if let Some(program_files) = &program_files {
            let foundation = program_files.join("Blender Foundation");
            let versions = subdirectories(&foundation);
            let mut versioned = versioned_windows_candidates(&foundation, &versions);
            versioned.append(&mut candidates);
            candidates = versioned;
        }
    }

    if let Some(found) = candidates.into_iter().find(|path| path.is_file()) {
        return Some(found);
    }
    which("blender")
}

/// Look a binary up on `PATH`.
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

/// The names of the subdirectories of `dir`, or nothing if it cannot be read.
fn subdirectories(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_candidates_cover_package_snap_and_flatpak() {
        let home = PathBuf::from("/home/u");
        let candidates = static_candidates(Os::Linux, Some(&home), None);
        assert!(candidates.contains(&PathBuf::from("/usr/bin/blender")));
        assert!(candidates.contains(&PathBuf::from("/snap/bin/blender")));
        assert!(candidates.contains(&PathBuf::from(
            "/var/lib/flatpak/exports/bin/org.blender.Blender"
        )));
        assert!(candidates.contains(&home.join(".local/bin/blender")));
    }

    #[test]
    fn macos_candidates_cover_both_applications_directories() {
        let home = PathBuf::from("/Users/u");
        let candidates = static_candidates(Os::MacOs, Some(&home), None);
        assert!(candidates.contains(&PathBuf::from(
            "/Applications/Blender.app/Contents/MacOS/Blender"
        )));
        assert!(candidates.contains(&home.join("Applications/Blender.app/Contents/MacOS/Blender")));
    }

    #[test]
    fn windows_candidates_need_program_files() {
        let program_files = PathBuf::from("C:\\Program Files");
        let candidates = static_candidates(Os::Windows, None, Some(&program_files));
        assert!(candidates.contains(
            &program_files
                .join("Blender Foundation")
                .join("Blender")
                .join("blender.exe")
        ));
        assert!(static_candidates(Os::Windows, None, None).is_empty());
    }

    #[test]
    fn versioned_windows_candidates_are_newest_first() {
        let foundation = PathBuf::from("C:\\Program Files\\Blender Foundation");
        let versions = vec![
            "Blender 3.6".to_string(),
            "Blender 4.2".to_string(),
            "Blender 4.10".to_string(),
        ];
        let candidates = versioned_windows_candidates(&foundation, &versions);
        assert_eq!(
            candidates,
            vec![
                foundation.join("Blender 4.10").join("blender.exe"),
                foundation.join("Blender 4.2").join("blender.exe"),
                foundation.join("Blender 3.6").join("blender.exe"),
            ]
        );
    }

    #[test]
    fn version_keys_compare_numerically_not_lexically() {
        assert!(version_key("4.10") > version_key("4.2"));
        assert_eq!(version_key("Blender 4.2.1"), vec![4, 2, 1]);
    }

    #[test]
    fn an_unknown_platform_has_no_candidates() {
        assert!(static_candidates(Os::Other, None, None).is_empty());
    }
}
