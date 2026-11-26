# Release qualification: idle soak

`tools/idle_soak.ps1` is the Stable-release idle evidence collector. It samples an already running Sidenote process tree; it does not launch the app. Launching a native synthetic desktop fixture is a separate release task, and the existing 30-second headless CPU proof is not a substitute for this run.

A failed run blocks the Stable release. Do not use an application restart as a workaround.

## Required isolation

Run on Windows in a dedicated synthetic environment with no live Hex process. The script requires all of the following before it samples anything:

- `-SyntheticDataRoot` is an explicit absolute path and `-ApproveSyntheticDataRoot` is present.
- The root is a normal directory rather than a drive root, is not a reparse point, and is either absent or empty. A nonempty directory is refused.
- The root does not overlap Sidenote or Hex application-data locations, and is not exactly a live `SIDENOTE_*_DIR` override.
- `-CandidateExePath` names the exact candidate executable. Standard install roots, including the Velopack `%LOCALAPPDATA%\Sidenote` root, are refused so an installed copy cannot be mistaken for the candidate.
- The running root process executable must match the candidate path. The report records the candidate SHA-256.
- No process named `hex.exe` or `hex.com` is present.

The script never starts, stops, or changes Sidenote or Hex. It only enumerates processes and reads process counters.

## Command shape

```powershell
pwsh -File tools/idle_soak.ps1 `
  -RootProcessId <pid> `
  -SyntheticDataRoot <absolute-empty-synthetic-root> `
  -ApproveSyntheticDataRoot `
  -CandidateExePath <candidate-sidenote.exe>
```

Default collection is 24 hours after a 30-minute warm-up. CPU is sampled every 60 seconds; working set, private bytes, and handle count are sampled every 3,600 seconds. The default timeout is the requested duration plus 15 minutes. CPU time uses cumulative kernel plus user time summed across the whole process tree, including the last observed contribution of children that exit. Elapsed time comes from a monotonic stopwatch rather than wall-clock timestamps.

## Pass bar

The constants are in `tools/idle_soak.ps1` and copied into each report. A run passes only when all are true:

| Check | Limit |
|---|---|
| Post-warm-up average CPU | at most 1% of one core; 100% means one fully occupied core |
| Missing CPU or resource samples | at most 5% of that series, and no more than 2 consecutive missing samples |
| Working-set upward trend | least-squares slope at most 4 MiB/hour when at least 60% of hourly steps are nonnegative |
| Private-bytes upward trend | least-squares slope at most 4 MiB/hour under the same step rule |
| Handle-count upward trend | least-squares slope at most 4 handles/hour under the same step rule |
| Post-warm-up final growth | at most 64 MiB working set, 64 MiB private bytes, or 64 handles |
| Completion | no crash, no timeout, enough post-warm-up samples, and CPU coverage through the requested duration |

The 60% nonnegative-step rule keeps one noisy decrease from hiding a steady upward series while still requiring sustained direction in addition to slope. Final growth is checked independently, so a large endpoint increase fails even when the slope test does not classify it as steady.

## Reports

The synthetic root receives `idle-soak-report.json` and `idle-soak-report.md`. The JSON report contains the result, failure reasons, thresholds, configuration, candidate path and SHA-256, missing-sample counts, CPU average, trend slopes, final deltas, positive-step fractions, and the raw CPU/resource series. `sample_source` is `process-tree` for a real collection.

`tools/idle_soak_proof.ps1` uses checked-in-code paths plus generated fake sample files only. It does not launch Sidenote and does not require Hex. Its fixture reports have `sample_source` of `fixture`; they prove script behavior and are never release evidence. Run it from the repository root:

```powershell
pwsh -File tools/idle_soak_proof.ps1
```
