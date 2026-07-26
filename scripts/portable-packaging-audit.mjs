import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { expectedFileNames, stagePortablePackage } from "./copy-portable.mjs";

const temporaryRoot = fs.mkdtempSync(
  path.join(os.tmpdir(), "novavei-portable-audit-"),
);

try {
  const sourceBinary = path.join(temporaryRoot, "novavei-agent.exe");
  const releaseRoot = path.join(temporaryRoot, "release");
  const packageRoot = path.join(releaseRoot, "packages");
  const userDataRoot = path.join(releaseRoot, "novavei");
  const privateDataPath = path.join(userDataRoot, "private.sqlite3");
  const binary = Buffer.from("deterministic portable audit binary", "utf8");
  fs.mkdirSync(userDataRoot, { recursive: true });
  fs.writeFileSync(sourceBinary, binary);
  fs.writeFileSync(privateDataPath, "must remain private", "utf8");

  const result = stagePortablePackage({
    sourceBinary,
    packageRoot,
    version: "1.2.3",
    createdAt: "2026-07-26T00:00:00.000Z",
  });
  assert.deepEqual(
    fs.readdirSync(result.outDir).sort(),
    [...expectedFileNames].sort(),
  );
  assert.equal(fs.readFileSync(privateDataPath, "utf8"), "must remain private");
  assert.equal(fs.readFileSync(result.executablePath).compare(binary), 0);

  const manifest = JSON.parse(fs.readFileSync(result.manifestPath, "utf8"));
  assert.equal(manifest.version, "1.2.3");
  assert.equal(manifest.sizeBytes, binary.length);
  assert.equal(
    manifest.sha256,
    createHash("sha256").update(binary).digest("hex"),
  );
  assert.deepEqual(JSON.parse(fs.readFileSync(result.markerPath, "utf8")), {
    schemaVersion: 1,
    mode: "portable",
  });
  assert.throws(
    () =>
      stagePortablePackage({
        sourceBinary,
        packageRoot,
        version: "1.2.3",
      }),
    /portable package already exists/,
  );
  assert.equal(
    fs
      .readdirSync(packageRoot)
      .some((name) => name.startsWith(".novavei-portable-")),
    false,
  );

  console.log("Portable packaging audit passed", {
    files: 3,
    existingPackageRejected: true,
    userDataUntouched: true,
  });
} finally {
  const resolvedTemporaryRoot = path.resolve(temporaryRoot);
  const resolvedSystemTemporary = path.resolve(os.tmpdir());
  if (
    path.dirname(resolvedTemporaryRoot) === resolvedSystemTemporary &&
    path.basename(resolvedTemporaryRoot).startsWith("novavei-portable-audit-")
  ) {
    fs.rmSync(resolvedTemporaryRoot, { recursive: true, force: true });
  }
}
