import { execFile } from "child_process";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as vscode from "vscode";

let resolvedPath: string | null = null;
let resolvePromise: Promise<string | null> | null = null;
let extensionMode: vscode.ExtensionMode | null = null;

/**
 * Call once at activation to pass in the extension context's mode.
 */
export function initBinaryResolver(mode: vscode.ExtensionMode): void {
  extensionMode = mode;
  resolvedPath = findInstalledGitAiBinary();
}

export function getInstalledGitAiBinaryCandidates(
  homeDir: string,
  platform: NodeJS.Platform,
): string[] {
  if (platform === "win32") {
    return [path.join(homeDir, ".git-ai", "bin", "git-ai.exe")];
  }
  return [path.join(homeDir, ".git-ai", "bin", "git-ai")];
}

function findInstalledGitAiBinary(): string | null {
  return getInstalledGitAiBinaryCandidates(os.homedir(), os.platform())
    .find((candidate) => {
      try {
        fs.accessSync(candidate, fs.constants.X_OK);
        return fs.statSync(candidate).isFile();
      } catch {
        return false;
      }
    }) ?? null;
}

/**
 * Resolve the full path to the `git-ai` binary. Production first uses the
 * standard per-user installation; development may additionally query a login
 * shell so locally built binaries remain discoverable.
 *
 * The result is cached after the first successful resolution.
 */
export function resolveGitAiBinary(): Promise<string | null> {
  if (resolvedPath) {
    return Promise.resolve(resolvedPath);
  }

  const installedBinary = findInstalledGitAiBinary();
  if (installedBinary) {
    resolvedPath = installedBinary;
    return Promise.resolve(resolvedPath);
  }

  // Production installations normally use ~/.git-ai/bin. If it is absent,
  // retain the PATH fallback without spawning a login shell from the editor.
  if (extensionMode !== vscode.ExtensionMode.Development) {
    return Promise.resolve(null);
  }
  if (resolvePromise) {
    return resolvePromise;
  }

  resolvePromise = new Promise((resolve) => {
    const platform = os.platform();

    if (platform === "win32") {
      // Windows: use `where git-ai`
      execFile("where", ["git-ai"], (err, stdout) => {
        if (err || !stdout.trim()) {
          console.log("[git-ai] Could not resolve git-ai binary via 'where'");
          resolve(null);
        } else {
          // `where` can return multiple lines; take the first
          resolvedPath = stdout.trim().split(/\r?\n/)[0];
          console.log("[git-ai] Resolved binary path:", resolvedPath);
          resolve(resolvedPath);
        }
      });
    } else {
      // macOS/Linux: spawn a login shell so the user's profile is sourced
      const shell = process.env.SHELL || "/bin/bash";
      execFile(shell, ["-ilc", "which git-ai"], { timeout: 5000 }, (err, stdout) => {
        if (err || !stdout.trim()) {
          console.log("[git-ai] Could not resolve git-ai binary via login shell");
          resolve(null);
        } else {
          resolvedPath = stdout.trim();
          console.log("[git-ai] Resolved binary path:", resolvedPath);
          resolve(resolvedPath);
        }
      });
    }
  });

  return resolvePromise;
}

/**
 * Get the resolved git-ai binary path, or fall back to just "git-ai"
 * (which relies on the current process PATH).
 */
export function getGitAiBinary(): string {
  return resolvedPath || "git-ai";
}
