export const COMPILE_GATE_WORKFLOW_FILE = 'compile-gate.yml';
export const COMPILE_GATE_WORKFLOW_NAME = 'Compile Gate';
export const MOBILE_BENCH_WORKFLOW_FILE = 'mobile-bench.yml';
export const TRUSTED_ASSOCIATIONS = new Set([
  'OWNER',
  'MEMBER',
  'COLLABORATOR',
]);

export const DEFAULTS = {
  platform: 'both',
  device_profile: 'low-spec',
  ios_device: '',
  ios_os_version: '',
  android_device: '',
  android_os_version: '',
  iterations: '30',
  warmup: '5',
};

const ALLOWED_KEYS = new Set(Object.keys(DEFAULTS));
const VALID_PLATFORMS = new Set(['ios', 'android', 'both']);

export function isTrustedAssociation(association) {
  return TRUSTED_ASSOCIATIONS.has(association ?? '');
}

export function isSameRepoPullRequest(pullRequest, repositoryFullName) {
  return pullRequest?.head?.repo?.full_name === repositoryFullName;
}

export function hasBenchLabel(pullRequest) {
  return (pullRequest?.labels ?? []).some((label) => label?.name === 'bench');
}

export function parseMobenchCommand(body) {
  const firstNonEmptyLine = body
    ?.split('\n')
    .map((line) => line.trim())
    .find((line) => line.length > 0);
  if (!firstNonEmptyLine?.startsWith('/mobench')) {
    return null;
  }

  const command = firstNonEmptyLine.slice('/mobench'.length);
  if (command.length > 0 && !/^\s/.test(command)) {
    return null;
  }

  const remainder = command.trim();
  const args = { ...DEFAULTS };
  if (remainder.length === 0) {
    return args;
  }

  let currentKey = null;
  let currentValue = '';

  for (const token of remainder.split(/\s+/)) {
    const equalsIndex = token.indexOf('=');
    if (equalsIndex !== -1) {
      if (!applyToken(args, currentKey, currentValue)) {
        return null;
      }
      currentValue = '';

      const key = token.slice(0, equalsIndex);
      const value = token.slice(equalsIndex + 1);
      if (!ALLOWED_KEYS.has(key)) {
        return null;
      }

      currentKey = key;
      currentValue = value;
    } else if (currentKey) {
      currentValue = currentValue.length === 0 ? token : `${currentValue} ${token}`;
    } else {
      return null;
    }
  }

  if (!applyToken(args, currentKey, currentValue)) {
    return null;
  }

  if (!VALID_PLATFORMS.has(args.platform)) {
    return null;
  }

  if (!isPositiveInteger(args.iterations) || !isPositiveInteger(args.warmup)) {
    return null;
  }

  return args;
}

export function buildDispatchInputs({
  prNumber,
  baseRef,
  requestedBy,
  triggerSource,
  requestCommand,
  overrides,
}) {
  return {
    ...DEFAULTS,
    ...overrides,
    pr_number: String(prNumber),
    base_ref: baseRef,
    requested_by: requestedBy,
    trigger_source: triggerSource,
    request_command: requestCommand ?? '',
    dispatch_id: '',
  };
}

export function decideWorkflowRunDispatch({
  workflowRun,
  pullRequest,
  repositoryFullName,
}) {
  if (workflowRun?.conclusion !== 'success') {
    return { dispatch: false, reason: 'compile-gate-failed' };
  }
  if (!pullRequest || pullRequest.state !== 'open') {
    return { dispatch: false, reason: 'no-open-pr' };
  }
  if (!isSameRepoPullRequest(pullRequest, repositoryFullName)) {
    return { dispatch: false, reason: 'fork-pr' };
  }
  if (!hasBenchLabel(pullRequest)) {
    return { dispatch: false, reason: 'bench-label-missing' };
  }

  return {
    dispatch: true,
    ref: pullRequest.head.ref,
    inputs: buildDispatchInputs({
      prNumber: pullRequest.number,
      baseRef: pullRequest.base.ref,
      requestedBy: 'github-actions',
      triggerSource: 'label',
      requestCommand: '',
      overrides: DEFAULTS,
    }),
  };
}

export async function handleWorkflowRun({ github, context, core }) {
  const workflowRun = context.payload.workflow_run;
  const repositoryFullName =
    context.payload.repository?.full_name ?? `${context.repo.owner}/${context.repo.repo}`;
  const pullRequest = await resolveWorkflowRunPullRequest({
    github,
    owner: context.repo.owner,
    repo: context.repo.repo,
    workflowRun,
  });

  const decision = decideWorkflowRunDispatch({
    workflowRun,
    pullRequest,
    repositoryFullName,
  });
  if (!decision.dispatch) {
    core.info(`Skipping mobile bench auto-dispatch: ${decision.reason}`);
    return decision;
  }

  await github.rest.actions.createWorkflowDispatch({
    owner: context.repo.owner,
    repo: context.repo.repo,
    workflow_id: MOBILE_BENCH_WORKFLOW_FILE,
    ref: decision.ref,
    inputs: decision.inputs,
  });

  core.info(
    `Dispatched ${MOBILE_BENCH_WORKFLOW_FILE} for PR #${pullRequest.number} at ${decision.ref}`,
  );
  return decision;
}

function applyToken(args, key, value) {
  if (!key) {
    return true;
  }

  if (!ALLOWED_KEYS.has(key)) {
    return false;
  }

  args[key] = value;
  return true;
}

function isPositiveInteger(value) {
  return /^[1-9]\d*$/.test(value);
}

async function resolveWorkflowRunPullRequest({ github, owner, repo, workflowRun }) {
  const explicitNumber = workflowRun?.pull_requests?.[0]?.number;
  if (explicitNumber) {
    const response = await github.rest.pulls.get({
      owner,
      repo,
      pull_number: explicitNumber,
    });
    return response.data;
  }

  const response = await github.request(
    'GET /repos/{owner}/{repo}/commits/{commit_sha}/pulls',
    {
      owner,
      repo,
      commit_sha: workflowRun.head_sha,
    },
  );
  const pullRequestNumber = response.data?.[0]?.number;
  if (!pullRequestNumber) {
    return null;
  }

  const pullRequest = await github.rest.pulls.get({
    owner,
    repo,
    pull_number: pullRequestNumber,
  });
  return pullRequest.data;
}
