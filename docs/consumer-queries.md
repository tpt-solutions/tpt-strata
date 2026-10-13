# Consumer queries

**Status: awaiting responses.** This file is the demand gate for the deferred
SQL and type work in [v1.1-scope.md](v1.1-scope.md). Each feature that is still
deferred names the consumer evidence it needs. Until a consumer lists a query,
those features stay deferred.

## What to send each consumer

Ask each team for the SQL they plan to run against tpt-strata in their first
release. Give them the supported subset in [the SQL support matrix in the README](../README.md#quick-start),
and ask them to mark anything they would need beyond it. Also ask for:

- the column types involved, especially dates, timestamps, decimals and
  unsigned integers, because those types are gated on this evidence;
- the approximate row count of the largest table, because that decides whether
  streaming matters (see the Parquet row-group note in [todo.md](../todo.md));
- whether they need `CAST`, arithmetic in `SELECT`, or `COUNT(DISTINCT ...)`.

## Responses

| Consumer | Contact | Queries received | Constructs they need beyond v1.1 | Status |
|---|---|---|---|---|
| tpt-keystone-db | | | | Awaiting |
| tpt-aion | | | | Awaiting |
| tpt-cloud-observability | | | | Awaiting |

## How to use the responses

1. Paste each query into the table in this file, or link to it.
2. Run it through `SqlContext::check`. A query that checks clean needs nothing.
3. For a query that is rejected, the diagnostic names the construct. Record it
   in the table above and in the matching row of [v1.1-scope.md](v1.1-scope.md).
4. A construct needed by a consumer graduates to a normal feature: parser,
   binder, engine and test. Date and Timestamp need a real column before work
   starts, per the blast-radius list in todo.md Phase 11c.
