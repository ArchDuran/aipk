# An explicit lifecycle for claims

> Claims living in a black box make it impossible to tell what's actually authoritative. AIPK gives every claim an explicit lifecycle: extracted → reviewed → canonical → deprecated.

## The problem

Once "facts" are extracted from source documents, something has to decide which ones are trustworthy enough to answer with, and that decision needs a record — who approved it, when, and why — or it's just as opaque as the black box it replaced.

## How AIPK solves it

Every fact extracted into the `CLMS` section starts life with status `extracted`. A human reviewer moves it forward or out with explicit commands, and only claims that reach `canonical` are usable in the strict modes ([strict-render.md](strict-render.md)); `deprecated` claims are filtered out at load time and never reach retrieval or rendering.

```
extracted ──► reviewed ──► canonical ──────────────► deprecated
```

| Action | Command | Result |
|---|---|---|
| Extract from a source | `aipk extract-claims regulations.pdf --dir ./legal-assistant` | new claims, status `extracted` |
| Human reviewed it | `aipk claims promote <id> --status reviewed` | `reviewed` + audit entry |
| Confirmed authoritative | `aipk claims promote <id>` | `canonical` + audit entry |
| Found invalid | `aipk claims reject <id> --reason "..."` | `deprecated` + audit entry |

## Walkthrough

```bash
# extract atomic claims from source documents
aipk extract-claims regulations.pdf contracts.pdf \
  --model llama3.2 --dir ./legal-assistant

# see what's pending
aipk claims list --status extracted
aipk claims stats

# review interactively (y/n/s/q per claim)
aipk claims review

# or promote one explicitly, with a reason on record
aipk claims promote regs_0012 --reason "verified against source paragraph 3.2"

# reject a claim that turned out to be wrong
aipk claims reject regs_0013 --reason "superseded by 2026 amendment"
```

Every transition is appended to an `audit[]` log on the claim itself. The CLI only ever appends to it — there's no command to edit or delete an entry:

```json
{
  "action": "promote",
  "from": "extracted",
  "to": "canonical",
  "reviewer": "dr-smith",
  "reason": "verified against source paragraph 3.2",
  "model": "llama3.2",
  "at": "2026-05-02T10:00:00Z"
}
```

The log only ever grows through the CLI, so the history of who approved what, and why, survives as long as the package does. This is append-only by convention, not by cryptographic guarantee: `aipk extract` unpacks a package back into an editable project directory, so anyone with the file can hand-edit the claims JSON and `aipk build` a new package from it. To make tampering detectable rather than just inconvenient, `aipk seal` the package with a publisher key — that gives you a verifiable signature to check on install, not the JSON `audit[]` log itself. `aipk claims check-conflicts` can also scan `canonical` claims pairwise for contradictions using the LLM, catching the case where two authoritative claims quietly disagree.

## See also

- [README.md — Extract claims](../../README.md#extract-claims)
- [SPEC.md — Claim lifecycle](../../SPEC.md)
- [provenance-verify.md](provenance-verify.md)
- [strict-render.md](strict-render.md)
