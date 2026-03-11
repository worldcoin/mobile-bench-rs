import test from 'node:test';
import assert from 'node:assert/strict';
import {
  resolveBaselineRun,
  selectBaselineRun,
} from '../mobench-baseline.mjs';

test('selectBaselineRun prefers the latest successful base-ref run with mobench-history-v1', () => {
  const run = selectBaselineRun({
    baseRef: 'main',
    currentRunId: 200,
    runs: [
      {
        id: 200,
        head_branch: 'feature/pr',
        conclusion: 'success',
        artifacts: ['mobench-history-v1'],
      },
      {
        id: 150,
        head_branch: 'main',
        conclusion: 'failure',
        artifacts: ['mobench-history-v1'],
      },
      {
        id: 140,
        head_branch: 'main',
        conclusion: 'success',
        artifacts: ['mobench-history-v1'],
      },
    ],
  });

  assert.equal(run.id, 140);
});

test('selectBaselineRun returns null when there is no successful base-ref benchmark artifact', () => {
  assert.equal(
    selectBaselineRun({
      baseRef: 'main',
      currentRunId: 200,
      runs: [
        {
          id: 150,
          head_branch: 'main',
          conclusion: 'failure',
          artifacts: [],
        },
      ],
    }),
    null,
  );
});

test('resolveBaselineRun keeps scanning candidates until it finds a usable artifact', async () => {
  const runs = Array.from({ length: 30 }, (_, index) => ({
    id: 500 - index,
    head_branch: 'main',
    conclusion: 'success',
  }));
  const seen = [];

  const run = await resolveBaselineRun({
    baseRef: 'main',
    currentRunId: 999,
    runs,
    hydrateArtifacts: async (candidate) => {
      seen.push(candidate.id);
      return candidate.id === 471 ? ['mobench-history-v1'] : [];
    },
  });

  assert.equal(run.id, 471);
  assert.equal(seen.at(-1), 471);
  assert.equal(seen.length, 30);
});
