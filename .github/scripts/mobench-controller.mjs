export const COMPILE_GATE_WORKFLOW_FILE = 'compile-gate.yml';
export const COMPILE_GATE_WORKFLOW_NAME = 'Compile Gate';
export const TRUSTED_ASSOCIATIONS = new Set([
  'OWNER',
  'MEMBER',
  'COLLABORATOR',
]);

export const DEFAULT_INPUTS = {
  platform: 'both',
  device_profile: 'low-spec',
  ios_device: '',
  ios_os_version: '',
  android_device: '',
  android_os_version: '',
  iterations: '30',
  warmup: '5',
};

const MOBENCH_COMMAND = '/mobench';
const ALLOWED_KEYS = new Set(Object.keys(DEFAULT_INPUTS));

export function isTrustedAssociation(association) {
  return TRUSTED_ASSOCIATIONS.has(association ?? '');
}

export function isSameRepoPullRequest(pullRequest, repositoryFullName) {
  return pullRequest?.head?.repo?.full_name === repositoryFullName;
}

export function hasBenchLabel(pullRequest) {
  return (pullRequest?.labels ?? []).some((label) => label.name === 'bench');
}

export function parseMobenchCommand(body) {
  const commandLine = body
    ?.split(/\r?\n/u)
    .map((line) => line.trim())
    .find((line) => line.length > 0);
  if (!commandLine?.startsWith(MOBENCH_COMMAND)) {
    return null;
  }

  const command = commandLine.slice(MOBENCH_COMMAND.length);
  if (command.length > 0 && !/^\s/u.test(command)) {
    return null;
  }

  const args = { ...DEFAULT_INPUTS };
  const remainder = command.trim();
  if (remainder.length === 0) {
    return args;
  }

  let currentKey = null;
  let currentValue = '';

  for (const token of remainder.split(/\s+/u)) {
    const separatorIndex = token.indexOf('=');
    if (separatorIndex !== -1) {
      if (!applyToken(args, currentKey, currentValue)) {
        return null;
      }
      currentValue = '';

      const key = token.slice(0, separatorIndex);
      const value = token.slice(separatorIndex + 1);
      if (!ALLOWED_KEYS.has(key)) {
        return null;
      }

      currentKey = key;
      currentValue = value;
      continue;
    }

    if (!currentKey) {
      return null;
    }

    currentValue = currentValue
      ? `${currentValue} ${token}`
      : token;
  }

  if (!applyToken(args, currentKey, currentValue)) {
    return null;
  }

  return validateArgs(args) ? args : null;
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
    ...DEFAULT_INPUTS,
    ...overrides,
    pr_number: String(prNumber),
    base_ref: baseRef,
    requested_by: requestedBy,
    trigger_source: triggerSource,
    request_command: requestCommand ?? '',
    dispatch_id: '',
  };
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

function validateArgs(args) {
  if (!['ios', 'android', 'both'].includes(args.platform)) {
    return false;
  }

  return [args.iterations, args.warmup].every(isPositiveInteger);
}

function isPositiveInteger(value) {
  if (!/^\d+$/u.test(value)) {
    return false;
  }

  return Number.parseInt(value, 10) > 0;
}
