// SPDX-License-Identifier: GPL-2.0-only

import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";

const privateKeyArgument = process.argv[2];

if (!privateKeyArgument) {
  console.error("usage: just bundle-updater <path-to-private-key>");
  process.exitCode = 2;
} else {
  await buildUpdater(privateKeyArgument);
}

async function buildUpdater(privateKeyArgument) {
  const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const appDirectory = join(repositoryRoot, "crates", "reveille-app");
  const privateKeyPath = resolve(privateKeyArgument);
  const publicKeyPath = `${privateKeyPath}.pub`;
  const temporaryDirectory = await mkdtemp(join(tmpdir(), "reveille-updater-"));
  const configPath = join(temporaryDirectory, "tauri-updater.json");

  try {
    const publicKey = (await readFile(publicKeyPath, "utf8")).trim();

    if (!publicKey) {
      throw new Error(`updater public key is empty: ${publicKeyPath}`);
    }

    await writeFile(
      configPath,
      JSON.stringify({
        bundle: { createUpdaterArtifacts: true },
        plugins: { updater: { pubkey: publicKey } },
      }),
      "utf8",
    );

    const tauriCli = join(appDirectory, "node_modules", "@tauri-apps", "cli", "tauri.js");
    const result = spawn(
      process.execPath,
      [tauriCli, "build", "--config", configPath, "--", "--locked"],
      {
        cwd: appDirectory,
        env: {
          ...process.env,
          REVEILLE_UPDATER_PUBKEY: publicKey,
          TAURI_SIGNING_PRIVATE_KEY: privateKeyPath,
        },
        stdio: "inherit",
      },
    );

    const exitCode = await new Promise((resolveExit, reject) => {
      result.once("error", reject);
      result.once("exit", (code, signal) => {
        if (signal) {
          reject(new Error(`Tauri build terminated by ${signal}`));
        } else {
          resolveExit(code ?? 1);
        }
      });
    });

    if (exitCode !== 0) {
      process.exitCode = exitCode;
    }
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  } finally {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
}
