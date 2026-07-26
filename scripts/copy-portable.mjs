import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const root = path.resolve(path.dirname(scriptPath), "..");
const expectedFileNames = [
  "NovaVei-portable.exe",
  "NovaVei-portable.manifest.json",
  "novavei-portable.json",
];
export { expectedFileNames };

function safePortableVersion(value) {
  const version = String(value ?? "").trim();
  if (!/^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$/.test(version)) {
    throw new Error(
      "package version is not safe for a portable directory name",
    );
  }
  return version;
}

function removeOwnedStagingDirectory(stagingDir, packageRoot) {
  const resolvedStaging = path.resolve(stagingDir);
  const resolvedPackageRoot = path.resolve(packageRoot);
  if (
    path.dirname(resolvedStaging) === resolvedPackageRoot &&
    path.basename(resolvedStaging).startsWith(".novavei-portable-")
  ) {
    fs.rmSync(resolvedStaging, { recursive: true, force: true });
  }
}

export function stagePortablePackage({
  sourceBinary,
  packageRoot,
  version: rawVersion,
  createdAt = new Date().toISOString(),
}) {
  const version = safePortableVersion(rawVersion);
  const source = path.resolve(sourceBinary);
  const resolvedPackageRoot = path.resolve(packageRoot);
  const outDir = path.join(resolvedPackageRoot, `NovaVei-${version}-portable`);

  if (!fs.existsSync(source) || !fs.statSync(source).isFile()) {
    throw new Error(`missing release binary: ${source}`);
  }
  fs.mkdirSync(resolvedPackageRoot, { recursive: true });
  if (fs.existsSync(outDir)) {
    throw new Error(
      `portable package already exists; move or archive it first: ${outDir}`,
    );
  }

  const stagingDir = fs.mkdtempSync(
    path.join(resolvedPackageRoot, ".novavei-portable-"),
  );
  try {
    const dest = path.join(stagingDir, expectedFileNames[0]);
    const manifestPath = path.join(stagingDir, expectedFileNames[1]);
    const portableMarkerPath = path.join(stagingDir, expectedFileNames[2]);
    fs.copyFileSync(source, dest, fs.constants.COPYFILE_EXCL);
    const st = fs.statSync(dest);
    const sha256 = createHash("sha256")
      .update(fs.readFileSync(dest))
      .digest("hex");
    const manifest = {
      product: "NovaVei",
      version,
      file: path.basename(dest),
      sizeBytes: st.size,
      sha256,
      runtime: "embedded-webview-pi",
      storageMode: "portable",
      sidecars: [],
      createdAt,
    };
    fs.writeFileSync(
      manifestPath,
      `${JSON.stringify(manifest, null, 2)}\n`,
      "utf8",
    );
    fs.writeFileSync(
      portableMarkerPath,
      `${JSON.stringify({ schemaVersion: 1, mode: "portable" }, null, 2)}\n`,
      "utf8",
    );

    const stagedEntries = fs.readdirSync(stagingDir, { withFileTypes: true });
    const stagedNames = stagedEntries.map((entry) => entry.name).sort();
    const expectedNames = [...expectedFileNames].sort();
    if (
      stagedEntries.some((entry) => !entry.isFile()) ||
      JSON.stringify(stagedNames) !== JSON.stringify(expectedNames)
    ) {
      throw new Error("portable staging contains unexpected files");
    }

    fs.renameSync(stagingDir, outDir);
    return {
      outDir,
      executablePath: path.join(outDir, expectedFileNames[0]),
      manifestPath: path.join(outDir, expectedFileNames[1]),
      markerPath: path.join(outDir, expectedFileNames[2]),
      sizeBytes: st.size,
      sha256,
    };
  } catch (error) {
    removeOwnedStagingDirectory(stagingDir, resolvedPackageRoot);
    throw error;
  }
}

function main() {
  // Cargo may use the shared E: target required by the workspace. Resolve the
  // artifact from the same target directory Cargo sees instead of assuming
  // the checkout-local default.
  const cargoTarget = process.env.CARGO_TARGET_DIR?.trim()
    ? path.resolve(process.env.CARGO_TARGET_DIR)
    : path.join(root, "src-tauri", "target");
  const sourceBinary = path.join(cargoTarget, "release", "novavei-agent.exe");
  const packageJson = JSON.parse(
    fs.readFileSync(path.join(root, "package.json"), "utf8"),
  );

  // Never stage beside the runtime `release/novavei/` directory. A portable
  // executable creates user data there when launched in place, so reusing the
  // release root could turn private data into a distribution.
  const result = stagePortablePackage({
    sourceBinary,
    packageRoot: path.join(root, "release", "packages"),
    version: packageJson.version,
  });
  console.log("portable_package:", result.outDir);
  console.log("portable:", result.executablePath);
  console.log("size_bytes:", result.sizeBytes);
  console.log("size_mb:", (result.sizeBytes / 1024 / 1024).toFixed(2));
  console.log("sha256:", result.sha256);
  console.log("manifest:", result.manifestPath);
  console.log("portable_marker:", result.markerPath);
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === path.resolve(scriptPath)
) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
