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
  const records = entries.map(({ features, name }) => ({
    features,
    id: `${name} 0.1.0`,
    name,
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

function mockExec({
  baseline,
  current,
  outputs = {},
  semverArgs = [],
  statuses = {},
}) {
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
        stdout: outputs[packageName] ?? "",
      };
    },
  };
}

test("reports incompatible and removed checked crates", async () => {
  const core = mockCore();
  const exec = mockExec({
    baseline: ["compio", "compio-driver", "compio-fs"],
    current: ["compio", "compio-net"],
    statuses: { compio: 100 },
    outputs: { compio: "compio compatibility report" },
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

    assert.deepEqual(breaking, ["compio", "compio-driver"]);
    assert.deepEqual(core.outputs, {
      breaking: true,
      crates: "compio\ncompio-driver",
    });
    const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
    assert.deepEqual(report.crates, ["compio", "compio-driver"]);
    assert.match(report.report, /\[compio\]/);
    assert.match(report.report, /compio compatibility report/);
    assert.match(report.report, /\[compio-driver\]/);
    assert.match(report.report, /removed from the current workspace/);
  } finally {
    fs.rmSync(tempDir, { force: true, recursive: true });
  }
});

test("reports a compatible workspace", async () => {
  const core = mockCore();
  const exec = mockExec({
    baseline: ["compio", "compio-driver", "compio-runtime"],
    current: ["compio", "compio-driver", "compio-runtime"],
  });

  const breaking = await check({ baselineRev: "base", core, exec });

  assert.deepEqual(breaking, []);
  assert.deepEqual(core.outputs, { breaking: false, crates: "" });
});

test("checks only compio and compio-driver", async () => {
  const core = mockCore();
  const semverArgs = [];
  const packages = ["compio", "compio-driver", "compio-macros", "compio-net"];
  const exec = mockExec({
    baseline: packages,
    current: packages,
    semverArgs,
  });

  await check({ baselineRev: "base", core, exec });

  assert.deepEqual(
    semverArgs.map((args) => args[args.indexOf("--package") + 1]),
    ["compio", "compio-driver"],
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
      name: "compio-driver",
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
      "compio-driver",
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
  const github = {
    paginate: async () => [],
    rest: {
      issues: {
        addLabels: async (args) => calls.push(["addLabels", args]),
        createComment: async (args) => calls.push(["createComment", args]),
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
    JSON.stringify({
      crates: ["compio-runtime", "compio-net"],
      report:
        "[compio-runtime]\n<breaking> runtime API\n\n[compio-net]\nremoved API",
    }),
  );
  try {
    assert.equal(
      await reportFromFile({
        core: mockCore(),
        github,
        context,
        issueNumber: 42,
        reportPath,
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
  assert.match(comment.body, /<details>/);
  assert.match(comment.body, /<summary>cargo-semver-checks report<\/summary>/);
  assert.match(comment.body, /<pre><code>/);
  assert.match(comment.body, /&lt;breaking&gt; runtime API/);
  assert.match(comment.body, /<\/code><\/pre>/);
  assert.doesNotMatch(comment.body, /workflow run|actions\/runs/);
  assert.ok(calls.some(([name]) => name === "addLabels"));
  assert.ok(!calls.some(([name]) => name === "getLabel"));
  assert.ok(!calls.some(([name]) => name === "createLabel"));
});
