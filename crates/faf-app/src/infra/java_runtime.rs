//! Shared discovery of the Java runtime used by native FAF tools.
//!
//! Both the ICE adapter and current Neroxis generator require a modern JVM.
//! Looking up `java` independently made the adapter reuse the official FAF
//! client's runtime while map generation still found an obsolete system Java.

use std::path::PathBuf;

/// Resolve an explicit override, a bundled runtime, or a reference client's
/// runtime before falling back to `PATH`.
pub(crate) fn preferred_java_path() -> String {
    if let Some(path) = crate::infra::paths::java_path() {
        return path.to_string_lossy().into_owned();
    }
    if let Ok(path) = std::env::var("FAF_JAVA_PATH") {
        if !path.trim().is_empty() {
            return path;
        }
    }

    // Beside the client first, and nowhere the working directory can reach:
    // see `infra::helper_search_roots`. A `java.exe` in a folder the client
    // happened to be started from is not the one to run.
    let mut roots = crate::infra::helper_search_roots();

    // The named install locations below are the documented places a JRE lives,
    // and all of them are directories an ordinary user cannot write to (or, in
    // JAVA_HOME's case, one the user set themselves). They come after the
    // bundled candidates so a shipped runtime always wins.
    if cfg!(windows) {
        for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(directory) = std::env::var_os(variable) {
                let directory = PathBuf::from(directory);
                roots.push(directory.join("FAF Client"));
                roots.push(directory.join("Downlord's FAF Client"));
            }
        }
        if let Some(directory) = std::env::var_os("LOCALAPPDATA") {
            roots.push(PathBuf::from(directory).join("Programs").join("FAF Client"));
        }
    }
    if let Some(bundled) = resolve_java_from_roots(&roots) {
        return bundled.to_string_lossy().into_owned();
    }

    // On a Mac the runtime the setup assistant installed, ahead of anything
    // the machine happens to have: the client was tested against that one,
    // and a system Java is a version nobody here chose.
    if let Some(java) = crate::infra::mac_setup::managed_java() {
        return java.to_string_lossy().into_owned();
    }

    let mut roots = Vec::new();
    if let Some(java_home) = std::env::var_os("JAVA_HOME") {
        roots.push(PathBuf::from(java_home));
    }

    resolve_java_from_roots(&roots)
        .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "java.exe" } else { "java" }))
        .to_string_lossy()
        .into_owned()
}

fn resolve_java_from_roots(roots: &[PathBuf]) -> Option<PathBuf> {
    let executable = if cfg!(windows) { "java.exe" } else { "java" };
    roots
        .iter()
        .flat_map(|root| {
            [
                root.join("jre").join("bin").join(executable),
                root.join("natives")
                    .join("jre")
                    .join("bin")
                    .join(executable),
                root.join("resources")
                    .join("natives")
                    .join("jre")
                    .join("bin")
                    .join(executable),
                root.join("bin").join(executable),
            ]
        })
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_runtime_bundled_like_the_reference_clients() {
        let directory = tempfile::tempdir().unwrap();
        let executable = if cfg!(windows) { "java.exe" } else { "java" };
        let java = directory.path().join("jre").join("bin").join(executable);
        std::fs::create_dir_all(java.parent().unwrap()).unwrap();
        std::fs::write(&java, b"test runtime").unwrap();

        assert_eq!(
            resolve_java_from_roots(&[directory.path().to_path_buf()]),
            Some(java)
        );
    }
}
