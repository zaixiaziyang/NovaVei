import fs from "node:fs";
import { dirname, extname, join } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const projectRoot = join(scriptDirectory, "..");

function read(relativePath) {
  return fs.readFileSync(join(projectRoot, relativePath), "utf8");
}

function sourceFiles(root) {
  return fs.readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
    const path = join(root, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    return [path];
  });
}

function quotedValues(value) {
  return [...value.matchAll(/"([^"]+)"/g)].map((match) => match[1]);
}

const defaultCapability = JSON.parse(
  read("src-tauri/capabilities/default.json"),
);
const tauriConfig = JSON.parse(read("src-tauri/tauri.conf.json"));
const permissionSource = read("src-tauri/permissions/app.toml");
const embeddedRuntime = read("src/runtime/embedded.ts");
const backendSource = read("src-tauri/src/backend.rs");
const activePermissions = new Set(defaultCapability.permissions);
const allowedCommands = new Set();
const permissionCommands = new Map();

for (const block of permissionSource.split("[[permission]]").slice(1)) {
  const identifier = block.match(/^\s*identifier\s*=\s*"([^"]+)"/m)?.[1];
  const commandArray = block.match(/commands\.allow\s*=\s*\[([\s\S]*?)\]/)?.[1];
  if (!identifier || commandArray === undefined) continue;
  const commands = quotedValues(commandArray);
  permissionCommands.set(identifier, new Set(commands));
  if (activePermissions.has(identifier)) {
    for (const command of commands) allowedCommands.add(command);
  }
}

const expectedPermissions = {
  "allow-composer-media": [
    "composer_pick_attachments",
    "composer_stage_pasted_image",
    "composer_media_load",
    "composer_media_discard",
  ],
  "allow-context-compaction": [
    "history_context_compaction_source_load",
    "session_context_compaction_set",
    "session_context_compaction_clear",
  ],
  "allow-portable-storage-unlock": [
    "portable_storage_status",
    "portable_storage_unlock",
    "portable_storage_recover",
  ],
  "allow-storage-mode-switch": ["storage_mode_status", "storage_mode_set"],
  "allow-global-read": ["fs_read_global_text"],
  "allow-workspace-capability": ["workspace_capability_issue"],
  "allow-workspace-read": ["fs_read_text", "fs_grep", "fs_list"],
  "allow-workspace-mutate": ["fs_write_text", "fs_edit_text", "fs_delete"],
  "allow-workspace-shell": ["shell_run", "shell_cancel"],
  "allow-git-status": ["git_status"],
  "allow-git-commit-confirm": ["git_commit_capability_issue"],
  "allow-git-commit": ["git_commit"],
};

const exactPermissions = new Map(
  [
    "allow-workspace-capability",
    "allow-workspace-read",
    "allow-workspace-mutate",
    "allow-workspace-shell",
    "allow-git-status",
    "allow-git-commit-confirm",
    "allow-git-commit",
  ].map((permission) => [permission, expectedPermissions[permission]]),
);

const rendererCommands = new Set();
const invokePatterns = [
  /\binvoke(?:\s*<[^>]+>)?\s*\(\s*["']([^"']+)["']/g,
  /\bcore\.invoke(?:\s*<[^>]+>)?\s*\(\s*["']([^"']+)["']/g,
  /\bproviderInvoke\s*\(\s*["']([^"']+)["']/g,
  /\binvokeTool\s*\(\s*invoke\s*,\s*["']([^"']+)["']/g,
];

for (const path of sourceFiles(join(projectRoot, "src"))) {
  if (![".ts", ".html"].includes(extname(path))) continue;
  const source = fs.readFileSync(path, "utf8");
  for (const pattern of invokePatterns) {
    for (const match of source.matchAll(pattern))
      rendererCommands.add(match[1]);
  }
}

const handlerSource = read("src-tauri/src/lib.rs");
const registeredCommands = new Set(
  [
    ...handlerSource.matchAll(
      /^\s*(?:backend|local_services|mcp_registry|proxy)::([a-z_]+),/gm,
    ),
  ].map((match) => match[1]),
);

const failures = [];
const broadLegacyPermissions = ["allow-workspace-tools", "allow-git-review"];
for (const permission of broadLegacyPermissions) {
  if (activePermissions.has(permission))
    failures.push(`default capability still enables broad ${permission}`);
  if (permissionCommands.has(permission))
    failures.push(`legacy broad permission ${permission} is still defined`);
}

for (const [permission, commands] of Object.entries(expectedPermissions)) {
  if (!activePermissions.has(permission)) {
    failures.push(`default capability does not enable ${permission}`);
    continue;
  }
  const granted = permissionCommands.get(permission);
  for (const command of commands) {
    if (!granted?.has(command))
      failures.push(`${permission} does not grant ${command}`);
  }
}

for (const [permission, expected] of exactPermissions) {
  const granted = [...(permissionCommands.get(permission) ?? [])].sort();
  const exact = [...expected].sort();
  if (JSON.stringify(granted) !== JSON.stringify(exact)) {
    failures.push(
      `${permission} must grant exactly ${exact.join(", ")}; saw ${granted.join(", ")}`,
    );
  }
}

if (tauriConfig.app?.withGlobalTauri === true) {
  for (const [permission, commands] of permissionCommands) {
    const hasWorkspaceRead = [
      "workspace_capability_issue",
      "fs_read_text",
      "fs_grep",
      "fs_list",
    ].some((command) => commands.has(command));
    const hasWorkspaceMutation = [
      "fs_write_text",
      "fs_edit_text",
      "fs_delete",
    ].some((command) => commands.has(command));
    const hasWorkspaceShell =
      commands.has("shell_run") || commands.has("shell_cancel");
    if (hasWorkspaceRead && hasWorkspaceMutation) {
      failures.push(`${permission} mixes workspace read and mutation commands`);
    }
    if (hasWorkspaceRead && hasWorkspaceShell) {
      failures.push(`${permission} mixes workspace read and shell commands`);
    }
    if (hasWorkspaceMutation && hasWorkspaceShell) {
      failures.push(
        `${permission} mixes workspace mutation and shell commands`,
      );
    }
    if (
      commands.has("git_commit") &&
      (commands.has("git_status") ||
        commands.has("git_commit_capability_issue"))
    ) {
      failures.push(
        `${permission} mixes Git commit with status or confirmation`,
      );
    }
  }
}

for (const command of rendererCommands) {
  if (!allowedCommands.has(command))
    failures.push(`renderer invokes ${command} without a default capability`);
  if (!registeredCommands.has(command))
    failures.push(`renderer invokes ${command} without a native handler`);
}

for (const command of allowedCommands) {
  if (!registeredCommands.has(command))
    failures.push(`default capability grants unregistered command ${command}`);
}

const globalReadStart = backendSource.indexOf("pub fn fs_read_global_text(");
const globalReadEnd = backendSource.indexOf(
  "#[derive(Debug, Clone, Serialize)]",
  globalReadStart,
);
const globalReadHandler =
  globalReadStart >= 0 && globalReadEnd > globalReadStart
    ? backendSource.slice(globalReadStart, globalReadEnd)
    : "";
const globalReadCapabilityStart = backendSource.indexOf(
  "fn require_global_read_capability(",
);
const globalReadCapabilityEnd = backendSource.indexOf(
  "fn require_worktree_child_mutation_capability(",
  globalReadCapabilityStart,
);
const globalReadCapability =
  globalReadCapabilityStart >= 0 &&
  globalReadCapabilityEnd > globalReadCapabilityStart
    ? backendSource.slice(globalReadCapabilityStart, globalReadCapabilityEnd)
    : "";
if (
  !embeddedRuntime.includes('"GlobalRead"') ||
  !embeddedRuntime.includes("Global reads do not require confirmation") ||
  !embeddedRuntime.includes('"globalread",') ||
  !globalReadHandler.includes("require_global_read_capability") ||
  !globalReadCapability.includes("ToolAction::Read") ||
  !globalReadHandler.includes("canonical_global_read_target(&path)?") ||
  globalReadHandler.includes("require_global_read_approval") ||
  backendSource.includes("fn require_global_read_approval") ||
  backendSource.includes("global_read_target:")
) {
  failures.push(
    "GlobalRead must remain confirmation-free, capability-bound, and read-only",
  );
}

const gitCommitStart = backendSource.indexOf("pub async fn git_commit(");
const gitCommitEnd = backendSource.indexOf(
  "// ---------------------------------------------------------------------------",
  gitCommitStart,
);
const gitCommitHandler =
  gitCommitStart >= 0 && gitCommitEnd > gitCommitStart
    ? backendSource.slice(gitCommitStart, gitCommitEnd)
    : "";
const gitCommitGrantStart = backendSource.indexOf(
  "fn issue_git_commit_capability(",
);
const gitCommitGrantEnd = backendSource.indexOf(
  "fn consume_git_commit_capability(",
  gitCommitGrantStart,
);
const gitCommitGrantHandler =
  gitCommitGrantStart >= 0 && gitCommitGrantEnd > gitCommitGrantStart
    ? backendSource.slice(gitCommitGrantStart, gitCommitGrantEnd)
    : "";
const gitSnapshotStart = backendSource.indexOf(
  "fn git_staged_snapshot_digest(",
);
const gitSnapshotEnd = backendSource.indexOf(
  "fn prune_git_commit_capabilities(",
  gitSnapshotStart,
);
const gitSnapshotHandler =
  gitSnapshotStart >= 0 && gitSnapshotEnd > gitSnapshotStart
    ? backendSource.slice(gitSnapshotStart, gitSnapshotEnd)
    : "";
if (
  !gitCommitHandler.includes("consume_git_commit_capability") ||
  gitCommitHandler.includes("require_read_capability") ||
  !gitCommitGrantHandler.includes("require_workspace_view_capability") ||
  !gitCommitGrantHandler.includes("request_native_git_commit_confirmation") ||
  !backendSource.includes(
    "Git staged changes changed after commit confirmation",
  )
) {
  failures.push(
    "Git commits must consume a one-use native grant, never a read-only workspace capability",
  );
}
if (
  !gitSnapshotHandler.includes('"diff"') ||
  !gitSnapshotHandler.includes('"--cached"') ||
  !gitSnapshotHandler.includes('"--raw"') ||
  !gitSnapshotHandler.includes('"--full-index"') ||
  !gitSnapshotHandler.includes('"--no-ext-diff"') ||
  !gitSnapshotHandler.includes('"--no-renames"') ||
  gitSnapshotHandler.includes(".entries")
) {
  failures.push("Git commit grants must bind the raw staged snapshot");
}

if (failures.length) {
  console.error("IPC capability audit failed:");
  for (const failure of failures) console.error(`- ${failure}`);
  process.exitCode = 1;
} else {
  console.log("IPC capability audit passed", {
    rendererCommands: rendererCommands.size,
    defaultCommands: allowedCommands.size,
    protectedFlows: [
      "composer-media",
      "context-compaction",
      "segmented-workspace-tools",
      "one-use-git-commit",
    ],
  });
}
