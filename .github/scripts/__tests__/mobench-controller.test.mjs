import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const fixturesDir = path.join(
  process.cwd(),
  'services',
  'mobench-webhook',
  'tests',
  'fixtures',
);

const fixture = (name) =>
  JSON.parse(fs.readFileSync(path.join(fixturesDir, name), 'utf8'));

const controller = await import('../mobench-controller.mjs');

test('parseMobenchCommand keeps existing webhook defaults', () => {
  assert.deepEqual(controller.parseMobenchCommand('/mobench'), {
    platform: 'both',
    device_profile: 'low-spec',
    ios_device: '',
    ios_os_version: '',
    android_device: '',
    android_os_version: '',
    iterations: '30',
    warmup: '5',
  });
});

test('parseMobenchCommand preserves values with spaces', () => {
  assert.equal(
    controller.parseMobenchCommand(
      '/mobench platform=ios iterations=50 ios_device=iPhone 15 ios_os_version=17',
    ).ios_device,
    'iPhone 15',
  );
});

test('parseMobenchCommand rejects invalid keys and values', () => {
  assert.equal(controller.parseMobenchCommand('/mobench foo=bar'), null);
  assert.equal(controller.parseMobenchCommand('/mobench iterations=0'), null);
});

test('trusted associations are OWNER MEMBER and COLLABORATOR only', () => {
  assert.equal(controller.isTrustedAssociation('MEMBER'), true);
  assert.equal(controller.isTrustedAssociation('COLLABORATOR'), true);
  assert.equal(controller.isTrustedAssociation('FIRST_TIME_CONTRIBUTOR'), false);
});

test('buildDispatchInputs matches the existing webhook normalization contract', () => {
  const inputs = controller.buildDispatchInputs({
    prNumber: 123,
    baseRef: 'release/1.2',
    requestedBy: 'octocat',
    triggerSource: 'pr_comment',
    requestCommand:
      '/mobench platform=ios iterations=50 ios_device=iPhone 15 ios_os_version=17',
    overrides: controller.parseMobenchCommand(
      '/mobench platform=ios iterations=50 ios_device=iPhone 15 ios_os_version=17',
    ),
  });

  assert.equal(inputs.pr_number, '123');
  assert.equal(inputs.base_ref, 'release/1.2');
  assert.equal(inputs.requested_by, 'octocat');
  assert.equal(inputs.trigger_source, 'pr_comment');
  assert.equal(
    inputs.request_command,
    '/mobench platform=ios iterations=50 ios_device=iPhone 15 ios_os_version=17',
  );
});

test('same-repo bench label is required for auto dispatch', () => {
  const payload = fixture('pull_request_labeled_bench.json');
  const pullRequest = {
    ...payload.pull_request,
    labels: [payload.label],
  };
  assert.equal(
    controller.isSameRepoPullRequest(
      pullRequest,
      'world/mobile-bench-rs',
    ),
    true,
  );
  assert.equal(controller.hasBenchLabel(pullRequest), true);
});

test('compile gate workflow file exists with stable name', () => {
  const yaml = fs.readFileSync('.github/workflows/compile-gate.yml', 'utf8');
  assert.match(yaml, /^name: Compile Gate$/m);
  assert.match(yaml, /cargo test --all --locked --no-run/);
});
