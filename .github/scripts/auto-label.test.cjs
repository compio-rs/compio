const test = require("node:test");
const assert = require("node:assert/strict");

const {
  collectConventionalLabels,
  collectIssueKeywordLabels,
  collectLabels,
} = require("./auto-label.cjs");

test("maps conventional commit types and multiple scopes", () => {
  assert.deepEqual(collectConventionalLabels("feat(executor,rt): spawn_unchecked"), [
    "enhancement",
    "package: executor",
    "package: runtime",
  ]);
});

test("maps driver package and driver backend aliases", () => {
  assert.deepEqual(
    collectConventionalLabels("fix(driver,iour): make Driver non-Send"),
    ["bug", "package: driver", "driver: io-uring"],
  );
});

test("tracks breaking and refactor labels from conventional titles", () => {
  assert.deepEqual(collectConventionalLabels("refactor!: use rustix"), [
    "enhancement",
    "breaking change",
  ]);
});

test("supports multiple workspace crates in scope", () => {
  assert.deepEqual(
    collectConventionalLabels("feat(runtime,fs,net): high-level multishot"),
    ["enhancement", "package: runtime", "package: fs", "package: net"],
  );
});

test("matches issue keywords when title is not conventional", () => {
  assert.deepEqual(
    collectIssueKeywordLabels("RFC: Feature request for better perf in runtime"),
    ["RFC", "enhancement", "performance"],
  );
});

test("only falls back to keyword matching for issues", () => {
  assert.deepEqual(
    collectLabels("BUG: wakeup lost under load", { isIssue: true }),
    ["bug"],
  );
  assert.deepEqual(collectLabels("BUG: wakeup lost under load", { isIssue: false }), []);
});
