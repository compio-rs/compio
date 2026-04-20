const TYPE_LABELS = {
  ci: "ci",
  fix: "bug",
  feat: "enhancement",
  refactor: "enhancement",
  perf: "performance",
  doc: "documentation",
  docs: "documentation"
};

const SCOPE_LABELS = {
  buf: "package: buf",
  dispatch: "package: dispatcher",
  dispatcher: "package: dispatcher",
  driver: "package: driver",
  iocp: "driver: iocp",
  iour: "driver: io-uring",
  poll: "driver: polling",
  fusion: "driver: fusion",
  stub: "driver: stub",
  executor: "package: executor",
  fs: "package: fs",
  io: "package: io",
  log: "package: log",
  macro: "package: macros",
  macros: "package: macros",
  net: "package: net",
  polling: "driver: polling",
  process: "package: process",
  quic: "package: quic",
  rt: "package: runtime",
  runtime: "package: runtime",
  signal: "package: signal",
  tls: "package: tls",
  uring: "driver: io-uring",
  websocket: "package: ws",
  ws: "package: ws",
};

const ISSUE_KEYWORDS = [
  { re: /\brfc\b/i, labels: ["RFC"] },
  {
    re: /\b(feature request|feature|enhancement)\b/i,
    labels: ["enhancement"],
  },
  {
    re: /\bbug\b|\bfix\b|\bregression\b|\bpanic\b|\bcrash\b|\bincorrect\b/i,
    labels: ["bug"],
  },
  {
    re: /\bperf(?:ormance)?\b|\bslow\b|\blatency\b|\bthroughput\b/i,
    labels: ["performance"],
  },
  { re: /\brefactor\b/i, labels: ["enhancement", "refactor"] },
  { re: /\bci\b|\bworkflow\b|\bgithub actions?\b/i, labels: ["ci"] },
  { re: /\bdocs?\b|\bdocumentation\b/i, labels: ["documentation"] },
  { re: /\bquestion\b|\?\s*$/i, labels: ["question"] },
];

const CONVENTIONAL_RE =
  /^(?<type>[a-z][a-z0-9-]*)(?:\((?<scope>[^)]+)\))?(?<breaking>!)?:\s+(?<main>.+)$/i;

function unique(values) {
  return [...new Set(values)];
}

function canonicalizeScopeToken(token) {
  return token
    .trim()
    .toLowerCase()
    .replace(/^compio-/, "")
    .replace(/^package:\s*/, "")
    .replace(/^driver:\s*/, "");
}

function collectScopeLabels(scope) {
  if (!scope) {
    return [];
  }

  const labels = [];
  for (const rawToken of scope.split(/[,\s/&+]+/)) {
    const token = canonicalizeScopeToken(rawToken);
    if (!token) {
      continue;
    }

    const direct = SCOPE_LABELS[token];
    if (direct) {
      labels.push(direct);
      continue;
    }

    for (const fragment of token.split("-")) {
      const mapped = SCOPE_LABELS[fragment];
      if (mapped) {
        labels.push(mapped);
      }
    }
  }

  return unique(labels);
}

function collectConventionalLabels(title) {
  const match = title.match(CONVENTIONAL_RE);
  if (!match) {
    return [];
  }

  const { type, scope, breaking } = match.groups;
  const labels = [];
  const normalizedType = type.toLowerCase();

  if (TYPE_LABELS[normalizedType]) {
    labels.push(TYPE_LABELS[normalizedType]);
  }

  labels.push(...collectScopeLabels(scope));

  if (breaking) {
    labels.push("breaking change");
  }

  return unique(labels);
}

function collectIssueKeywordLabels(title) {
  const labels = [];
  for (const { re, labels: mapped } of ISSUE_KEYWORDS) {
    if (re.test(title)) {
      labels.push(...mapped);
    }
  }
  return unique(labels);
}

function collectLabels(title, { isIssue }) {
  const conventionalLabels = collectConventionalLabels(title);
  if (conventionalLabels.length > 0) {
    return conventionalLabels;
  }

  if (isIssue) {
    return collectIssueKeywordLabels(title);
  }

  return [];
}

async function listRepoLabels(github, context) {
  return github.paginate(github.rest.issues.listLabelsForRepo, {
    owner: context.repo.owner,
    per_page: 100,
    repo: context.repo.repo,
  });
}

async function run({ github, context, core }) {
  const isPullRequest = context.eventName === "pull_request_target";
  const item = isPullRequest ? context.payload.pull_request : context.payload.issue;

  if (!item?.title?.trim()) {
    core.info("No title found; skipping.");
    return;
  }

  const repoLabels = await listRepoLabels(github, context);
  const availableLabels = new Set(repoLabels.map(({ name }) => name));
  const existingLabels = new Set(
    (item.labels || []).map((label) =>
      typeof label === "string" ? label : label.name,
    ),
  );

  const desiredLabels = collectLabels(item.title, { isIssue: !isPullRequest })
    .filter((label) => availableLabels.has(label))
    .filter((label) => !existingLabels.has(label));

  if (desiredLabels.length === 0) {
    core.info(`No new labels matched title: "${item.title}"`);
    return;
  }

  core.info(`Adding labels: ${desiredLabels.join(", ")}`);
  await github.rest.issues.addLabels({
    issue_number: item.number,
    labels: desiredLabels,
    owner: context.repo.owner,
    repo: context.repo.repo,
  });
}

module.exports = {
  collectConventionalLabels,
  collectIssueKeywordLabels,
  collectLabels,
  run,
};
