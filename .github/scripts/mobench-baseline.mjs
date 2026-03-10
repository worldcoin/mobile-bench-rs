export function selectBaselineRun({ baseRef, currentRunId, runs }) {
  return (
    runs
      .filter(
        (run) =>
          run.id !== currentRunId &&
          run.head_branch === baseRef &&
          run.conclusion === 'success' &&
          (run.artifacts ?? []).includes('mobench-history-v1'),
      )
      .sort((a, b) => b.id - a.id)[0] ?? null
  );
}
