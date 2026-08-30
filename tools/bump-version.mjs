// SPDX-License-Identifier: GPL-2.0-only

import { readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const nextVersion = process.argv[2];
const dryRun = process.argv.slice(3).includes("--dry-run");

if (!nextVersion || process.argv.slice(3).some((argument) => argument !== "--dry-run")) {
  console.error("usage: just bump-version <new-version> [--dry-run]");
  process.exitCode = 2;
} else {
  await bumpVersion(nextVersion, dryRun);
}

async function bumpVersion(next, checkOnly) {
  try {
    const parsedNext = parseSemver(next);
    const files = {
      cargoToml: "Cargo.toml",
      cargoLock: "Cargo.lock",
      packageJson: "crates/reveille-app/package.json",
      packageLock: "crates/reveille-app/package-lock.json",
      tauriConfig: "crates/reveille-app/tauri.conf.json",
    };
    const contents = Object.fromEntries(
      await Promise.all(
        Object.entries(files).map(async ([key, path]) => [
          key,
          await readFile(join(repository, path), "utf8"),
        ]),
      ),
    );

    const packageJson = JSON.parse(contents.packageJson);
    const packageLock = JSON.parse(contents.packageLock);
    const tauriConfig = JSON.parse(contents.tauriConfig);
    const versions = new Map([
      ["Cargo workspace", capture(contents.cargoToml, /\[workspace\.package\][\s\S]*?\nversion = "([^"]+)"/, "Cargo.toml")],
      ["npm package", packageJson.version],
      ["npm lockfile", packageLock.version],
      ["npm lockfile root package", packageLock.packages?.[""]?.version],
      ["Tauri", tauriConfig.version],
    ]);
    for (const name of ["reveille-app", "reveille-cli", "reveille-core", "reveille-platform"]) {
      const pattern = new RegExp(`\\[\\[package\\]\\]\\r?\\nname = "${name}"\\r?\\nversion = "([^"]+)"`);
      versions.set(`Cargo.lock ${name}`, capture(contents.cargoLock, pattern, "Cargo.lock"));
    }

    const current = versions.values().next().value;
    const mismatches = [...versions].filter(([, version]) => version !== current);
    if (mismatches.length > 0) {
      const detail = [...versions].map(([label, version]) => `${label}=${version}`).join(", ");
      throw new Error(`release versions already disagree: ${detail}`);
    }
    const parsedCurrent = parseSemver(current);
    if (compareSemver(parsedNext, parsedCurrent) <= 0) {
      throw new Error(`${next} is not newer than the current release version ${current}`);
    }

    const updated = {
      cargoToml: replaceOne(
        contents.cargoToml,
        /(\[workspace\.package\]\r?\nversion = ")[^"]+("\r?\n)/,
        next,
        "Cargo.toml workspace version",
      ),
      cargoLock: contents.cargoLock,
      packageJson: replaceOne(
        contents.packageJson,
        /(^\{\r?\n  "name": "reveille-app",\r?\n  "private": true,\r?\n  "version": ")[^"]+("[,\r\n])/,
        next,
        "package.json version",
      ),
      packageLock: replaceOne(
        replaceOne(
          contents.packageLock,
          /(^\{\r?\n  "name": "reveille-app",\r?\n  "version": ")[^"]+("[,\r\n])/,
          next,
          "package-lock.json version",
        ),
        /("packages": \{\r?\n    "": \{\r?\n      "name": "reveille-app",\r?\n      "version": ")[^"]+("[,\r\n])/,
        next,
        "package-lock.json root package version",
      ),
      tauriConfig: replaceOne(
        contents.tauriConfig,
        /("mainBinaryName": "Reveille",\r?\n  "version": ")[^"]+("[,\r\n])/,
        next,
        "tauri.conf.json version",
      ),
    };
    for (const name of ["reveille-app", "reveille-cli", "reveille-core", "reveille-platform"]) {
      const pattern = new RegExp(`(\\[\\[package\\]\\]\\r?\\nname = "${name}"\\r?\\nversion = ")[^"]+("\\r?\\n)`);
      updated.cargoLock = replaceOne(updated.cargoLock, pattern, next, `Cargo.lock ${name}`);
    }

    if (!checkOnly) {
      await Promise.all(
        Object.entries(files).map(([key, path]) =>
          writeFile(join(repository, path), updated[key], "utf8"),
        ),
      );
    }
    console.log(`${current} -> ${next}${checkOnly ? " (dry run)" : ""}`);
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}

function capture(contents, pattern, path) {
  const match = contents.match(pattern);
  if (!match) throw new Error(`could not find the release version in ${path}`);
  return match[1];
}

function replaceOne(contents, pattern, next, label) {
  const matches = contents.match(pattern);
  if (!matches) throw new Error(`could not update ${label}`);
  return contents.replace(pattern, (...parts) => `${parts[1]}${next}${parts[2]}`);
}

function parseSemver(version) {
  const match = version.match(
    /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/,
  );
  if (!match) throw new Error(`${version} is not a semantic version`);
  const prerelease = match[4]?.split(".") ?? null;
  if (prerelease?.some((part) => /^0\d+$/.test(part))) {
    throw new Error(`${version} is not a semantic version`);
  }
  return {
    core: match.slice(1, 4).map(BigInt),
    prerelease,
  };
}

function compareSemver(left, right) {
  for (let index = 0; index < left.core.length; index += 1) {
    if (left.core[index] !== right.core[index]) {
      return left.core[index] > right.core[index] ? 1 : -1;
    }
  }
  if (left.prerelease === null || right.prerelease === null) {
    if (left.prerelease === right.prerelease) return 0;
    return left.prerelease === null ? 1 : -1;
  }
  const length = Math.max(left.prerelease.length, right.prerelease.length);
  for (let index = 0; index < length; index += 1) {
    const leftPart = left.prerelease[index];
    const rightPart = right.prerelease[index];
    if (leftPart === undefined || rightPart === undefined) {
      return leftPart === undefined ? -1 : 1;
    }
    if (leftPart === rightPart) continue;
    const leftNumeric = /^\d+$/.test(leftPart);
    const rightNumeric = /^\d+$/.test(rightPart);
    if (leftNumeric && rightNumeric) return BigInt(leftPart) > BigInt(rightPart) ? 1 : -1;
    if (leftNumeric !== rightNumeric) return leftNumeric ? -1 : 1;
    return leftPart > rightPart ? 1 : -1;
  }
  return 0;
}
