import assert from "node:assert/strict";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, unlinkSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { parse } from "yaml";

const root = new URL("../", import.meta.url).pathname;
const sha = "3".repeat(40);
const policy = new URL("./fixtures/triggerd.yml", import.meta.url).pathname;

function run(file, args, options = {}) {
  return spawnSync(process.execPath, [file, ...args], { encoding: "utf8", ...options });
}

test("action metadata resolves the checked-in Node 24 bundle", () => {
  const metadata = parse(readFileSync(join(root, "action.yml"), "utf8"));
  assert.equal(metadata.runs.using, "node24");
  assert.equal(metadata.runs.main, "dist/main.js");
  assert.equal(existsSync(join(root, metadata.runs.main)), true);
  assert.equal(existsSync(join(root, "dist", "worker-main.js")), true);
  assert.equal(existsSync(join(root, "dist", "gate.js")), true);
});

test("installed dist runs the offline CLI and action from only dist plus runner files", () => {
  const temporary = mkdtempSync(join(tmpdir(), "triggerd-installed-"));
  try {
    const action = join(temporary, "action");
    const checkout = join(temporary, "checkout");
    cpSync(join(root, "dist"), join(action, "dist"), { recursive: true });
    mkdirSync(join(checkout, ".github"), { recursive: true });
    cpSync(policy, join(checkout, ".github", "triggerd.yml"));
    const cli = run(join(action, "dist", "main.js"), ["validate", "--rules", join(checkout, ".github", "triggerd.yml")]);
    assert.equal(cli.status, 0, cli.stderr);
    const gate = run(join(action, "dist", "gate.js"), [], { env: {
      ...process.env,
      RULES_JSON: JSON.stringify({ "api.check": true }),
      NEEDS_JSON: JSON.stringify({ plan: { result: "success" }, api: { result: "success" } }),
      BINDINGS_JSON: JSON.stringify({ planner: "plan", bindings: { "api.check": "api" }, needs: { api: ["plan"] } })
    } });
    assert.equal(gate.status, 0, gate.stderr);
    assert.deepEqual(JSON.parse(gate.stdout), { selected: ["api.check"], overhead: [] });
    const eventPath = join(temporary, "event.json");
    const outputPath = join(temporary, "output.txt");
    const summaryPath = join(temporary, "summary.md");
    writeFileSync(eventPath, JSON.stringify({ repository: { full_name: "aRustyDev/thrust" } }));
    writeFileSync(outputPath, "");
    writeFileSync(summaryPath, "");
    const actionRun = run(join(action, "dist", "main.js"), [], {
      cwd: checkout,
      env: {
        ...process.env,
        GITHUB_ACTIONS: "true",
        GITHUB_EVENT_NAME: "workflow_dispatch",
        GITHUB_EVENT_PATH: eventPath,
        GITHUB_SHA: sha,
        GITHUB_WORKSPACE: checkout,
        GITHUB_OUTPUT: outputPath,
        GITHUB_STEP_SUMMARY: summaryPath,
        INPUT_RULES_FILE: ".github/triggerd.yml",
        INPUT_SCOPE: "",
        "INPUT_GITHUB-TOKEN": "fixture-token",
        INPUT_FULL_CHECK: "false"
      }
    });
    assert.equal(actionRun.status, 0, `${actionRun.stderr}\n${actionRun.stdout}`);
    const output = readFileSync(outputPath, "utf8");
    assert.match(output, /^rules<</m);
    assert.match(output, /^details<</m);
  } finally { rmSync(temporary, { recursive: true, force: true }); }
});

test("check:dist rejects stale, missing, and extra installed files", () => {
  const candidates = ["stale", "missing", "extra"];
  for (const candidate of candidates) {
    const target = join(root, `dist-${candidate}`);
    try {
      cpSync(join(root, "dist"), target, { recursive: true });
      if (candidate === "stale") writeFileSync(join(target, "main.js"), "stale\n");
      if (candidate === "missing") unlinkSync(join(target, "licenses.txt"));
      if (candidate === "extra") writeFileSync(join(target, "unexpected.js"), "extra\n");
      const result = run(join(root, "scripts", "check-dist.mjs"), ["--dist", `dist-${candidate}`], { cwd: root });
      assert.notEqual(result.status, 0, `${candidate} distribution unexpectedly passed`);
    } finally { rmSync(target, { recursive: true, force: true }); }
  }
});
