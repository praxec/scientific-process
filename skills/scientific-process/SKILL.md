---
name: scientific-process
description: Use when evidence-driven work should be recorded as hypotheses, experiments, observations and conclusions, or when the user wants the computed standing and a structured report. Covers the session lifecycle and session_id with SCIENTIFIC_PROCESS_STATE_DIR, the append variants add_hypothesis/add_experiment/add_observation/conclude, state.get, experiment.next, report.export, and the stateless adjudicate batch falsifier.
---

# scientific-process: hypothesis, experiment, observation, conclusion

The server keeps an append-only event log per session. Nothing is stored as a
mutable "status" — the standing is a pure fold over the log, so replay always
reproduces the same result. Every claim you want to keep must be an event.

## Tool surface

| Tool | Purpose |
|---|---|
| `session.open` | Open or resume a session by `session_id`; replays its log |
| `append` | Append one event, selected by `variant` |
| `state.get` | Return the computed standing (hypotheses, experiments, observations) |
| `experiment.next` | Return the highest-leverage open hypothesis (fewest observations), or null |
| `report.export` | Export each hypothesis with its experiments, observations and verdict |
| `adjudicate` | Stateless batch falsification; does not touch the live session |

`append` variants:

- `add_hypothesis` — `id`, `statement`
- `add_experiment` — `id`, `hypothesis_id`, `design`, `prediction`
- `add_observation` — `id`, `experiment_id`, `result`, `supports` (bool), `evidence`
- `conclude` — `hypothesis_id`, `verdict` (`Supported`, `Refuted`, `Inconclusive`), `rationale`

Duplicate ids are rejected with `DUPLICATE_ID`; references to unknown ids with
`HYPOTHESIS_NOT_FOUND` or `EXPERIMENT_NOT_FOUND`. A rejected event writes
nothing: the log is validated before it is appended.

## Calling the tools

The tools are the same whichever way you reach them; the method names and
arguments in this skill do not change.

- **Registered MCP server.** When `scientific-process` is registered with your agent
  host, call the tools directly (`state.get session_id=<id>` with its arguments).
- **Through a host wrapper.** When the server is not registered, a wrapper can
  spawn the binary per call. With Junior, `junior tools call scientific-process state.get session_id=<id>
  key=value ...` takes inline arguments (values parse as JSON),
  `echo '{"tool":"state.get","arguments":{"session_id":"<id>"}}' | junior tools call scientific-process -`
  reads the request from stdin, and `junior tools call scientific-process request.json`
  reads it from a file. Each call prints the bare result.

Do not register a server just to follow this skill; use whichever route your
host already has.

## Advisory assist tools (opt-in, never append)

When `SCIENTIFIC_PROCESS_ASSIST` includes `jev` and a Jev key is configured
(`TYPESAFE_JEV_API_KEY` or `OPENROUTER_API_KEY`), four advisory tools ask Jev
for a second opinion. They only read the session and always return proposals —
they never append events and never change the standing:

| Tool | Question |
|---|---|
| `assist.experiment_check` | Is the design falsifiable, and the prediction specific and measurable? |
| `assist.observation_support` | P(supports / refutes / irrelevant) for an observation, flagging disagreement with `supports` |
| `assist.verdict_check` | Is the verdict backed by the observations? Warnings plus a confidence |
| `assist.hypothesis_check` | Is the statement testable? Near-duplicates of existing hypotheses |

Each response includes the exact question text sent and per-option
probabilities. With no key every assist tool returns `ASSIST_UNAVAILABLE` and
the core tools continue to work unchanged. Tune the timeout with
`SCIENTIFIC_PROCESS_JEV_TIMEOUT_SECS` (seconds, 1–300).

## Session lifecycle

1. Call `session.open` with a `session_id` before any `append` if you want an
   explicit session. A new id starts fresh; an existing id replays its JSONL log
   and resumes. If you omit `session_id`, the server uses `default`.
   Alternatively, `append`, `state.get`, `experiment.next` and `report.export`
   accept an optional `session_id` that opens/replays that session for the call,
   so per-call clients need no separate `session.open`.
2. Append events with `append`. There is no edit or delete; correct a mistake by
   appending a new event (a later `conclude` overrides an earlier one).
3. Read `state.get` for the standing and `experiment.next` for the next best
   experiment.
4. Call `session.open` again with the same id to resume after a restart.

State lives in one JSONL file per session under `SCIENTIFIC_PROCESS_STATE_DIR`
(falling back to `$XDG_DATA_HOME/scientific-process`, then
`~/.local/share/scientific-process`, then `./scientific-process-state`). Point
the variable at a scratch directory when you do not want to pollute the default.

## When to use

- Any debugging, performance, or design question that needs evidence before a
  claim is accepted: use `sp-investigate`.
- A batch of claims or use cases that must be falsified in one pass: use
  `sp-adjudicate`.
- Recording a single conclusion without a full loop: `append` directly.

Never conclude `Supported` without at least one observation with `supports:
true`, and never flip a `supports` flag to change a verdict — the verdict is for
you to state, the observations are the evidence.
