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
fn snapshot_v1_1_or_in_where() {
    let err = ctx()
        .run("SELECT * FROM employees WHERE salary > 1 OR years < 2")
        .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("'OR in WHERE' is not supported in tpt-strata v1.1")
            && text.contains("combine predicates with AND only")
            && text.contains("docs/v1.1-scope.md"),
        "unexpected diagnostic: {text}"
    );
}

#[test]
fn snapshot_v1_1_is_null_in_where() {
    let err = ctx()
        .run("SELECT * FROM employees WHERE department IS NOT NULL")
        .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("'IS NULL / IS NOT NULL' is not supported")
            && text.contains("sentinel value"),
        "unexpected diagnostic: {text}"
    );
}

#[test]
fn snapshot_v1_1_in_between_like_in_where() {
    for sql in [
        "SELECT * FROM employees WHERE years IN (1, 2)",
        "SELECT * FROM employees WHERE years BETWEEN 1 AND 2",
        "SELECT * FROM employees WHERE department LIKE 'e%'",
    ] {
        let err = ctx().run(sql).unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("is not supported in tpt-strata v1.1")
                && text.contains("docs/v1.1-scope.md"),
            "unexpected diagnostic for {sql}: {text}"
        );
    }
}

#[test]
fn snapshot_v1_1_parens_in_where() {
    let err = ctx()
        .run("SELECT * FROM employees WHERE (years = 1 OR salary > 0)")
        .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("parenthesized expressions / operator precedence")
            && text.contains("docs/v1.1-scope.md"),
        "unexpected diagnostic: {text}"
    );
}

#[test]
fn snapshot_v1_1_column_to_column_comparison() {
    let err = ctx()
        .run("SELECT * FROM employees WHERE salary = years")
        .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("column-to-column comparisons are not supported")
            && text.contains("compare 'salary' against a literal instead"),
        "unexpected diagnostic: {text}"
    );
}

#[test]
fn snapshot_v1_1_distinct() {
    let err = ctx()
        .run("SELECT DISTINCT department FROM employees")
        .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("'DISTINCT' is not supported") && text.contains("GROUP BY"),
        "unexpected diagnostic: {text}"
    );
}

#[test]
fn snapshot_v1_1_having() {
    let err = ctx()
        .run("SELECT department FROM employees GROUP BY department HAVING COUNT(*)> 1")
        .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("'HAVING' is not supported") && text.contains("nest two queries"),
        "unexpected diagnostic: {text}"
    );
}

#[test]
fn snapshot_v1_1_left_and_multiple_joins() {
    for sql in [
        "SELECT * FROM employees LEFT JOIN t ON employees.department = t.name",
        "SELECT * FROM employees JOIN t ON employees.department = t.name JOIN t2 ON t.name = t2.name",
    ] {
        let err = ctx().run(sql).unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("is not supported in tpt-strata v1.1") && text.contains("equi-join"),
            "unexpected diagnostic for {sql}: {text}"
        );
    }
}
