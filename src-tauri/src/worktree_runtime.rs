//! Native Git worktree and reviewed-patch boundary for isolated subagents.
//!
//! This module deliberately invokes a small fixed Git command set rather than
//! exposing a shell. A worktree is an isolation aid, not a security sandbox:
//! the child runtime must still receive a restricted tool registry.

use crate::diagnostics;
use crate::path_display::path_for_display;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const WORKTREES_DIRECTORY_NAME: &str = "subagent-worktrees";
const PATCHES_DIRECTORY_NAME: &str = "subagent-patches";
const MAX_TASK_IDENTIFIER_BYTES: usize = 128;
const MAX_PATCH_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeLease {
    pub task_id: String,
    pub repository_root: String,
    pub worktree_path: String,
    pub base_commit: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreePatch {
    pub task_id: String,
    pub base_commit: String,
    pub digest: String,
    pub changed_paths: Vec<String>,
    pub patch: String,
}

/// Native-only Git identity captured before the child receives its worktree.
///
/// A linked worktree's top-level `.git` is a writable gitfile, so it is never
/// used again to discover the index for review collection. This record lives
/// beside the native-managed review patch rather than in the child checkout.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WorktreeGitIdentity {
    repository_root: PathBuf,
    worktree_path: PathBuf,
    git_dir: PathBuf,
    common_git_dir: PathBuf,
}

pub fn managed_storage_root() -> PathBuf {
    diagnostics::application_data_dir().join(WORKTREES_DIRECTORY_NAME)
}

fn patch_storage_root(storage_root: &Path) -> PathBuf {
    storage_root
        .parent()
        .unwrap_or(storage_root)
        .join(PATCHES_DIRECTORY_NAME)
}

/// Provision an isolated detached worktree at an app-managed location.
///
/// The fixed Git command is equivalent to `git worktree add --detach`; no
/// branch is created and no model-selected command is ever executed.
pub fn provision_isolated_worktree(
    storage_root: &Path,
    task_id: &str,
    selected_workdir: &Path,
) -> Result<WorktreeLease, String> {
    validate_task_id(task_id)?;
    let selected_root = fs::canonicalize(selected_workdir)
        .map_err(|error| format!("canonicalize selected workdir: {error}"))?;
    let repository_root = git_repository_root(&selected_root)?;
    if repository_root != selected_root {
        return Err("worktree tasks require the selected Git repository root".to_string());
    }
    let base_commit = git_stdout(&repository_root, ["rev-parse", "HEAD"])?;
    if base_commit.len() != 40 || !base_commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Git repository did not return a valid base commit".to_string());
    }

    fs::create_dir_all(storage_root)
        .map_err(|error| format!("create managed worktree directory: {error}"))?;
    let canonical_storage_root =
        canonical_existing_directory(storage_root, "managed worktree directory")?;
    let worktree_path = canonical_storage_root.join(task_id);
    if worktree_path.exists() {
        return Err("managed worktree path is already in use".to_string());
    }
    // `canonicalize` returns a `\\?\`-prefixed path on Windows. Keep that
    // form for the filesystem boundary checks below, but pass Git its regular
    // Win32 path syntax: Git for Windows does not reliably accept the
    // verbatim prefix as a worktree destination argument. On other platforms,
    // retain the prior rejection of a non-UTF-8 destination argument.
    #[cfg(windows)]
    let git_worktree_path = path_for_display(&worktree_path);
    #[cfg(not(windows))]
    let git_worktree_path = worktree_path
        .to_str()
        .ok_or_else(|| "managed worktree path is not valid UTF-8".to_string())?
        .to_string();
    run_git(
        &repository_root,
        [
            "worktree",
            "add",
            "--detach",
            &git_worktree_path,
            &base_commit,
        ],
    )?;
    let canonical_worktree = fs::canonicalize(&worktree_path)
        .map_err(|error| format!("canonicalize created worktree: {error}"))?;
    if !canonical_worktree.starts_with(&canonical_storage_root) {
        return Err("created worktree escaped the managed directory".to_string());
    }
    let identity = match capture_worktree_git_identity(&repository_root, &canonical_worktree) {
        Ok(identity) => identity,
        Err(error) => {
            let _ = run_git(
                &repository_root,
                ["worktree", "remove", "--force", &git_worktree_path],
            );
            return Err(error);
        }
    };
    if let Err(error) = write_worktree_git_identity(storage_root, task_id, &identity) {
        let _ = run_git(
            &repository_root,
            ["worktree", "remove", "--force", &git_worktree_path],
        );
        return Err(error);
    }
    Ok(WorktreeLease {
        task_id: task_id.to_string(),
        repository_root: path_for_display(&repository_root),
        worktree_path: path_for_display(&canonical_worktree),
        base_commit,
    })
}

/// Generate a canonical patch for the worktree without applying it anywhere.
/// `git add -N` is limited to the isolated worktree index so untracked files
/// appear in the binary diff; `git reset --mixed` restores that index intent
/// while leaving child working-tree changes untouched.
pub fn collect_review_patch(
    storage_root: &Path,
    lease: &WorktreeLease,
) -> Result<WorktreePatch, String> {
    validate_task_id(&lease.task_id)?;
    let repository_root = canonical_existing_directory(&lease.repository_root, "repository root")?;
    let worktree_path =
        canonical_managed_worktree(storage_root, &lease.task_id, &lease.worktree_path)?;
    let identity =
        load_worktree_git_identity(storage_root, lease, &repository_root, &worktree_path)?;
    verify_worktree_git_identity(&identity)?;
    let current_base = git_stdout_for_worktree(&identity, ["rev-parse", "HEAD"])?;
    if current_base != lease.base_commit {
        return Err("isolated worktree base commit changed unexpectedly".to_string());
    }

    run_git_for_worktree(&identity, ["add", "--intent-to-add", "--", "."])?;
    let diff_result =
        git_output_for_worktree(&identity, ["diff", "--binary", "--no-ext-diff", "--"]);
    let names_result = git_output_for_worktree(&identity, ["diff", "--name-only", "--"]);
    let reset_result = run_git_for_worktree(&identity, ["reset", "--mixed", "--quiet", "--"]);
    let diff = diff_result?;
    let names = names_result?;
    reset_result?;
    if !diff.status.success() {
        return Err(git_failure("collect Git worktree patch", &diff));
    }
    if !names.status.success() {
        return Err(git_failure("list Git worktree patch paths", &names));
    }
    if diff.stdout.len() > MAX_PATCH_BYTES {
        return Err("review patch exceeds the 16 MiB safety limit".to_string());
    }
    let patch = String::from_utf8(diff.stdout)
        .map_err(|_| "Git generated a non-UTF-8 review patch".to_string())?;
    let digest = patch_digest(&patch);
    let changed_paths = String::from_utf8_lossy(&names.stdout)
        .lines()
        .filter_map(safe_changed_path)
        .collect::<Vec<_>>();
    write_patch(storage_root, &lease.task_id, &patch)?;
    Ok(WorktreePatch {
        task_id: lease.task_id.clone(),
        base_commit: lease.base_commit.clone(),
        digest,
        changed_paths,
        patch,
    })
}

/// Return the exact stored patch for review. The path remains native-only.
pub fn load_review_patch(
    storage_root: &Path,
    task_id: &str,
    expected_digest: &str,
    base_commit: &str,
) -> Result<WorktreePatch, String> {
    validate_task_id(task_id)?;
    validate_digest(expected_digest)?;
    let patch = fs::read_to_string(patch_path(storage_root, task_id))
        .map_err(|error| format!("read managed review patch: {error}"))?;
    if patch.len() > MAX_PATCH_BYTES {
        return Err("stored review patch exceeds the 16 MiB safety limit".to_string());
    }
    let digest = patch_digest(&patch);
    if digest != expected_digest {
        return Err("stored review patch digest does not match the approved review".to_string());
    }
    let changed_paths = patch_changed_paths(&patch);
    Ok(WorktreePatch {
        task_id: task_id.to_string(),
        base_commit: base_commit.to_string(),
        digest,
        changed_paths,
        patch,
    })
}

/// Apply the exact reviewed patch only after a native user confirmation.
/// The preflight is strictly `git apply --check`; no three-way merge, commit,
/// rebase, or file-copy fallback exists in this boundary.
pub fn apply_reviewed_patch(
    storage_root: &Path,
    task_id: &str,
    repository_root: &Path,
    base_commit: &str,
    expected_digest: &str,
) -> Result<WorktreePatch, String> {
    let repository_root = canonical_existing_directory(repository_root, "repository root")?;
    let current_base = git_stdout(&repository_root, ["rev-parse", "HEAD"])?;
    if current_base != base_commit {
        return Err("parent repository base commit changed since patch review".to_string());
    }
    let patch = load_review_patch(storage_root, task_id, expected_digest, base_commit)?;
    let patch_file = patch_path(storage_root, task_id);
    run_git(
        &repository_root,
        [
            "apply",
            "--check",
            "--whitespace=nowarn",
            patch_file
                .to_str()
                .ok_or_else(|| "managed patch path is not valid UTF-8".to_string())?,
        ],
    )?;
    request_native_apply_confirmation(&patch)?;
    run_git(
        &repository_root,
        [
            "apply",
            "--whitespace=nowarn",
            patch_file
                .to_str()
                .ok_or_else(|| "managed patch path is not valid UTF-8".to_string())?,
        ],
    )?;
    Ok(patch)
}

/// Remove a reviewed child checkout only after an explicit user discard.
/// `--force` is intentionally confined to this fixed native lifecycle: it is
/// never selected by model output, renderer arguments, or parent cancellation.
pub fn discard_reviewed_worktree(storage_root: &Path, lease: &WorktreeLease) -> Result<(), String> {
    let repository_root = canonical_existing_directory(&lease.repository_root, "repository root")?;
    let worktree_path =
        canonical_managed_worktree(storage_root, &lease.task_id, &lease.worktree_path)?;
    request_native_discard_confirmation(&lease.task_id)?;
    run_git(
        &repository_root,
        [
            "worktree",
            "remove",
            "--force",
            worktree_path
                .to_str()
                .ok_or_else(|| "managed worktree path is not valid UTF-8".to_string())?,
        ],
    )?;
    remove_patch_file(storage_root, &lease.task_id)
}

/// Discard needs an explicit caller action. Git refuses to remove a dirty
/// worktree by default, preserving failed or unreviewed child changes.
pub fn discard_isolated_worktree(storage_root: &Path, lease: &WorktreeLease) -> Result<(), String> {
    let repository_root = canonical_existing_directory(&lease.repository_root, "repository root")?;
    let worktree_path =
        canonical_managed_worktree(storage_root, &lease.task_id, &lease.worktree_path)?;
    run_git(
        &repository_root,
        [
            "worktree",
            "remove",
            worktree_path
                .to_str()
                .ok_or_else(|| "managed worktree path is not valid UTF-8".to_string())?,
        ],
    )?;
    remove_patch_file(storage_root, &lease.task_id)
}

fn remove_patch_file(storage_root: &Path, task_id: &str) -> Result<(), String> {
    remove_managed_file(
        worktree_identity_path(storage_root, task_id),
        "worktree identity",
    )?;
    remove_managed_file(patch_path(storage_root, task_id), "review patch")
}

fn remove_managed_file(path: PathBuf, label: &str) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("remove managed {label}: {error}")),
    }
}

fn git_repository_root(selected_root: &Path) -> Result<PathBuf, String> {
    // `git rev-parse --show-toplevel` proves both Git membership and the
    // canonical repository root without auto-initializing arbitrary folders.
    let inside = git_stdout(selected_root, ["rev-parse", "--is-inside-work-tree"])?;
    if inside != "true" {
        return Err("selected workdir is not a Git worktree".to_string());
    }
    let root = git_stdout(selected_root, ["rev-parse", "--show-toplevel"])?;
    canonical_existing_directory(Path::new(&root), "Git repository root")
}

fn capture_worktree_git_identity(
    repository_root: &Path,
    worktree_path: &Path,
) -> Result<WorktreeGitIdentity, String> {
    let repository_root = canonical_existing_directory(repository_root, "repository root")?;
    let worktree_path = canonical_existing_directory(worktree_path, "isolated worktree")?;
    let common_git_dir = git_directory(
        worktree_path.as_path(),
        "--git-common-dir",
        "Git common directory",
    )?;
    let repository_common_git_dir = git_directory(
        repository_root.as_path(),
        "--git-common-dir",
        "repository Git directory",
    )?;
    if common_git_dir != repository_common_git_dir {
        return Err("isolated worktree is not attached to the selected repository".to_string());
    }
    let git_dir = git_directory(
        worktree_path.as_path(),
        "--git-dir",
        "isolated worktree Git directory",
    )?;
    let identity = WorktreeGitIdentity {
        repository_root,
        worktree_path,
        git_dir,
        common_git_dir,
    };
    validate_worktree_git_identity_shape(&identity)?;
    verify_worktree_git_identity(&identity)?;
    Ok(identity)
}

fn git_directory(current_dir: &Path, argument: &str, label: &str) -> Result<PathBuf, String> {
    let raw = git_stdout(current_dir, ["rev-parse", argument])?;
    canonical_command_directory(current_dir, &raw, label)
}

fn canonical_command_directory(
    current_dir: &Path,
    raw: &str,
    label: &str,
) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_dir.join(path)
    };
    canonical_existing_directory(candidate, label)
}

fn validate_worktree_git_identity_shape(identity: &WorktreeGitIdentity) -> Result<(), String> {
    let expected_directory = identity.common_git_dir.join("worktrees");
    if identity.git_dir == identity.common_git_dir
        || !identity.git_dir.starts_with(expected_directory)
    {
        return Err(
            "isolated worktree Git directory is outside the managed Git worktree area".to_string(),
        );
    }
    Ok(())
}

fn verify_worktree_git_identity(identity: &WorktreeGitIdentity) -> Result<(), String> {
    validate_worktree_git_identity_shape(identity)?;
    let reported_worktree = git_stdout_for_worktree(identity, ["rev-parse", "--show-toplevel"])?;
    let reported_worktree = canonical_command_directory(
        &identity.worktree_path,
        &reported_worktree,
        "configured Git worktree",
    )?;
    if reported_worktree != identity.worktree_path {
        return Err(
            "isolated worktree Git identity no longer matches its managed checkout".to_string(),
        );
    }
    let reported_git_dir = git_stdout_for_worktree(identity, ["rev-parse", "--git-dir"])?;
    let reported_git_dir = canonical_command_directory(
        &identity.worktree_path,
        &reported_git_dir,
        "configured isolated worktree Git directory",
    )?;
    if reported_git_dir != identity.git_dir {
        return Err("isolated worktree Git directory changed unexpectedly".to_string());
    }
    let reported_common_git_dir =
        git_stdout_for_worktree(identity, ["rev-parse", "--git-common-dir"])?;
    let reported_common_git_dir = canonical_command_directory(
        &identity.worktree_path,
        &reported_common_git_dir,
        "configured Git common directory",
    )?;
    if reported_common_git_dir != identity.common_git_dir {
        return Err("isolated worktree Git common directory changed unexpectedly".to_string());
    }
    Ok(())
}

fn canonical_existing_directory(path: impl AsRef<Path>, label: &str) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(path.as_ref())
        .map_err(|error| format!("canonicalize {label}: {error}"))?;
    if !canonical.is_dir() {
        return Err(format!("{label} is not a directory"));
    }
    Ok(canonical)
}

fn canonical_managed_worktree(
    storage_root: &Path,
    task_id: &str,
    advertised_path: &str,
) -> Result<PathBuf, String> {
    validate_task_id(task_id)?;
    let storage_root = canonical_existing_directory(storage_root, "managed worktree directory")?;
    let expected = storage_root.join(task_id);
    let worktree_path = canonical_existing_directory(advertised_path, "managed worktree")?;
    if worktree_path != expected || !worktree_path.starts_with(&storage_root) {
        return Err("worktree path is outside native-managed storage".to_string());
    }
    Ok(worktree_path)
}

fn git_output<const COUNT: usize>(
    current_dir: &Path,
    args: [&str; COUNT],
) -> Result<Output, String> {
    Command::new("git")
        .args(args)
        .current_dir(current_dir)
        .output()
        .map_err(|error| format!("start fixed Git command: {error}"))
}

fn git_output_for_worktree<const COUNT: usize>(
    identity: &WorktreeGitIdentity,
    args: [&str; COUNT],
) -> Result<Output, String> {
    let mut command = Command::new("git");
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
    ] {
        command.env_remove(name);
    }
    command
        // Git for Windows rejects the verbatim `\\?\` paths returned by
        // `canonicalize`, so retain those paths only for native comparisons.
        .env("GIT_DIR", path_for_display(&identity.git_dir))
        .env("GIT_WORK_TREE", path_for_display(&identity.worktree_path))
        .args(args)
        .current_dir(path_for_display(&identity.worktree_path))
        .output()
        .map_err(|error| format!("start fixed isolated-worktree Git command: {error}"))
}

fn run_git<const COUNT: usize>(current_dir: &Path, args: [&str; COUNT]) -> Result<(), String> {
    let output = git_output(current_dir, args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(git_failure("run fixed Git command", &output))
    }
}

fn run_git_for_worktree<const COUNT: usize>(
    identity: &WorktreeGitIdentity,
    args: [&str; COUNT],
) -> Result<(), String> {
    let output = git_output_for_worktree(identity, args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(git_failure(
            "run fixed isolated-worktree Git command",
            &output,
        ))
    }
}

fn git_stdout<const COUNT: usize>(
    current_dir: &Path,
    args: [&str; COUNT],
) -> Result<String, String> {
    let output = git_output(current_dir, args)?;
    if !output.status.success() {
        return Err(git_failure("read fixed Git command output", &output));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|_| "Git command returned non-UTF-8 text".to_string())
}

fn git_stdout_for_worktree<const COUNT: usize>(
    identity: &WorktreeGitIdentity,
    args: [&str; COUNT],
) -> Result<String, String> {
    let output = git_output_for_worktree(identity, args)?;
    if !output.status.success() {
        return Err(git_failure(
            "read fixed isolated-worktree Git command output",
            &output,
        ));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|_| "Git command returned non-UTF-8 text".to_string())
}

fn git_failure(operation: &str, output: &Output) -> String {
    let code = output.status.code().unwrap_or(-1);
    format!("{operation} failed with exit code {code}")
}

fn validate_task_id(task_id: &str) -> Result<(), String> {
    let task_id = task_id.trim();
    if task_id.is_empty()
        || task_id.len() > MAX_TASK_IDENTIFIER_BYTES
        || !task_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("worktree task identifier is invalid".to_string());
    }
    Ok(())
}

fn validate_digest(digest: &str) -> Result<(), String> {
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("approved patch digest is invalid".to_string());
    }
    Ok(())
}

fn patch_path(storage_root: &Path, task_id: &str) -> PathBuf {
    patch_storage_root(storage_root).join(format!("{task_id}.patch"))
}

fn worktree_identity_path(storage_root: &Path, task_id: &str) -> PathBuf {
    patch_storage_root(storage_root).join(format!("{task_id}.worktree.json"))
}

fn write_worktree_git_identity(
    storage_root: &Path,
    task_id: &str,
    identity: &WorktreeGitIdentity,
) -> Result<(), String> {
    let directory = patch_storage_root(storage_root);
    fs::create_dir_all(&directory)
        .map_err(|error| format!("create managed patch directory: {error}"))?;
    let contents = serde_json::to_vec(identity)
        .map_err(|error| format!("serialize managed worktree identity: {error}"))?;
    fs::write(worktree_identity_path(storage_root, task_id), contents)
        .map_err(|error| format!("write managed worktree identity: {error}"))
}

fn load_worktree_git_identity(
    storage_root: &Path,
    lease: &WorktreeLease,
    repository_root: &Path,
    worktree_path: &Path,
) -> Result<WorktreeGitIdentity, String> {
    let contents = fs::read(worktree_identity_path(storage_root, &lease.task_id))
        .map_err(|error| format!("read managed worktree identity: {error}"))?;
    let mut identity = serde_json::from_slice::<WorktreeGitIdentity>(&contents)
        .map_err(|error| format!("parse managed worktree identity: {error}"))?;
    identity.repository_root =
        canonical_existing_directory(&identity.repository_root, "stored repository root")?;
    identity.worktree_path =
        canonical_existing_directory(&identity.worktree_path, "stored isolated worktree")?;
    identity.git_dir =
        canonical_existing_directory(&identity.git_dir, "stored isolated worktree Git directory")?;
    identity.common_git_dir =
        canonical_existing_directory(&identity.common_git_dir, "stored Git common directory")?;
    if identity.repository_root != repository_root || identity.worktree_path != worktree_path {
        return Err("managed worktree identity does not match the stored task lease".to_string());
    }
    validate_worktree_git_identity_shape(&identity)?;
    Ok(identity)
}

fn write_patch(storage_root: &Path, task_id: &str, patch: &str) -> Result<(), String> {
    let directory = patch_storage_root(storage_root);
    fs::create_dir_all(&directory)
        .map_err(|error| format!("create managed patch directory: {error}"))?;
    fs::write(patch_path(storage_root, task_id), patch)
        .map_err(|error| format!("write managed review patch: {error}"))
}

fn patch_digest(patch: &str) -> String {
    let digest = Sha256::digest(patch.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn safe_changed_path(value: &str) -> Option<String> {
    let path = value.trim();
    if path.is_empty()
        || path.len() > 4096
        || path.contains('\0')
        || Path::new(path).is_absolute()
        || path
            .split('/')
            .any(|component| matches!(component, "" | "." | ".."))
    {
        return None;
    }
    Some(path.replace('\\', "/"))
}

fn patch_changed_paths(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter_map(|line| line.strip_prefix("+++ b/"))
        .filter_map(safe_changed_path)
        .collect()
}

fn request_native_apply_confirmation(patch: &WorktreePatch) -> Result<(), String> {
    let response = rfd::MessageDialog::new()
        .set_title("Apply Reviewed NovaVei Patch")
        .set_description(format!(
            "Apply the reviewed patch for task {}?\n\n{} changed path(s), SHA-256 {}.\n\nThis updates the current repository and cannot be undone automatically.",
            patch.task_id,
            patch.changed_paths.len(),
            patch.digest,
        ))
        .set_buttons(rfd::MessageButtons::OkCancel)
        .show();
    if response == rfd::MessageDialogResult::Ok {
        Ok(())
    } else {
        Err("user_cancelled".to_string())
    }
}

fn request_native_discard_confirmation(task_id: &str) -> Result<(), String> {
    let response = rfd::MessageDialog::new()
        .set_title("Discard NovaVei Worktree")
        .set_description(format!(
            "Discard the isolated worktree for task {task_id}?\n\nIts unmerged changes and stored review patch will be permanently removed."
        ))
        .set_buttons(rfd::MessageButtons::OkCancel)
        .show();
    if response == rfd::MessageDialogResult::Ok {
        Ok(())
    } else {
        Err("user_cancelled".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_directory(label: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("novavei-{label}-{timestamp}"))
    }

    fn run_test_git(repository: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repository)
            .output()
            .expect("Git must be available for the worktree integration test");
        assert!(
            output.status.success(),
            "git command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn test_git_stdout(repository: &Path, args: &[&str]) -> Vec<u8> {
        let output = Command::new("git")
            .args(args)
            .current_dir(repository)
            .output()
            .expect("Git must be available for the worktree integration test");
        assert!(
            output.status.success(),
            "git command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    fn assert_parent_index_is_unchanged(repository: &Path, expected_index_diff: &[u8]) {
        assert_eq!(
            test_git_stdout(repository, &["diff", "--cached", "--binary"]),
            expected_index_diff,
            "collecting a child patch must not alter the parent index"
        );
    }

    #[test]
    fn isolated_worktree_patch_includes_untracked_files_without_touching_parent() {
        let root = temporary_directory("worktree-test");
        let repository = root.join("repository");
        let storage = root.join("managed-worktrees");
        fs::create_dir_all(&repository).expect("create temporary repository");
        run_test_git(&repository, &["init"]);
        run_test_git(
            &repository,
            &["config", "user.email", "test@example.invalid"],
        );
        run_test_git(&repository, &["config", "user.name", "NovaVei Test"]);
        fs::write(repository.join("tracked.txt"), "base\n").expect("write base file");
        run_test_git(&repository, &["add", "tracked.txt"]);
        run_test_git(&repository, &["commit", "-m", "base"]);

        let lease = provision_isolated_worktree(&storage, "task_test", &repository)
            .expect("provision detached worktree");
        let worktree = PathBuf::from(&lease.worktree_path);
        fs::write(worktree.join("tracked.txt"), "changed\n").expect("change tracked file");
        fs::write(worktree.join("new.txt"), "new\n").expect("create untracked file");
        let patch = collect_review_patch(&storage, &lease).expect("collect review patch");

        assert!(patch.patch.contains("tracked.txt"));
        assert!(patch.patch.contains("new.txt"));
        assert!(patch.changed_paths.contains(&"tracked.txt".to_string()));
        assert!(patch.changed_paths.contains(&"new.txt".to_string()));
        run_test_git(&repository, &["diff", "--quiet"]);
        assert_parent_index_is_unchanged(&repository, b"");

        // Test teardown may force-remove its disposable fixture. Production
        // discard intentionally never supplies a force flag.
        run_test_git(
            &repository,
            &["worktree", "remove", "--force", &lease.worktree_path],
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn tampered_worktree_gitfile_cannot_redirect_patch_collection_to_parent_index() {
        let root = temporary_directory("worktree-gitfile-tamper");
        let repository = root.join("repository");
        let storage = root.join("managed-worktrees");
        fs::create_dir_all(&repository).expect("create temporary repository");
        run_test_git(&repository, &["init"]);
        run_test_git(
            &repository,
            &["config", "user.email", "test@example.invalid"],
        );
        run_test_git(&repository, &["config", "user.name", "NovaVei Test"]);
        fs::write(repository.join("tracked.txt"), "base\n").expect("write base file");
        run_test_git(&repository, &["add", "tracked.txt"]);
        run_test_git(&repository, &["commit", "-m", "base"]);
        fs::write(repository.join("parent-staged.txt"), "keep this staged\n")
            .expect("write staged parent file");
        run_test_git(&repository, &["add", "parent-staged.txt"]);
        let parent_index_before = test_git_stdout(&repository, &["diff", "--cached", "--binary"]);

        let lease = provision_isolated_worktree(&storage, "task_tamper", &repository)
            .expect("provision detached worktree");
        let worktree = PathBuf::from(&lease.worktree_path);
        let parent_git_dir =
            fs::canonicalize(repository.join(".git")).expect("parent Git directory should exist");
        let expected_gitfile = fs::read_to_string(worktree.join(".git"))
            .expect("created worktree should have a Git gitfile");
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", path_for_display(&parent_git_dir)),
        )
        .expect("tamper child gitfile");
        fs::write(worktree.join("tracked.txt"), "child change\n")
            .expect("change tracked child file");
        fs::write(worktree.join("child-new.txt"), "child new\n")
            .expect("create untracked child file");

        let patch = collect_review_patch(&storage, &lease)
            .expect("collection must use the provisioned Git identity");
        assert!(patch.patch.contains("tracked.txt"));
        assert!(patch.patch.contains("child-new.txt"));
        assert_parent_index_is_unchanged(&repository, &parent_index_before);

        // Test teardown may force-remove its disposable fixture. Production
        // discard intentionally never supplies a force flag.
        fs::write(worktree.join(".git"), expected_gitfile)
            .expect("restore child gitfile for Git teardown");
        run_test_git(
            &repository,
            &["worktree", "remove", "--force", &lease.worktree_path],
        );
        let _ = fs::remove_dir_all(root);
    }
}
