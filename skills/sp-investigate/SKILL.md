---
name: sp-investigate
description: Use when debugging a failure, explaining a performance problem, or answering any question that needs evidence before a claim is accepted. Covers the hypothesis to experiment to observation to conclusion loop, one bounded experiment per step, recording evidence with each observation, and refusing to conclude Supported without supporting observations.
---

# sp-investigate: run the loop

Calling the tools: see [Calling the tools](../scientific-process/SKILL.md#calling-the-tools)
(registered MCP server or a host wrapper such as `junior tools call`).

Investigate by the scientific method. State a hypothesis, design the smallest
experiment that could refute it, record the observation, and only then conclude.
Each step is one bounded experiment, not a sweep of the codebase.

## The loop

1. **Hypothesis** — `append {variant: add_hypothesis, id, statement}`. Write a
   falsifiable statement, not a goal. Prefer one hypothesis per round.
2. **Experiment** — `append {variant: add_experiment, id, hypothesis_id, design,
   prediction}`. `prediction` is what you expect to observe if the hypothesis is
   true. Keep the experiment bounded: one change, one measurement, one command.
3. **Observation** — `append {variant: add_observation, id, experiment_id,
   result, supports: bool, evidence}`. `result` is what happened, `supports` says
   whether it matched `prediction`, and `evidence` is the raw material (command,
   output, file, numbers) a reviewer could check. One experiment per bounded
   step: do not bundle several independent changes into one observation.
4. **Conclusion** — `append {variant: conclude, hypothesis_id, verdict,
   rationale}` with `Supported`, `Refuted` or `Inconclusive`.

Then call `experiment.next` to get the open hypothesis with the fewest
observations. That is the highest-leverage next experiment.

## Rules

- **Never conclude `Supported` without supporting observations.** At least one
  observation for the hypothesis must have `supports: true`. If the evidence is
  missing, stop at `Inconclusive` or design another experiment.
- **Record evidence, not opinions.** `evidence` should let someone else rerun
  the check; `rationale` explains the inference from the observations.
- **A refuting observation is a result, not a failure.** Append it with
  `supports: false` and conclude `Refuted` when the prediction clearly failed.
- **One experiment per bounded step.** Split a broad question into hypotheses
  that can each be refuted by one experiment.
- **Resume, do not restart.** Reopen the same `session_id` so the whole chain is
  available to `report.export`.

## Stop when

- the hypothesis is `Supported` with supporting observations, or `Refuted` with a
  visible mismatch, or `Inconclusive` because a decisive experiment is not
  available;
- `report.export` shows every claim paired with the experiment and evidence that
  produced it.

For batches of independent claims, use `sp-adjudicate` instead of looping.
