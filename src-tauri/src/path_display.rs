//! Human-facing path formatting for Windows extended / device paths.
//!
//! `std::fs::canonicalize` on Windows often returns extended-length paths such as
//! `\\?\E:\project` or `\\?\UNC\server\share`. Those forms are correct for the
//! Win32 long-path API, but must not be shown in the UI, stored as session `cwd`,
//! or returned through Tauri IPC as the user-visible workdir string.
//!
//! Internal `PathBuf` values may keep the verbatim form for filesystem checks
//! (`starts_with`, `strip_prefix`). Only convert at the display / persistence
//! boundary with [`path_for_display`].

use std::path::{Component, Path, Prefix};

/// Format a path for UI, settings, session metadata, and IPC.
///
/// On Windows, strips the `\\?\` / `//?/` extended prefix and rewrites
/// `\\?\UNC\...` to the ordinary `\\server\share` form. On other platforms this
/// is equivalent to `path.display()`.
pub fn path_for_display(path: &Path) -> String {
    #[cfg(windows)]
    {
        if let Some(formatted) = format_windows_display_path(path) {
            return formatted;
        }
    }
    path.display().to_string()
}

/// Same as [`path_for_display`], for already-owned strings (error messages,
/// legacy stored values, or paths that never went through `PathBuf`).
pub fn path_string_for_display(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    path_for_display(Path::new(trimmed))
}

#[cfg(windows)]
fn format_windows_display_path(path: &Path) -> Option<String> {
    let mut components = path.components();
    let prefix = match components.next() {
        Some(Component::Prefix(prefix_component)) => prefix_component.kind(),
        _ => return strip_extended_string_prefix(&path.display().to_string()),
    };

    match prefix {
        Prefix::VerbatimDisk(disk) => {
            let drive = (disk as char).to_ascii_uppercase();
            let mut formatted = format!("{drive}:");
            for component in components {
                match component {
                    Component::RootDir => formatted.push('\\'),
                    Component::Normal(segment) => {
                        if !formatted.ends_with('\\') {
                            formatted.push('\\');
                        }
                        formatted.push_str(&segment.to_string_lossy());
                    }
                    Component::CurDir => {}
                    Component::ParentDir => {
                        if !formatted.ends_with('\\') {
                            formatted.push('\\');
                        }
                        formatted.push_str("..");
                    }
                    Component::Prefix(_) => return None,
                }
            }
            if formatted.len() == 2 {
                formatted.push('\\');
            }
            Some(formatted)
        }
        Prefix::VerbatimUNC(server, share) => {
            let mut formatted = format!(
                "\\\\{}\\{}",
                server.to_string_lossy(),
                share.to_string_lossy()
            );
            for component in components {
                if let Component::Normal(segment) = component {
                    formatted.push('\\');
                    formatted.push_str(&segment.to_string_lossy());
                }
            }
            Some(formatted)
        }
        Prefix::Verbatim(verbatim) => {
            // Device paths such as `\\?\GLOBALROOT\...` are not ordinary
            // filesystem roots; surface a cleaned string without the marker.
            let mut formatted = verbatim.to_string_lossy().into_owned();
            for component in components {
                if let Component::Normal(segment) = component {
                    if !formatted.is_empty() && !formatted.ends_with('\\') {
                        formatted.push('\\');
                    }
                    formatted.push_str(&segment.to_string_lossy());
                }
            }
            Some(formatted)
        }
        _ => strip_extended_string_prefix(&path.display().to_string()),
    }
}

/// Last-resort string cleanup when component parsing does not apply.
fn strip_extended_string_prefix(raw: &str) -> Option<String> {
    let value = raw.trim();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        return Some(format!(r"\\{rest}"));
    }
    if let Some(rest) = value.strip_prefix(r"\\?\") {
        return Some(rest.to_string());
    }
    if let Some(rest) = value.strip_prefix("//?/UNC/") {
        return Some(format!("//{rest}"));
    }
    if let Some(rest) = value.strip_prefix("//?/") {
        return Some(rest.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn strips_verbatim_disk_prefix_from_string() {
        let displayed = path_string_for_display(r"\\?\D:\workspace\project");
        assert_eq!(displayed, r"D:\workspace\project");
    }

    #[test]
    fn strips_verbatim_unc_prefix_from_string() {
        let displayed = path_string_for_display(r"\\?\UNC\server\share\folder");
        assert_eq!(displayed, r"\\server\share\folder");
    }

    #[test]
    fn leaves_ordinary_windows_paths_unchanged() {
        let ordinary = r"D:\workspace\project";
        assert_eq!(path_string_for_display(ordinary), ordinary);
    }

    #[test]
    fn strips_verbatim_disk_pathbuf_components() {
        let path = PathBuf::from(r"\\?\C:\Users\example\project");
        assert_eq!(path_for_display(&path), r"C:\Users\example\project");
    }
}
