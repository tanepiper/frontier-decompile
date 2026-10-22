# Failure path

Every team ends with `re-verifier`. When it finds a difference, the team stops before
landing and goes back to the agent that can explain the difference, then ports and verifies again. This
page shows where a failure goes.

```mermaid
flowchart TD
    V[re-verifier: a check fails] --> K{What differs?}
    K -->|a value or a picture| T[re-tracer]
    K -->|random draw order| R[re-randomiser]
    K -->|the last bit| M[re-measurer]
    K -->|a file's bytes| D[re-decoder]
    K -->|a run differs from itself| L[re-listener]
    T --> P[re-porter: fix the port]
    R --> P
    M --> P
    D --> P
    L --> P
    P --> V2[re-verifier: check again]
    V2 -->|identical| Land[re-lander]
    V2 -->|still differs| K
```

| What differs | Agent | What it does |
|---|---|---|
| A value or a picture | `re-tracer` | Re-reads the function at the first difference |
| The order of random draws | `re-randomiser` | Finds the extra or missing draw |
| Arithmetic in the last bit | `re-measurer` | Measures the step that differs |
| A file's bytes | `re-decoder` | Finds the field that differs |
| A run differs from itself | `re-listener` | Finds the input that is not in the record |

## What the verifier hands on

A failing hand-off names the first difference:

```text
## Hand-off
Stage: re-verifier
Done: compared 600 values; 412 identical
Evidence: derived/verify/terrain-diff.txt
Open: first difference at input 413, field height, 0.5 ULP
Next: re-measurer - measure FUN_140041a20 at input 413, step by step
```

## Rules for the main session

- Send the agent the table above points to, with the first difference in its brief.
- Three rounds on the same difference without progress is a signal to stop. Write down what
  was tried in `docs/CHECKLIST.md` and ask the user.
- A run that differs from itself points to something outside the input record that reaches
  the rules: a clock, an uninitialised value, iteration over a hash map. Send `re-listener`.
- Never land a slice with a known difference unless the user agrees to record it as a
  deviation.
