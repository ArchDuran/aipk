# Stop hallucination with strict-render

> LLMs hallucinate even with good context. Strict-render mode prompts the model to cite a verified claim after every factual sentence, then reports exactly which sentences weren't grounded.

## The problem

Retrieval doesn't stop a model from asserting things the retrieved context never said — a model can have the right chunks in front of it and still confidently state something none of them support. Checking that after the fact (see [provenance-verify.md](provenance-verify.md)) tells you it happened; it doesn't prevent it during generation.

## How AIPK solves it

`aipk serve --strict-render` changes how the model is prompted: it's instructed to attach a `[claim_id]` to every factual sentence, citing only claims that reached `canonical` status (see [claim-lifecycle.md](claim-lifecycle.md)). The response is **not blocked or regenerated** if the model ignores this — strict-render is a prompting + reporting mechanism, not a hard gate. After generation, a sentence-to-claim check scores the response and attaches the result as `_aipk` metadata, so the caller decides what to do with an ungrounded answer (reject it, flag it, re-ask). Neither strict-render nor `--strict-verify` below currently enforces a hard refusal in the runtime — both are prompt constraints the model can be scored against, not gates that block a response before it's sent. If your workflow needs a hard refusal, check `_aipk.fully_grounded` (or `aipk verify`'s exit code — see [provenance-verify.md](provenance-verify.md)) on the caller side and reject there.

## Walkthrough

```bash
aipk serve legal-assistant.aipk --strict-render --model llama3.2
```

Every response carries an `_aipk` metadata block reporting exactly what was and wasn't grounded:

```json
{
  "_aipk": {
    "mode": "strict-render",
    "coverage": 0.87,
    "canonical_claims_used": 4,
    "uncited_sentences": 1,
    "invalid_claim_ids": [],
    "unsupported_sentences": ["This sentence had no citation."],
    "fully_grounded": false
  }
}
```

`fully_grounded: false` and the `unsupported_sentences` list mean the answer had a sentence the model produced without backing it against the canonical knowledge base — visible immediately, not discovered later in an audit.

A related mode, `--strict-verify`, constrains the model to answer only from canonical claims and to say it has no verified information if none match — again a prompt constraint the model can ignore, not an enforced refusal, and it falls back to general-knowledge answering if the package has no canonical claims at all:

```bash
aipk serve legal-assistant.aipk --strict-verify --model llama3.2
```

## See also

- [README.md — Strict serving modes](../../README.md#strict-serving-modes)
- [provenance-verify.md](provenance-verify.md)
- [claim-lifecycle.md](claim-lifecycle.md)
