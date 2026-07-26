//! Resolve NovaVei's one authoritative persistence root.
//!
//! Installed builds retain the operating-system data directory. A portable
//! distribution carries a small sibling marker next to its executable and
//! stores all NovaVei-managed files in an adjacent `novavei` directory. The
//! marker makes the mode explicit: merely placing an installed executable
//! beside a directory named `novavei` can never redirect its data.

use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const PORTABLE_MARKER_FILE: &str = "novavei-portable.json";
pub const PORTABLE_DATA_DIRECTORY: &str = "novavei";
const INSTANCE_LOCK_FILE: &str = ".novavei-instance.lock";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageMode {
    Installed,
    Portable,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageLayout {
    pub mode: StorageMode,
    pub root: PathBuf,
    #[serde(skip_serializing)]
    marker_valid: bool,
}

static STORAGE_LAYOUT: OnceLock<StorageLayout> = OnceLock::new();

/// Resolve storage before application state is constructed. A process keeps
/// this immutable for its lifetime so databases, logs, and worktrees cannot
/// accidentally split across two roots. A portable package refuses to start
/// when its marker or data root is a link/reparse point: silently following one
/// could redirect a USB package into an unrelated local location.
pub fn initialize() -> Result<(), String> {
    let _ = STORAGE_LAYOUT.set(resolve_current_layout());
    ensure_portable_root()
}

pub fn layout() -> &'static StorageLayout {
    STORAGE_LAYOUT.get_or_init(resolve_current_layout)
}

pub fn application_data_dir() -> PathBuf {
    layout().root.clone()
}

pub fn is_portable() -> bool {
    layout().mode == StorageMode::Portable
}

/// Report the mode this process will use after its next launch.  The current
/// process intentionally keeps its resolved root immutable: changing it while
/// SQLite, diagnostics, and the WebView profile are already open would split
/// application state across two locations.
pub fn next_launch_mode() -> Result<StorageMode, String> {
    let marker = portable_marker_path()?;
    match fs::symlink_metadata(&marker) {
        Ok(metadata) if is_plain_file(&metadata) => {
            let contents = fs::read_to_string(&marker)
                .map_err(|_| "read portable mode marker failed".to_string())?;
            if is_portable_marker(&contents) {
                Ok(StorageMode::Portable)
            } else {
                Err("portable mode marker is invalid; repair the portable package before changing modes".to_string())
            }
        }
        Ok(_) => Err("portable mode marker must be a regular non-link file".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(StorageMode::Installed),
        Err(_) => Err("inspect portable mode marker failed".to_string()),
    }
}

/// Change only the marker used by the following application launch.  The
/// marker is deliberately the sole switch: data is not moved, copied, or
/// deleted as a side effect.  This keeps a portable drive's encrypted data
/// isolated from the installed app's per-user data root.
pub fn set_next_launch_mode(mode: StorageMode) -> Result<StorageMode, String> {
    let marker = portable_marker_path()?;
    match mode {
        StorageMode::Portable => match fs::symlink_metadata(&marker) {
            Ok(metadata) if is_plain_file(&metadata) => {
                let contents = fs::read_to_string(&marker)
                    .map_err(|_| "read portable mode marker failed".to_string())?;
                if !is_portable_marker(&contents) {
                    return Err("portable mode marker is invalid; repair the portable package before changing modes".to_string());
                }
            }
            Ok(_) => {
                return Err("portable mode marker must be a regular non-link file".to_string());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&marker)
                    .map_err(|_| "create portable mode marker failed; place the app in a writable folder and try again".to_string())?;
                file.write_all(br#"{"schemaVersion":1,"mode":"portable"}"#)
                    .and_then(|_| file.sync_all())
                    .map_err(|_| "write portable mode marker failed".to_string())?;
            }
            Err(_) => return Err("inspect portable mode marker failed".to_string()),
        },
        StorageMode::Installed => match fs::symlink_metadata(&marker) {
            Ok(metadata) if is_plain_file(&metadata) => {
                let contents = fs::read_to_string(&marker)
                    .map_err(|_| "read portable mode marker failed".to_string())?;
                if !is_portable_marker(&contents) {
                    return Err("portable mode marker is invalid; repair the portable package before changing modes".to_string());
                }
                // Only the validated marker is removed. The sibling portable
                // data directory is intentionally retained for recovery.
                fs::remove_file(&marker)
                    .map_err(|_| "remove portable mode marker failed".to_string())?;
            }
            Ok(_) => {
                return Err("portable mode marker must be a regular non-link file".to_string());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("inspect portable mode marker failed".to_string()),
        },
    }
    next_launch_mode()
}

/// The folder containing the portable executable. This is intentionally
/// derived from the already-validated portable data root instead of trusting
/// the process working directory, which can be inherited from a launcher.
pub fn portable_application_dir() -> Option<PathBuf> {
    is_portable().then(|| application_data_dir().parent().map(Path::to_path_buf))?
}

/// A malformed marker must not silently redirect a partially copied portable
/// package back into installed-mode AppData. Callers can surface a repair
/// action while still keeping every write under the executable's sibling root.
pub fn portable_marker_valid() -> bool {
    !is_portable() || layout().marker_valid
}

/// Held for the process lifetime; dropping it releases the exclusive lock.
pub struct InstanceLock {
    _file: fs::File,
}

/// Two processes sharing one data root would race the SQLite WAL files and
/// the WebView2 profile. Windows enforces this through an exclusive
/// share-mode open of a lock file inside the resolved root; the handle stays
/// open until process exit, so even a killed process releases it. On other
/// platforms the open is best-effort advisory.
pub fn acquire_instance_lock() -> Result<InstanceLock, String> {
    let root = application_data_dir();
    // Installed mode may run before anything created the data directory.
    fs::create_dir_all(&root)
        .map_err(|_| "create application data directory failed".to_string())?;
    let path = root.join(INSTANCE_LOCK_FILE);
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    match options.open(&path) {
        Ok(file) => Ok(InstanceLock { _file: file }),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Err(
            "another NovaVei instance is already using this data folder, or the folder is read-only"
                .to_string(),
        ),
        Err(_) => Err(
            "another NovaVei instance is already using this data folder, or it is not writable"
                .to_string(),
        ),
    }
}

fn resolve_current_layout() -> StorageLayout {
    let Some(executable) = env::current_exe().ok() else {
        return installed_layout();
    };
    let Some(directory) = executable.parent() else {
        return installed_layout();
    };
    let marker = directory.join(PORTABLE_MARKER_FILE);
    match fs::symlink_metadata(&marker) {
        Ok(metadata) => StorageLayout {
            mode: StorageMode::Portable,
            root: directory.join(PORTABLE_DATA_DIRECTORY),
            marker_valid: is_plain_file(&metadata)
                && fs::read_to_string(&marker)
                    .ok()
                    .as_deref()
                    .is_some_and(is_portable_marker),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => installed_layout(),
        // An unreadable marker is deliberately portable-but-invalid. Falling
        // back to AppData could mix a partly copied portable package with the
        // computer's installed identity.
        Err(_) => StorageLayout {
            mode: StorageMode::Portable,
            root: directory.join(PORTABLE_DATA_DIRECTORY),
            marker_valid: false,
        },
    }
}

fn portable_marker_path() -> Result<PathBuf, String> {
    let executable = env::current_exe()
        .map_err(|_| "locate the current application executable failed".to_string())?;
    let directory = executable
        .parent()
        .ok_or_else(|| "locate the current application directory failed".to_string())?;
    Ok(directory.join(PORTABLE_MARKER_FILE))
}

#[cfg(test)]
fn resolve_layout(
    executable: Option<&Path>,
    marker_exists: impl Fn(&Path) -> bool,
    read_marker: impl Fn(&Path) -> Option<String>,
) -> StorageLayout {
    if let Some(executable) = executable {
        if let Some(directory) = executable.parent() {
            let marker = directory.join(PORTABLE_MARKER_FILE);
            if marker_exists(&marker) {
                return StorageLayout {
                    mode: StorageMode::Portable,
                    root: directory.join(PORTABLE_DATA_DIRECTORY),
                    marker_valid: read_marker(&marker)
                        .as_deref()
                        .is_some_and(is_portable_marker),
                };
            }
        }
    }
    installed_layout()
}

fn installed_layout() -> StorageLayout {
    StorageLayout {
        mode: StorageMode::Installed,
        root: installed_data_dir(),
        marker_valid: true,
    }
}

fn ensure_portable_root() -> Result<(), String> {
    if !is_portable() {
        return Ok(());
    }
    if !portable_marker_valid() {
        return Err("portable distribution marker is invalid or linked; repair the portable package before starting".to_string());
    }
    let root = &layout().root;
    match fs::symlink_metadata(root) {
        Ok(metadata) if is_plain_directory(&metadata) => Ok(()),
        Ok(_) => Err("portable data directory must be a regular non-link directory".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(root)
                .map_err(|_| "create portable data directory failed".to_string())?;
            let metadata = fs::symlink_metadata(root)
                .map_err(|_| "inspect portable data directory failed".to_string())?;
            if is_plain_directory(&metadata) {
                Ok(())
            } else {
                Err("portable data directory must be a regular non-link directory".to_string())
            }
        }
        Err(_) => Err("inspect portable data directory failed".to_string()),
    }
}

fn is_plain_file(metadata: &fs::Metadata) -> bool {
    metadata.is_file() && !is_link_or_reparse(metadata)
}

fn is_plain_directory(metadata: &fs::Metadata) -> bool {
    metadata.is_dir() && !is_link_or_reparse(metadata)
}

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}

fn is_portable_marker(value: &str) -> bool {
    serde_json::from_str::<PortableMarker>(value)
        .map(|marker| marker.mode == "portable" && marker.schema_version == 1)
        .unwrap_or(false)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PortableMarker {
    schema_version: u8,
    mode: String,
}

fn installed_data_dir() -> PathBuf {
    for (variable, application_name) in [
        ("LOCALAPPDATA", "NovaVei Agent"),
        ("APPDATA", "NovaVei Agent"),
        ("XDG_DATA_HOME", "novavei-agent"),
    ] {
        if let Ok(value) = env::var(variable) {
            if !value.trim().is_empty() {
                return PathBuf::from(value).join(application_name);
            }
        }
    }
    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".novavei")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_marker_selects_sibling_data_directory() {
        let executable = Path::new(r"E:\NovaVei\NovaVei-portable.exe");
        let layout = resolve_layout(
            Some(executable),
            |_| true,
            |_| Some(r#"{"schemaVersion":1,"mode":"portable"}"#.to_string()),
        );
        assert_eq!(layout.mode, StorageMode::Portable);
        assert_eq!(layout.root, PathBuf::from(r"E:\NovaVei\novavei"));
    }

    #[test]
    fn absent_marker_keeps_installed_mode_but_an_invalid_marker_stays_portable() {
        let executable = Path::new(r"E:\NovaVei\NovaVei.exe");
        assert_eq!(
            resolve_layout(Some(executable), |_| false, |_| None).mode,
            StorageMode::Installed
        );
        let invalid = resolve_layout(Some(executable), |_| true, |_| Some("not-json".to_string()));
        assert_eq!(invalid.mode, StorageMode::Portable);
        assert!(
            !invalid.marker_valid,
            "an invalid sibling marker must not fall back to installed AppData"
        );
    }

    #[test]
    fn portable_application_directory_is_the_parent_of_its_data_root() {
        let layout = StorageLayout {
            mode: StorageMode::Portable,
            root: PathBuf::from(r"E:\NovaVei\novavei"),
            marker_valid: true,
        };
        assert_eq!(
            layout.root.parent(),
            Some(Path::new(r"E:\NovaVei")),
            "portable projects must use the EXE's folder, not the data folder"
        );
    }
}
