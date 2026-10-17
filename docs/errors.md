# Error reference

Every `QueryError` has a stable `kind()` string for code that needs to branch
on the error type, and a message written for the caller. This page explains
each kind, shows a real message, and says what to change. The examples come
from `cargo run -p tpt-strata --example diagnostics`.

| `kind()` | Variant | Usually means |
|---|---|---|
| `missing-column` | `MissingColumn` | A name in the query or builder call is not in the table. |
| `type-mismatch` | `TypeMismatch` | A value or aggregate does not fit the column's type. |
| `unsupported` | `UnsupportedOperation` | The operation is valid SQL but not in this engine, or it breaks a rule such as duplicate output names. |
| `sql` | `Sql` | The SQL text does not parse, or uses a construct outside the supported subset. |
| `io` | `Io` | Reading or writing a file failed. The underlying error is the `source()`. |
| `import` | `Import` | A file was read but its contents could not be turned into a table. |

## `missing-column`

```text
No column named 'nonexistent'. Available columns: department, salary
```

The message names the column you used and lists the columns the table really
has. Check the spelling. Column names are case-sensitive. If the column comes
from a join, qualify it with its table name, for example `employees.salary`.

## `type-mismatch`

```text
Type mismatch in 'salary': expected f64, found str. Cannot compare 'salary' (f64) with a literal of str type
```

A comparison, filter or aggregate used a value whose type does not match the
column. Compare numbers with numbers and strings with quoted strings. An
integer literal is accepted against a float column, and an exact comparison
is used, so `salary > 50000` works. A string literal against a numeric column
does not.

## `unsupported` (`UnsupportedOperation`)

```text
Unsupported operation: aggregate 'SUM(department)' requires a numeric column; column 'department' has type str
```

The operation exists but cannot run on this input. Here `SUM` needs a number.
Other messages in this group report a join whose output would have two
columns with the same name (rename one side, or qualify the columns) and a
self-join, which is not supported yet.

## `sql`

```text
SQL error: unsupported cast: tpt-strata has no CAST support; provide values in the target type directly at line 1, column 13
  SELECT CAST(salary AS TEXT) FROM employees
              ^
```

SQL errors carry the line and column, and a caret under the problem. There are
two cases:

- **A syntax error**, such as `WHERE name =` with nothing after it. Fix the
  SQL text.
- **A construct outside the supported subset**, such as `CAST`, `COUNT(DISTINCT ...)`,
  subqueries, or arithmetic in `SELECT`. The message names the construct and
  says what to do instead. The subset and the reasons for each gap are in
  [scope.md](scope.md).

## `io`

I/O errors come from reading or writing a file, for example a missing CSV or
Parquet path. The error's `source()` is the operating-system error, so a
caller can show it. Check that the path exists and is readable.

## `import`

```text
Import error: <message>
```

The file was opened but its contents do not match what the bridge supports:
an unsupported Parquet column type, or a CSV or JSON value that does not fit
the inferred schema. The message names the column or the record where it
failed. Narrow and unsigned integers and `Float32` are widened to `i32`,
`i64` or `f64` at import. Other types are rejected, and the message says so.

## Branching on the kind

```rust
match ctx.run(sql) {
    Ok(table) => { /* use the result */ }
    Err(e) if e.kind() == "missing-column" => { /* ask the user for another column */ }
    Err(e) => return Err(e),
}
```

Match on `kind()` rather than on the message text, because the wording of a
message may improve over time.
