const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { check, reportFromFile } = require("./check-breaking.cjs");

function metadata(packages) {
  const entries = packages.map((package) =>
    typeof package === "string" ? { name: package, features: {} } : package,
  );
  const records = entries.map(({ features, name, targets }) => ({
    features,
    id: `${name} 0.1.0`,
    name,
    targets: targets ?? [{ crate_types: ["lib"], kind: ["lib"] }],
  }));
  return {
    packages: records,
    workspace_members: records.map(({ id }) => id),
  };
}

function mockCore() {
  return {
    groups: [],
    info() {},
    outputs: {},
    endGroup() {},
    setOutput(name, value) {
      this.outputs[name] = value;
    },
    startGroup(name) {
      this.groups.push(name);
    },
  };
}

function mockExec({ baseline, current, semverArgs = [], statuses = {} }) {
  let metadataCall = 0;
  return {
    async exec() {
      return 0;
    },
    async getExecOutput(tool, args) {
      assert.equal(tool, "cargo");
      if (args[0] === "metadata") {
        const value = metadataCall++ === 0 ? current : baseline;
        return { exitCode: 0, stderr: "", stdout: JSON.stringify(metadata(value)) };
      }

      semverArgs.push(args);
      const packageName = args[args.indexOf("--package") + 1];
      return {
        exitCode: statuses[packageName] ?? 0,
        stderr: "",
        stdout: "",
      };
    },
  };
}

test("reports incompatible and removed workspace crates", async () => {
  const core = mockCore();
  const exec = mockExec({
    baseline: ["compio", "compio-fs", "compio-runtime"],
    current: ["compio", "compio-net", "compio-runtime"],
    statuses: { "compio-runtime": 100 },
  });
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "compio-semver-test-"));
  const reportPath = path.join(tempDir, "report.json");
  try {
    const breaking = await check({
      baselineRev: "base",
      core,
      exec,
      reportPath,
    });

    assert.deepEqual(breaking, ["compio-fs", "compio-runtime"]);
    assert.deepEqual(core.outputs, {
      breaking: true,
      crates: "compio-fs\ncompio-runtime",
    });
    assert.deepEqual(JSON.parse(fs.readFileSync(reportPath, "utf8")), {
      crates: ["compio-fs", "compio-runtime"],
    });
  } finally {
    fs.rmSync(tempDir, { force: true, recursive: true });
  }
});

test("reports a compatible workspace", async () => {
  const core = mockCore();
  const exec = mockExec({
    baseline: ["compio", "compio-runtime"],
    current: ["compio", "compio-runtime"],
  });

  const breaking = await check({ baselineRev: "base", core, exec });

  assert.deepEqual(breaking, []);
  assert.deepEqual(core.outputs, { breaking: false, crates: "" });
});

test("skips crates without checkable library targets", async () => {
  const core = mockCore();
  const semverArgs = [];
  const packages = [
    "compio",
    {
      features: {},
      name: "compio-macros",
      targets: [{ crate_types: ["proc-macro"], kind: ["proc-macro"] }],
    },
  ];
  const exec = mockExec({
    baseline: packages,
    current: packages,
    semverArgs,
  });

  await check({ baselineRev: "base", core, exec });

  assert.deepEqual(
    semverArgs.map((args) => args[args.indexOf("--package") + 1]),
    ["compio"],
  );
});

test("checks one package with stable features only", async () => {
  const core = mockCore();
  const semverArgs = [];
  const packages = [
    {
      features: {
        all: ["bytes"],
        bytes: ["dep:bytes"],
        default: ["bytes"],
        nightly: ["read_buf"],
        "nightly-all": ["nightly"],
        read_buf: [],
      },
      name: "compio-buf",
    },
  ];
  const exec = mockExec({
    baseline: packages,
    current: packages,
    semverArgs,
  });

  await check({ baselineRev: "base", core, exec });

  assert.deepEqual(semverArgs, [
    [
      "semver-checks",
      "check-release",
      "--package",
      "compio-buf",
      "--baseline-rev",
      "base",
      "--release-type",
      "minor",
      "--only-explicit-features",
      "--features",
      "all,bytes,default",
    ],
  ]);
});

test("fails when cargo-semver-checks cannot complete", async () => {
  const core = mockCore();
  const exec = mockExec({
    baseline: ["compio"],
    current: ["compio"],
    statuses: { compio: 101 },
  });

  await assert.rejects(
    check({ baselineRev: "base", core, exec }),
    /could not check compio \(exit code 101\)/,
  );
});

test("labels the PR and lists each breaking crate", async () => {
  const calls = [];
  const missingLabel = new Error("missing label");
  missingLabel.status = 404;
  const github = {
    paginate: async () => [],
    rest: {
      issues: {
        addLabels: async (args) => calls.push(["addLabels", args]),
        createComment: async (args) => calls.push(["createComment", args]),
        createLabel: async (args) => calls.push(["createLabel", args]),
        getLabel: async () => {
          throw missingLabel;
        },
        listComments: async () => {},
      },
    },
  };
  const context = {
    issue: { number: 42 },
    repo: { owner: "compio-rs", repo: "compio" },
    runId: 123,
  };

  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "compio-report-test-"));
  const reportPath = path.join(tempDir, "report.json");
  fs.writeFileSync(
    reportPath,
    JSON.stringify({ crates: ["compio-runtime", "compio-net"] }),
  );
  try {
    assert.equal(
      await reportFromFile({
        core: mockCore(),
        github,
        context,
        issueNumber: 42,
        reportPath,
        runUrl: "https://github.com/compio-rs/compio/actions/runs/123",
      }),
      true,
    );
  } finally {
    fs.rmSync(tempDir, { force: true, recursive: true });
  }

  const comment = calls.find(([name]) => name === "createComment")[1];
  assert.match(comment.body, /- `compio-runtime`/);
  assert.match(comment.body, /- `compio-net`/);
  assert.match(comment.body, /`feat!: \.\.\.`/);
  assert.match(comment.body, /migration path/);
  assert.ok(calls.some(([name]) => name === "createLabel"));
  assert.ok(calls.some(([name]) => name === "addLabels"));
});
