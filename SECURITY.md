# Security Policy

## Supported versions

AIPK is pre-1.0 and has no versioned releases yet — there is a single line of
development on `master`. Security fixes land there; there is no older branch
to backport to.

## Reporting a vulnerability

Email **support@aipk.dev** with a description of the issue and, if
possible, steps to reproduce. Please do not open a public issue for anything
that isn't already public knowledge.

You should get an initial response within 72 hours. There is no bug bounty at
this stage.

## Threat model and known limitations

AIPK can sign and encrypt `.aipk` packages, but it does not sandbox what a
package can do once you run it:

- **Package signatures are optional, except for sealed packages.** A
  package *may* be signed with Ed25519; `aipk verify` reports whether a
  signature is present and valid, but an ordinary unsigned package still
  runs — treat it as untrusted input. A **sealed** package (`aipk seal`)
  is different: it always carries a mandatory Ed25519 signature, and the
  runtime refuses to load it if that signature is missing or invalid.
  Either way, a valid signature only proves the bytes match what some key
  signed (integrity) — it says nothing about whether that key's owner, or
  the knowledge/tools inside, are trustworthy.
- **Passphrase encryption** (`aipk encrypt`) protects package content with
  AES-256-GCM, with the key derived from a user-supplied passphrase via
  Argon2id (OWASP-recommended parameters) and a random per-package salt.
  Passphrase strength is the user's responsibility.
- **Sealing** (`aipk seal`) is a different mechanism from encryption: it
  produces an opaque, author-controlled package using a key derived from a
  random salt embedded in the package itself — not a human passphrase.
  Sealing is meant for controlled distribution (like model weights), not
  for protecting a file from someone who holds the whole runtime; it is
  not DRM and won't stop a determined attacker who can inspect or modify
  the open-source runtime.
- **MCP tools run unsandboxed.** A package can declare MCP tools that execute
  external commands on your machine. AIPK shows you what a tool is about to
  run and asks for confirmation before the first use — but once approved, the
  tool can do anything your user account can do. There is no seccomp/container
  sandbox yet.

**Practical takeaway:** treat an unfamiliar `.aipk` file the way you would an
email attachment or a shell script from someone you don't already trust — the
signature tells you who signed it, not whether you should run it.

## Dependencies

`cargo audit` runs in CI on every push against the RustSec advisory database,
and a CycloneDX SBOM is generated on every build. Known high-severity
advisories in direct or transitive dependencies are treated as release
blockers.
