import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const scriptDirectory = path.dirname(scriptPath);
const applicationDirectory = path.resolve(scriptDirectory, "..");
const workflowsDirectory = path.resolve(
  applicationDirectory,
  ".github",
  "workflows",
);
const dependencySecurityWorkflowPath = path.join(
  workflowsDirectory,
  "dependency-security.yml",
);
const verifyWorkflowPath = path.join(workflowsDirectory, "verify.yml");
const legacyDependencyAuditWorkflowPath = path.join(
  workflowsDirectory,
  "dependency-audit.yml",
);
const dependabotPath = path.resolve(
  applicationDirectory,
  ".github",
  "dependabot.yml",
);
const packageJsonPath = path.resolve(applicationDirectory, "package.json");
const packageLockPath = path.resolve(applicationDirectory, "package-lock.json");
const readmePath = path.resolve(applicationDirectory, "README.md");
const cargoLockPath = path.resolve(
  applicationDirectory,
  "src-tauri",
  "Cargo.lock",
);
const rustsecAuditCheckSha = "69366f33c96575abad1ee0dba8212993eecbe998"; // v2.0.0

const failures = [];

function check(description, condition) {
  if (!condition) failures.push(description);
}

function readText(absolutePath) {
  return fs.existsSync(absolutePath)
    ? fs.readFileSync(absolutePath, "utf8")
    : "";
}

function readJson(absolutePath, description) {
  const source = readText(absolutePath);
  if (!source) {
    failures.push(`${description} is present and valid JSON`);
    return {};
  }

  try {
    return JSON.parse(source);
  } catch {
    failures.push(`${description} is present and valid JSON`);
    return {};
  }
}

function collectWorkflowFiles(directory) {
  if (!fs.existsSync(directory)) return [];

  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) return collectWorkflowFiles(entryPath);
    return /\.ya?ml$/i.test(entry.name) ? [entryPath] : [];
  });
}

function collectUses(workflowPath, source) {
  const references = [];
  const usesPattern =
    /^\s*(?:-\s+)?uses:\s*(?:"([^"]+)"|'([^']+)'|([^\s#]+))(?:\s+#.*)?$/gm;

  for (const match of source.matchAll(usesPattern)) {
    references.push({
      workflow: path
        .relative(applicationDirectory, workflowPath)
        .split(path.sep)
        .join("/"),
      reference: match[1] ?? match[2] ?? match[3],
    });
  }

  return references;
}

function hasWeeklySchedule(source) {
  const crons = [
    ...source.matchAll(/^\s*-\s*cron:\s*["']?([^"'\r\n#]+)["']?/gm),
  ].map((match) => match[1].trim());

  return crons.some((cron) => {
    const fields = cron.split(/\s+/);
    return (
      fields.length === 5 &&
      fields[2] === "*" &&
      fields[3] === "*" &&
      fields[4] !== "*"
    );
  });
}

function findDependabotUpdate(source, ecosystem, directory) {
  const updateBlocks = source.split(/\r?\n(?=\s*-\s+package-ecosystem:)/);
  const ecosystemPattern = new RegExp(
    `^\\s*-\\s+package-ecosystem:\\s*["']?${ecosystem}["']?\\s*$`,
    "m",
  );
  const directoryPattern = new RegExp(
    `^\\s+directory:\\s*["']?${directory.replaceAll("/", "\\/")}["']?\\s*$`,
    "m",
  );

  return updateBlocks.find(
    (block) => ecosystemPattern.test(block) && directoryPattern.test(block),
  );
}

const workflowPaths = collectWorkflowFiles(workflowsDirectory);
const workflowSources = workflowPaths.map((workflowPath) => ({
  path: workflowPath,
  source: readText(workflowPath),
}));
const workflowUses = workflowSources.flatMap(({ path: workflowPath, source }) =>
  collectUses(workflowPath, source),
);
const dependencySecurityWorkflow = readText(dependencySecurityWorkflowPath);
const verifyWorkflow = readText(verifyWorkflowPath);
const dependabot = readText(dependabotPath);
const packageJson = readJson(packageJsonPath, "package.json");
const packageLock = readJson(packageLockPath, "package-lock.json");
const packageScripts = packageJson.scripts ?? {};
const readme = readText(readmePath);

const directDependencies = {
  ...(packageJson.dependencies ?? {}),
  ...(packageJson.devDependencies ?? {}),
};
const lockedRootPackage = packageLock.packages?.[""] ?? {};
const lockedDirectDependencies = {
  ...(lockedRootPackage.dependencies ?? {}),
  ...(lockedRootPackage.devDependencies ?? {}),
};

for (const [description, condition] of [
  [
    "the audit script is in the repository scripts directory",
    path.relative(applicationDirectory, scriptPath) ===
      path.join("scripts", "ci-supply-chain-audit.mjs"),
  ],
  ["the application root has package.json", packageJsonPath],
  ["the application root has package-lock.json", packageLockPath],
  ["the application root has README.md", readmePath],
  ["the application root has .github/dependabot.yml", dependabotPath],
  ["the application root has .github/workflows", workflowsDirectory],
  ["the application root has src-tauri/Cargo.lock", cargoLockPath],
]) {
  check(
    description,
    typeof condition === "string" ? fs.existsSync(condition) : condition,
  );
}

check(
  "at least one GitHub Actions workflow is present",
  workflowPaths.length > 0,
);
for (const { workflow, reference } of workflowUses) {
  check(
    `${workflow} pins ${reference} to a 40-character commit SHA`,
    /@[0-9a-f]{40}$/i.test(reference),
  );
}

check(
  "dependency-security workflow is present",
  dependencySecurityWorkflow.length > 0,
);
check(
  "legacy dependency-audit workflow is retired",
  !fs.existsSync(legacyDependencyAuditWorkflowPath),
);
check(
  "dependency-security workflow has the documented name",
  /^name:\s*Dependency security\s*$/m.test(dependencySecurityWorkflow),
);
check(
  "dependency-security workflow has a weekly schedule",
  /^\s*schedule:\s*$/m.test(dependencySecurityWorkflow) &&
    hasWeeklySchedule(dependencySecurityWorkflow),
);
check(
  "dependency-security workflow runs on pull requests without pull_request_target",
  /^\s+pull_request:\s*(?:#.*)?$/m.test(dependencySecurityWorkflow) &&
    !/^\s+pull_request_target:\s*/m.test(dependencySecurityWorkflow),
);
check(
  "dependency-security workflow supports manual dispatch",
  /^\s+workflow_dispatch:\s*(?:#.*)?$/m.test(dependencySecurityWorkflow),
);
check(
  "dependency-security workflow has read-only repository permissions",
  /^permissions:\s*\r?\n\s+contents:\s*read\s*$/m.test(
    dependencySecurityWorkflow,
  ) && !/^\s*[A-Za-z-]+:\s*write\s*$/m.test(dependencySecurityWorkflow),
);
check(
  "dependency-security workflow pins checkout by commit SHA",
  workflowUses.some(
    ({ workflow, reference }) =>
      workflow === ".github/workflows/dependency-security.yml" &&
      /^actions\/checkout@[0-9a-f]{40}$/i.test(reference),
  ),
);
check(
  "dependency-security checkout does not persist the repository token",
  /^\s+persist-credentials:\s*false\s*$/m.test(dependencySecurityWorkflow),
);
check(
  "verify checkout does not persist the repository token before project scripts run",
  /^\s+persist-credentials:\s*false\s*$/m.test(verifyWorkflow) &&
    verifyWorkflow.includes("npm ci") &&
    verifyWorkflow.includes("npm run verify"),
);
check(
  "dependency-security workflow pins RustSec audit-check v2.0.0",
  workflowUses.some(
    ({ workflow, reference }) =>
      workflow === ".github/workflows/dependency-security.yml" &&
      reference === `rustsec/audit-check@${rustsecAuditCheckSha}`,
  ),
);
check(
  "dependency-security workflow scans src-tauri/Cargo.lock",
  /^\s+working-directory:\s*src-tauri\s*$/m.test(dependencySecurityWorkflow) &&
    fs.existsSync(cargoLockPath),
);
check(
  "dependency-security workflow supplies only the GitHub Actions token",
  /^\s+token:\s*\$\{\{\s*secrets\.GITHUB_TOKEN\s*\}\}\s*$/m.test(
    dependencySecurityWorkflow,
  ) && !/^\s+ignore:\s*/m.test(dependencySecurityWorkflow),
);
check(
  "dependency-security workflow audits production and build npm dependencies from the committed lockfile",
  dependencySecurityWorkflow.includes("npm-production-audit") &&
    dependencySecurityWorkflow.includes(
      "npm audit --package-lock-only --omit=dev --audit-level=high",
    ) &&
    dependencySecurityWorkflow.includes(
      "npm audit --package-lock-only --audit-level=high",
    ) &&
    dependencySecurityWorkflow.includes("npm install --global npm@11.16.0") &&
    /^\s+node-version:\s*24\.18\.0\s*(?:#.*)?$/m.test(
      dependencySecurityWorkflow,
    ),
);

for (const [ecosystem, directory] of [
  ["npm", "/"],
  ["cargo", "/src-tauri"],
  ["github-actions", "/"],
]) {
  const update = findDependabotUpdate(dependabot, ecosystem, directory);
  check(
    `Dependabot covers ${ecosystem} dependencies in ${directory}`,
    Boolean(update),
  );
  check(
    `Dependabot updates ${ecosystem} dependencies weekly as one group`,
    Boolean(update) &&
      /^\s+interval:\s*["']?weekly["']?\s*$/m.test(update) &&
      /^\s+groups:\s*$/m.test(update) &&
      /^\s+patterns:\s*\r?\n\s+-\s*["']\*["']\s*$/m.test(update),
  );
}
check(
  "Dependabot configuration does not enable automatic merging",
  !/\bauto-?merge\b/i.test(dependabot),
);

check(
  "test:ci-supply-chain runs the static supply-chain audit",
  packageScripts["test:ci-supply-chain"] ===
    "node scripts/ci-supply-chain-audit.mjs",
);
check(
  "all direct Node dependencies use exact versions rather than mutable tags or ranges",
  Object.values(directDependencies).every((version) =>
    /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version),
  ),
);
check(
  "package.json direct dependency versions exactly match the committed lockfile root",
  Object.entries(directDependencies).every(
    ([name, version]) => lockedDirectDependencies[name] === version,
  ),
);
check(
  "Node type definitions stay on the supported Node 24 major",
  directDependencies["@types/node"]?.startsWith("24.") &&
    packageLock.packages?.["node_modules/@types/node"]?.version ===
      directDependencies["@types/node"],
);
check(
  "verify includes the static supply-chain audit",
  packageScripts.verify?.includes("npm run test:ci-supply-chain"),
);
check(
  "README documents the pull-request, scheduled, and manual dependency-security workflow",
  readme.includes(".github/workflows/dependency-security.yml") &&
    readme.includes("每个 PR") &&
    readme.includes("不持久化仓库凭据") &&
    readme.includes("联网"),
);
check(
  "README prohibits advisory ignores without an expiry explanation",
  readme.includes("advisory ignore") && readme.includes("过期"),
);

if (failures.length) {
  console.error("CI supply-chain audit failed:");
  for (const failure of failures) console.error(`- ${failure}`);
  process.exit(1);
}

console.log("CI supply-chain audit passed", {
  workflows: workflowPaths.length,
  pinnedActionUses: workflowUses.length,
  dependabotEcosystems: 3,
  rustsecAuditCheck: "v2.0.0",
});
