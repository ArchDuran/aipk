# Audit what the LLM actually said

> There's no standard way to check what an LLM actually said against your sources. `aipk verify` gives you sentence-level provenance.

## The problem

An LLM answer either matches your source documents or it doesn't, and with plain RAG there is no reliable way to tell which sentences are grounded and which one the model quietly invented. That's a hard problem the moment an answer needs to survive a compliance review, a legal audit, or just a colleague asking "where does this come from?"

## How AIPK solves it

AIPK stores knowledge not just as retrievable chunks but as atomic **claims** — the `CLMS` section holds one entry per fact, each with a source document, the exact span it was extracted from, and a lifecycle status (`extracted → reviewed → canonical → deprecated`, see [claim-lifecycle.md](claim-lifecycle.md)). `aipk verify` takes a finished answer and checks every sentence against those claims independently of how the answer was produced.

## Walkthrough

```bash
aipk verify legal-assistant.aipk \
  "Data retention must not exceed 5 years under Article 17." \
  --json
```

```json
{
  "package": "legal-assistant",
  "sentences": [
    {
      "text": "Data retention must not exceed 5 years under Article 17.",
      "verdict": "grounded",
      "claim_id": "regs_0012",
      "claim_text": "Article 17 limits data retention to 5 years.",
      "source": "regulations.pdf",
      "span": "retention period shall not exceed five years",
      "score": 0.71
    }
  ],
  "summary": {
    "total": 1,
    "grounded": 1,
    "coverage": 1.0,
    "unsupported_sentences": []
  }
}
```

Each sentence resolves to a specific `claim_id`, which resolves to a specific source file and the exact span the claim was extracted from — a full chain from generated text back to the document that justifies it.

This is CI-friendly by design. `--min-coverage` turns the check into a pass/fail gate:

```bash
aipk verify legal-assistant.aipk "Answer text..." --min-coverage 0.8 --json
```

Exit codes: `0` — coverage at or above threshold; `1` — below threshold, or unsupported sentences found; `2` — package or runtime error. A CI job can fail a release the moment an answer's grounding drops, the same way it would fail on a broken test.

## See also

- [README.md — Epistemic Mode](../../README.md#epistemic-mode--grounded-auditable-answers)
- [strict-render.md](strict-render.md)
- [claim-lifecycle.md](claim-lifecycle.md)
