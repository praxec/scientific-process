---
name: sp-adjudicate
description: Use when a batch of independent claims, use cases or design checks must be falsified in one pass, or when an adequacy decision needs a computed verdict instead of an asserted one. Covers supplying one supports observation per case and reading the computed adequate, refuted and report response of the adjudicate tool.
---

# sp-adjudicate: batch falsification

`adjudicate` is a stateless batch falsifier. It does not touch the live session.
Each case is one claim; you supply the observation, the tool computes the verdict.

## Call it

```json
{
  "cases": [
    {
      "use_case_id": "large-upload",
      "supports": false,
      "evidence": "the request body parser caps at 1 MiB",
      "rationale": "the design cannot represent a 5 MiB upload"
    },
    {
      "use_case_id": "offline-edit",
      "supports": true,
      "evidence": "the local queue replays edits on reconnect"
    }
  ]
}
```

For each case the tool folds an adequacy hypothesis, a refutation experiment, one
observation carrying your `supports` flag, and a conclusion (`Refuted` iff
`supports` is false). It returns:

- `adequate` — `true` iff no case was refuted;
- `refuted` — the `use_case_id`s that were refuted;
- `report` — the full per-case standing.

## Rules

- **One case per claim.** Do not merge unrelated claims; the verdict is per case
  and the `refuted` list is what the caller acts on.
- **Supply only the observation.** The caller provides `supports`, `evidence` and
  optional `rationale`; the tool derives the verdict. Never pre-compute the
  verdict for it.
- **`supports: false` means the case is refuted.** Treat each refutation as a
  concrete gap to fix, and re-run the batch after the fix.
- **`adequate: true` is not proof.** It means no supplied case failed, so the
  batch is only as strong as the cases you chose.

## Use it for

- Checking a design against a list of use cases at once.
- Gating an artifact on a set of falsification checks.
- Reproducing a decision where "nothing failed" must be visible as evidence.

For investigating a single uncertain question, use `sp-investigate`.
