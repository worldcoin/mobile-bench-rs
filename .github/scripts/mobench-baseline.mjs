function isBaselineCandidate({ baseRef, currentRunId, run }) {
  return (
    run.id !== currentRunId &&
    run.head_branch === baseRef &&
    run.conclusion === 'success'
  );
}

function hasHistoryArtifact(artifacts) {
  return artifacts.includes('mobench-history-v1');
}

function sortNewestFirst(a, b) {
  return b.id - a.id;
}

export function selectBaselineRun({ baseRef, currentRunId, runs }) {
  return (
    runs
      .filter((run) => isBaselineCandidate({ baseRef, currentRunId, run }))
      .filter((run) => hasHistoryArtifact(run.artifacts ?? []))
      .sort(sortNewestFirst)[0] ?? null
  );
}

export async function resolveBaselineRun({
  baseRef,
  currentRunId,
  runs,
  hydrateArtifacts,
}) {
  for (const run of runs
    .filter((candidate) =>
      isBaselineCandidate({ baseRef, currentRunId, run: candidate }),
    )
    .sort(sortNewestFirst)) {
    const artifacts = run.artifacts ?? (await hydrateArtifacts(run));
    if (hasHistoryArtifact(artifacts)) {
      return {
        ...run,
        artifacts,
      };
    }
  }

  return null;
}
