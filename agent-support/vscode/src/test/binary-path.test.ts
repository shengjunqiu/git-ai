import * as assert from "assert";
import * as path from "path";
import { getInstalledGitAiBinaryCandidates } from "../utils/binary-path";

//jj

suite("git-ai binary path", () => {
  test("uses the per-user installation on macOS and Linux", () => {
    assert.deepStrictEqual(
      getInstalledGitAiBinaryCandidates("/Users/example", "darwin"),
      [path.join("/Users/example", ".git-ai", "bin", "git-ai")],
    );
    assert.deepStrictEqual(
      getInstalledGitAiBinaryCandidates("/home/example", "linux"),
      [path.join("/home/example", ".git-ai", "bin", "git-ai")],
    );
  });

  test("uses the executable suffix on Windows", () => {
    assert.deepStrictEqual(
      getInstalledGitAiBinaryCandidates("C:\\Users\\example", "win32"),
      [path.join("C:\\Users\\example", ".git-ai", "bin", "git-ai.exe")],
    );
  });
});
