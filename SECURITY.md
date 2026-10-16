# Security policy

Swarn handles agent approvals and end-to-end encrypted pushes, so we take reports seriously.

## Reporting a vulnerability

Please don't open a public issue. Use GitHub's private reporting instead: go to the **Security** tab
of this repository and click **Report a vulnerability**. Only the maintainers can see the report.

Include what you found, how to reproduce it, and what an attacker could do with it. You'll get an
answer within a week. We'll keep you posted while we work on a fix, and we'll credit you when it
ships unless you'd rather we didn't.

## Supported versions

Nothing has been released yet. Until v0.1 ships, fixes land on `main` only.

## Known design issues

[docs/threat-model.md](docs/threat-model.md) says what Swarn defends against and lists the gaps we
already know about.

Open questions about the security design, including some unresolved ones, are tracked in
[docs/PRD_REVIEW.md](docs/PRD_REVIEW.md). You don't need to report those again, but comments on
them are welcome.
