# Security Policy

## Supported Versions

Only the latest release receives security fixes.

| Version | Supported |
|---|---|
| latest | :white_check_mark: |
| older | :x: |

## Reporting a Vulnerability

Please report suspected security vulnerabilities privately to the project
maintainers before disclosing them publicly.

- Email the maintainers directly, with "SECURITY" in the subject line.
- Do **not** open a public issue for security findings.
- Include a minimal reproducer and, if possible, a suggested fix.

You should receive an acknowledgment within 3 business days. Depending on the
severity, we will coordinate a disclosure timeline with you before a public
fix is released.

## Scope

This covers the tpt-strata workspace under the following directory:

```
crates/
```

In particular, note the core-engine design constraint: `tpt-strata` ships with
zero external dependencies, so its supply-chain surface is intentionally small.
Review required dependencies only when touching `tpt-strata-parquet`.