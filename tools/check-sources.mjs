// SPDX-License-Identifier: GPL-2.0-only

// Two gates nothing else covers.
//
// 1. The shell's frontend has no build step, so a syntax error in it first appears as a blank
//    window rather than as a failed build.
// 2. CLAUDE.md requires `SPDX-License-Identifier: GPL-2.0-only` in every source file; the
//    repository licence is GPL-2.0-only and a missing header is a licensing defect, not a style
//    one.
//
// Only tracked files are checked, so `target/`, `node_modules/`, and generated Tauri output are
// excluded without maintaining a second ignore list.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, resolve } from "node:path";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const SPDX = "SPDX-License-Identifier: GPL-2.0-only";
// Extensions that carry a comment syntax and belong to us. JSON has no comments, and the
// frozen fixtures under tests/fixtures are captured wire data that must stay byte-faithful.
const NEEDS_HEADER = [".rs", ".js", ".mjs", ".css", ".html", ".yml", ".yaml"];

function tracked() {
  return execFileSync("git", ["-C", repository, "ls-files", "-z"], { encoding: "utf8" })
    .split("\0")
    .filter(Boolean);
}

const files = tracked();
const failures = [];

// --- 1. JavaScript parses -------------------------------------------------

const scripts = files.filter((file) => file.endsWith(".js") || file.endsWith(".mjs"));
for (const file of scripts) {
  try {
    execFileSync(process.execPath, ["--check", join(repository, file)], { stdio: "pipe" });
  } catch (error) {
    const detail = String(error.stderr ?? error.message).trim();
    failures.push(`${file}: does not parse\n${detail}`);
  }
}

// --- 2. SPDX headers ------------------------------------------------------

const headered = files.filter((file) => NEEDS_HEADER.some((suffix) => file.endsWith(suffix)));
for (const file of headered) {
  // The identifier belongs at the top; reading the first 2 KiB avoids loading large sources
  // while still allowing a shebang or a leading block comment above it.
  const head = readFileSync(join(repository, file), "utf8").slice(0, 2048);
  if (!head.includes(SPDX)) {
    failures.push(`${file}: missing ${SPDX}`);
  }
}

// --- Report ---------------------------------------------------------------

if (failures.length > 0) {
  for (const failure of failures) {
    console.error(`  ${failure}`);
  }
  console.error(`\n${failures.length} problem(s) in ${files.length} tracked files.`);
  process.exit(1);
}

console.log(`${scripts.length} scripts parse, ${headered.length} sources carry the SPDX header.`);
