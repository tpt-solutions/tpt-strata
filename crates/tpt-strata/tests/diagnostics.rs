use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, QueryBuilder, Scalar, Table};

fn table() -> Table {
    Table::try_new(vec![
        Column::new(
            "department",
            vec![Scalar::Str("eng".into()), Scalar::Str("sales".into())],
        ),
        Column::new("salary", vec![Scalar::F64(100000.0), Scalar::F64(80000.0)]),
        Column::new("years", vec![Scalar::I32(5), Scalar::I32(3)]),
    ])
    .unwrap()
}

fn ctx() -> SqlContext<'static> {
    let mut c = SqlContext::new();
    c.add_table("employees", Box::leak(Box::new(table())));
    c
}

// === Missing column ===

#[test]
fn snapshot_missing_column_builder() {
    let err = QueryBuilder::new(&table())
        .filter("nonexistent", ">", Scalar::F64(0.0))
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "No column named 'nonexistent'. Available columns: department, salary, years"
    );
}

#[test]
fn snapshot_missing_column_sql() {
    let err = ctx().run("SELECT nonexistent FROM employees").unwrap_err();
    assert_eq!(
        err.to_string(),
        "No column named 'nonexistent'. Available columns: department, salary, years"
    );
}

// === Type mismatch ===

#[test]
fn snapshot_type_mismatch_builder() {
    let err = QueryBuilder::new(&table())
        .filter("department", ">", Scalar::I32(42))
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "Type mismatch in 'department': expected str, found i32. Cannot compare 'department' (str) with a value of i32 type"
    );
}

#[test]
fn snapshot_type_mismatch_sql() {
    let err = ctx()
        .run("SELECT department FROM employees WHERE salary = 'abc'")
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "Type mismatch in 'salary': expected f64, found str. Cannot compare 'salary' (f64) with a literal of str type"
    );
}

// === Unsupported cast ===

#[test]
fn snapshot_unsupported_cast_sql() {
    let err = ctx()
        .run("SELECT CAST(salary AS STRING) FROM employees")
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "SQL error: unsupported cast: tpt-strata v1 has no CAST support; provide values in the target type directly at line 1, column 13\n  SELECT CAST(salary AS STRING) FROM employees\n              ^"
    );
}

// === Malformed SQL ===

#[test]
fn snapshot_malformed_sql_missing_keyword() {
    let err = ctx().run("SELECT * employees").unwrap_err();
    assert_eq!(
        err.to_string(),
        "SQL error: expected 'FROM' but found 'employees' at line 1, column 10\n  SELECT * employees\n           ^"
    );
}

#[test]
fn snapshot_malformed_sql_bad_literal() {
    let err = ctx()
        .run("SELECT department FROM employees WHERE salary = =")
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "SQL error: expected a literal value but found '=' at line 1, column 50\n  SELECT department FROM employees WHERE salary = =\n                                                   ^"
    );
}

#[test]
fn snapshot_malformed_sql_trailing() {
    let err = ctx().run("SELECT * FROM employees ORDER BY").unwrap_err();
    assert_eq!(
        err.to_string(),
        "SQL error: expected a column name in ORDER BY but found 'end of input' at line 1, column 33\n  SELECT * FROM employees ORDER BY\n                                  ^"
    );
}

// === Unsupported construct ===

#[test]
fn snapshot_unsupported_function_sql() {
    let err = ctx()
        .run("SELECT MEDIAN(salary) FROM employees")
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "SQL error: unsupported function 'MEDIAN' in SELECT; supported: SUM, COUNT, MIN, MAX, AVG at line 1, column 23\n  SELECT MEDIAN(salary) FROM employees\n                        ^"
    );
}

#[test]
fn snapshot_group_by_validation() {
    let err = ctx()
        .run("SELECT department FROM employees GROUP BY years")
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "Unsupported operation: column 'department' must appear in the GROUP BY clause or be used in an aggregate function"
    );
}

// === Deliberate v1.1 scope gaps (targeted diagnostics) ===
//
// These features are *decided* omissions (docs/v1.1-scope.md), so each is
// rejected with a message naming the construct and pointing at the decision,
// rather than a generic parse error. The wording prefix is pinned below.

#[test]
fn or_and_parentheses_in_where_are_supported() {
    // The fixture has two rows: eng (salary 100000, years 5) and sales (80000, 3).
    let out = ctx()
        .run("SELECT department FROM employees WHERE salary > 1 OR years < 2")
        .unwrap();
    assert_eq!(out.row_count(), 2);
    let out = ctx()
        .run("SELECT department FROM employees WHERE (salary > 1 OR years < 2) AND department = 'eng'")
        .unwrap();
    assert_eq!(out.row_count(), 1);
}

#[test]
fn in_list_in_where_is_supported() {
    let out = ctx()
        .run("SELECT department FROM employees WHERE department IN ('eng', 'hr')")
        .unwrap();
    assert_eq!(out.row_count(), 1);
}

#[test]
fn column_to_column_comparison_is_supported() {
    let out = ctx()
        .run("SELECT department FROM employees WHERE salary > years")
        .unwrap();
    assert_eq!(out.row_count(), 2);
}

#[test]
fn comparing_a_text_column_with_a_number_column_names_both() {
    let err = ctx()
        .run("SELECT department FROM employees WHERE department = years")
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("cannot compare 'department' (str) with 'years' (i32)"),
        "{err}"
    );
}

#[test]
fn is_null_between_and_like_in_where_are_supported() {
    // Shipped in the v1.1 subset: these must run, not be rejected.
    let c = ctx();
    let nulls = c
        .run("SELECT * FROM employees WHERE department IS NOT NULL")
        .unwrap();
    assert!(nulls.row_count() <= 4);
    c.run("SELECT * FROM employees WHERE years BETWEEN 1 AND 2")
        .unwrap();
    c.run("SELECT * FROM employees WHERE department LIKE 'e%'")
        .unwrap();
}

#[test]
fn distinct_is_supported_as_group_by() {
    let out = ctx()
        .run("SELECT DISTINCT department FROM employees")
        .unwrap();
    assert_eq!(out.row_count(), 2);
}

#[test]
fn having_filters_aggregates() {
    // Each department has one row, so COUNT(*) is 1 for both groups.
    let out = ctx()
        .run("SELECT department FROM employees GROUP BY department HAVING COUNT(*) >= 1")
        .unwrap();
    assert_eq!(out.row_count(), 2);
    let having_none = ctx()
        .run("SELECT department FROM employees GROUP BY department HAVING COUNT(*) > 1")
        .unwrap();
    assert_eq!(having_none.row_count(), 0);
}

#[test]
fn cast_is_still_rejected_with_a_targeted_message() {
    let err = ctx()
        .run("SELECT CAST(salary AS TEXT) FROM employees")
        .unwrap_err();
    assert!(
        err.to_string().contains("unsupported cast"),
        "unexpected diagnostic: {err}"
    );
}
