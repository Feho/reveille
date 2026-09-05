// SPDX-License-Identifier: GPL-2.0-only

// One release in one command: bump the release identities, run the CI gate, commit, tag.
//
// Pushing stays a separate, deliberate act. A pushed `v*` tag starts the installer build in
// .github/workflows/release.yml, which attaches the result to a *draft* release, and publishing
// that draft is what the website's `releases/latest` link starts pointing at.

import { execFileSync, spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// The files `bump-version` writes; staged by name so an unrelated stray change cannot ride along.
const VERSIONED = [
  "Cargo.toml",
  "Cargo.lock",
  "crates/reveille-app/package.json",
  "crates/reveille-app/package-lock.json",
  "crates/reveille-app/tauri.conf.json",
];

const nextVersion = process.argv[2];
const flags = process.argv.slice(3);
const dryRun = flags.includes("--dry-run");
const skipCheck = flags.includes("--no-check");

if (!nextVersion || flags.some((flag) => flag !== "--dry-run" && flag !== "--no-check")) {
  console.error("usage: just release <new-version> [--dry-run] [--no-check]");
  process.exitCode = 2;
} else {
  await release(nextVersion, dryRun, skipCheck);
}

async function release(next, checkOnly, noCheck) {
  const tag = `v${next}`;
  try {
    // Preconditions first: everything below either writes files or writes history.
    const status = git("status", "--porcelain").trim();
    if (status) {
      throw new Error(
        `the working tree has uncommitted changes; commit or stash them first:\n${status}`,
      );
    }
    if (git("tag", "--list", tag).trim()) {
      throw new Error(`${tag} already exists`);
    }
    const branch = git("rev-parse", "--abbrev-ref", "HEAD").trim();

    // `bump-version` owns the version rules: valid SemVer, newer than the current one, and every
    // release identity already in agreement.
    step("bump", process.execPath, [
      join(repository, "tools", "bump-version.mjs"),
      next,
      ...(checkOnly ? ["--dry-run"] : []),
    ]);

    if (checkOnly) {
      console.log(`dry run: would gate, commit and tag ${tag} on ${branch}`);
      return;
    }

    if (noCheck) {
      console.log("skipping `just check` (--no-check)");
    } else {
      // The gate runs against the bumped tree, so a lockfile the bump left inconsistent fails
      // here rather than in CI after the tag is pushed.
      step("check", "just", ["check"], {
        // `just` may be a shim rather than an `.exe` on a Windows PATH, so this one goes through
        // the shell. Its arguments contain no spaces, which is what makes that safe here.
        shell: process.platform === "win32",
        hint: "the gate failed. The version files are updated but nothing was committed; fix the failure and rerun, or `git checkout --` the version files to start over",
      });
    }

    step("commit", "git", ["-C", repository, "commit", "-m", `chore: bump version to ${tag}`, "--", ...VERSIONED]);
    step("tag", "git", ["-C", repository, "tag", "-a", tag, "-m", `Reveille ${tag}`]);

    console.log(`\n${tag} committed and tagged on ${branch}. To start the installer build:`);
    console.log(`  git push origin ${branch} && git push origin ${tag}`);
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}

function git(...args) {
  return execFileSync("git", ["-C", repository, ...args], { encoding: "utf8" });
}

// `shell` is off by default: a commit message contains spaces, and Windows shell invocation does
// not quote arguments for you.
function step(label, command, args, { shell = false, hint } = {}) {
  console.log(`\n--- ${label} ---`);
  const result = spawnSync(command, args, { cwd: repository, stdio: "inherit", shell });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(hint ?? `${label} failed (exit ${result.status})`);
  }
}
