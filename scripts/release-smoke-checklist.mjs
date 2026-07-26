/**
 * Print the remaining release-acceptance checklist.
 *
 * This file only records live gates that require a human, a real provider, or
 * a packaged WebView — they cannot be faked by local source checks.
 */
const gates = [
  {
    id: "P0-provider",
    title: "Real provider stream",
    steps: [
      "Start desktop: npm run dev (or packaged portable EXE)",
      "Add a real provider with key; fetch models; save",
      "Open a project + session; send a short prompt",
      "Observe text_delta stream and a terminal done/error/cancelled event",
      "Restart app; confirm session history rehydrates from SQLite",
    ],
  },
  {
    id: "P0-mcp",
    title: "Real MCP server smoke",
    steps: [
      "Configure one stdio or HTTP MCP server in Hub (secrets stay native)",
      "Test server from Hub; list tools",
      "Ask main chat to call one tool; approve once",
      "Confirm tool result appears and secrets stay redacted",
    ],
  },
  {
    id: "P0-package",
    title: "Packaged WebView + IPC",
    steps: [
      "npm run pack:portable (or build:installer)",
      "Launch release/packages/NovaVei-<version>-portable/NovaVei-portable.exe",
      "For a portable package, keep novavei-portable.json beside the EXE; create a portable password, restart, and unlock it",
      "Move the complete portable folder to another drive letter and confirm its novavei/ data reopens without writing installed-mode AppData",
      "Repeat a short chat turn and one settings save",
      "Record SHA-256 of the new artifact; do not reuse historical hashes",
    ],
  },
  {
    id: "P1-cron",
    title: "Cron native execution (HTTP / Shell / Prompt)",
    steps: [
      "Create enabled HTTP job; wait or Run now; status completed/failed only",
      "Create Shell job with safe workdir command; confirm auto/manual run",
      "Create Prompt job with saved provider; confirm completion without UI payload echo",
    ],
  },
];

console.log("NovaVei release smoke checklist");
console.log("================================");
console.log("Source/deterministic tests do not mark these complete.\n");
for (const gate of gates) {
  console.log(`[ ] ${gate.id} · ${gate.title}`);
  for (const step of gate.steps) {
    console.log(`    - ${step}`);
  }
  console.log("");
}
console.log("Run: node scripts/release-smoke-checklist.mjs");
