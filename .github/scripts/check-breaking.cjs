const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const COMMENT_MARKER = "<!-- cargo-semver-checks: breaking-change -->";
const LABEL = "breaking change";

const CHECKABLE_TARGET_KINDS = new Set([
  "lib",
  "rlib",
  "dylib",
  "cdylib",
  "staticlib",
]);

function hasCheckableTarget(targets = []) {
  return targets.some(({ kind = [] }) =>
    kind.some((targetKind) => CHECKABLE_TARGET_KINDS.has(targetKind)),
  );
}

function stableFeatureNames(features) {
  const unstable = new Set();
  const visit = (name) => {
    if (unstable.has(name) || !Object.hasOwn(features, name)) {
      return;
    }
    unstable.add(name);
    for (const member of features[name]) {
      visit(member);
    }
  };
  visit("nightly");

  let changed;
  do {
    changed = false;
    for (const [name, members] of Object.entries(features)) {
      if (
        !unstable.has(name) &&
        members.some((member) => unstable.has(member))
      ) {
        unstable.add(name);
        changed = true;
      }
    }
  } while (changed);

  return Object.keys(features)
    .filter((name) => !unstable.has(name))
    .sort();
}

function workspacePackages(metadata) {
  const members = new Set(metadata.workspace_members);
  return metadata.packages
    .filter(
      ({ id, targets }) => members.has(id) && hasCheckableTarget(targets),
    )
    .map(({ features = {}, name }) => ({
      name,
      stableFeatures: stableFeatureNames(features),
    }))
    .sort(({ name: left }, { name: right }) => left.localeCompare(right));
}

function classifyPackages(currentPackages, baselinePackages) {
  const current = new Map(
    currentPackages.map((package) => [package.name, package]),
  );
  const baseline = new Set(baselinePackages.map(({ name }) => name));
  return {
    common: [...current.values()].filter(({ name }) => baseline.has(name)),
    removed: [...baseline].filter((name) => !current.has(name)).sort(),
  };
}

async function readWorkspacePackages(exec, manifestPath) {
  const result = await exec.getExecOutput(
    "cargo",
    [
      "metadata",
      "--manifest-path",
      manifestPath,
      "--no-deps",
      "--format-version",
      "1",
    ],
    { silent: true },
  );
  return workspacePackages(JSON.parse(result.stdout));
}

async function checkPackage({
  baselineRev,
  core,
  exec,
  packageName,
  stableFeatures,
}) {
  core.startGroup(`Checking ${packageName}`);
  const args = [
    "semver-checks",
    "check-release",
    "--package",
    packageName,
    "--baseline-rev",
    baselineRev,
    "--release-type",
    "minor",
    "--only-explicit-features",
  ];
  if (stableFeatures.length > 0) {
    args.push("--features", stableFeatures.join(","));
  }

  let result;
  try {
    result = await exec.getExecOutput("cargo", args, {
      ignoreReturnCode: true,
    });
  } finally {
    core.endGroup();
  }

  if (result.exitCode === 0) {
    return false;
  }
  if (result.exitCode === 100) {
    return true;
  }
  throw new Error(
    `cargo-semver-checks could not check ${packageName} (exit code ${result.exitCode}).`,
  );
}

async function check({
  baselineRev = process.env.BASELINE_REV,
  core,
  cwd = process.cwd(),
  exec,
  reportPath = process.env.BREAKING_REPORT_PATH,
}) {
  if (!baselineRev) {
    throw new Error("BASELINE_REV is required.");
  }

  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "compio-semver-"));
  const baselineDir = path.join(tempDir, "baseline");
  let worktreeAdded = false;

  try {
    await exec.exec("git", ["worktree", "add", "--detach", baselineDir, baselineRev]);
    worktreeAdded = true;

    const currentPackages = await readWorkspacePackages(
      exec,
      path.join(cwd, "Cargo.toml"),
    );
    const baselinePackages = await readWorkspacePackages(
      exec,
      path.join(baselineDir, "Cargo.toml"),
    );
    const { common, removed } = classifyPackages(
      currentPackages,
      baselinePackages,
    );
    const breaking = [...removed];

    if (removed.length > 0) {
      core.info(`Removed workspace crates: ${removed.join(", ")}`);
    }

    for (const { name, stableFeatures } of common) {
      if (
        await checkPackage({
          baselineRev,
          core,
          exec,
          packageName: name,
          stableFeatures,
        })
      ) {
        breaking.push(name);
      }
    }

    breaking.sort();
    core.setOutput("breaking", breaking.length > 0);
    core.setOutput("crates", breaking.join("\n"));
    if (reportPath) {
      fs.mkdirSync(path.dirname(reportPath), { recursive: true });
      fs.writeFileSync(reportPath, `${JSON.stringify({ crates: breaking })}\n`);
    }
    return breaking;
  } finally {
    if (worktreeAdded) {
      await exec.exec("git", ["worktree", "remove", "--force", baselineDir], {
        ignoreReturnCode: true,
        silent: true,
      });
    }
    fs.rmSync(tempDir, { force: true, recursive: true });
  }
}

function buildReportBody({ crates, runUrl }) {
  const crateList = crates.map((name) => `- \`${name}\``).join("\n");
  return `${COMMENT_MARKER}
### Breaking API change detected

\`cargo-semver-checks\` found breaking public API changes in these crates:

${crateList}

Add \`!\` before the colon in the PR title, for example \`feat!: ...\` or \`feat(runtime)!: ...\`. In the PR description, explain the affected API, the impact on users, and the migration path.

See the [workflow run](${runUrl}) for the full diagnostics.`;
}

function readReport(reportPath) {
  return JSON.parse(fs.readFileSync(reportPath, "utf8"));
}

async function report({
  crates,
  github,
  context,
  issueNumber = context.issue?.number,
  runUrl = `https://github.com/${context.repo.owner}/${context.repo.repo}/actions/runs/${context.runId}`,
}) {
  if (crates.length === 0) {
    throw new Error("At least one breaking crate is required.");
  }
  if (!issueNumber) {
    throw new Error("A pull request number is required.");
  }

  const body = buildReportBody({ crates, runUrl });

  try {
    await github.rest.issues.getLabel({
      ...context.repo,
      name: LABEL,
    });
  } catch (error) {
    if (error.status !== 404) {
      throw error;
    }
    await github.rest.issues.createLabel({
      ...context.repo,
      name: LABEL,
      color: "b60205",
      description: "Introduces a breaking public API change",
    });
  }

  await github.rest.issues.addLabels({
    ...context.repo,
    issue_number: issueNumber,
    labels: [LABEL],
  });

  const comments = await github.paginate(github.rest.issues.listComments, {
    ...context.repo,
    issue_number: issueNumber,
    per_page: 100,
  });
  const existing = comments.find(
    (comment) =>
      comment.user?.type === "Bot" && comment.body?.includes(COMMENT_MARKER),
  );

  if (existing) {
    await github.rest.issues.updateComment({
      ...context.repo,
      comment_id: existing.id,
      body,
    });
  } else {
    await github.rest.issues.createComment({
      ...context.repo,
      issue_number: issueNumber,
      body,
    });
  }
}

async function reportFromFile({ core, reportPath, ...options }) {
  const { crates } = readReport(reportPath);
  if (!Array.isArray(crates)) {
    throw new Error("The breaking change report has no crate list.");
  }
  if (crates.length === 0) {
    core.info("No breaking public API changes detected.");
    return false;
  }

  await report({ crates, ...options });
  return true;
}

module.exports = {
  buildReportBody,
  check,
  classifyPackages,
  readReport,
  report,
  reportFromFile,
  stableFeatureNames,
  workspacePackages,
};
