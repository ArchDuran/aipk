# Use cases

Each doc below is a self-contained problem → solution → worked example, grounded in the actual CLI (see [SPEC.md](../../SPEC.md) and the main [README.md](../../README.md) for the full reference).

| Case | Problem | Doc |
|---|---|---|
| Packaging | Sharing a "smart agent" means sharing a complex, multi-part setup | [single-file-packaging.md](single-file-packaging.md) |
| Retrieval | RAG pipelines need a separate vector database to provision and sync | [embedded-vectors.md](embedded-vectors.md) |
| Audit | No standard way to check what an LLM actually said against your sources | [provenance-verify.md](provenance-verify.md) |
| Hallucination | LLMs assert things even the right context never supported | [strict-render.md](strict-render.md) |
| Governance | Claims used to answer with live in a black box, with no accountability | [claim-lifecycle.md](claim-lifecycle.md) |
