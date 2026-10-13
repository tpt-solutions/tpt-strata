//! AND / OR / parentheses, IN lists, and column-to-column comparisons.

use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, Scalar, Table};

fn s(v: &str) -> Scalar {
    Scalar::Str(v.to_string())
}

fn table() -> Table {
    Table::try_new(vec![
        Column::new(
            "id",
            vec![
                Scalar::I64(1),
                Scalar::I64(2),
                Scalar::I64(3),
                Scalar::I64(4),
            ],
        ),
        Column::new(
            "a",
            vec![Scalar::I64(1), Scalar::Null, Scalar::I64(5), Scalar::I64(5)],
        ),
        Column::new(
            "b",
            vec![Scalar::I64(1), Scalar::I64(2), Scalar::Null, Scalar::I64(9)],
        ),
        Column::new("tag", vec![s("x"), s("y"), s("x"), s("z")]),
    ])
    .unwrap()
}

fn ids(sql: &str) -> Vec<i64> {
    let t = table();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx.run(sql).unwrap();
    let mut v = Vec::new();
    for b in out.batches() {
        for r in 0..b.row_count() {
            match b.columns[0].get(r) {
                Some(Scalar::I64(n)) => v.push(n),
                other => panic!("expected an integer id, found {other:?}"),
            }
        }
    }
    v
}

#[test]
fn and_binds_tighter_than_or() {
    // a = 5 OR (tag = 'x' AND id = 1) keeps rows 3, 4 and 1.
    assert_eq!(
        ids("SELECT id FROM t WHERE a = 5 OR tag = 'x' AND id = 1 ORDER BY id"),
        vec![1, 3, 4]
    );
}

#[test]
fn parentheses_override_precedence() {
    // (a = 5 OR tag = 'x') AND id > 2 keeps rows 3 and 4.
    assert_eq!(
        ids("SELECT id FROM t WHERE (a = 5 OR tag = 'x') AND id > 2 ORDER BY id"),
        vec![3, 4]
    );
}

#[test]
fn nested_parentheses() {
    assert_eq!(
        ids("SELECT id FROM t WHERE ((id = 1 OR id = 2) AND (tag = 'x' OR tag = 'y')) ORDER BY id"),
        vec![1, 2]
    );
}

#[test]
fn null_in_an_or_only_matters_when_the_other_side_is_false() {
    // a is NULL for row 2. Row 2 still passes through the other branch.
    assert_eq!(
        ids("SELECT id FROM t WHERE a > 3 OR id = 2 ORDER BY id"),
        vec![2, 3, 4]
    );
    // With the other branch false too, the NULL comparison filters the row out.
    assert_eq!(
        ids("SELECT id FROM t WHERE a > 3 OR id = 9 ORDER BY id"),
        vec![3, 4]
    );
}

#[test]
fn in_list_matches_any_listed_value() {
    assert_eq!(
        ids("SELECT id FROM t WHERE tag IN ('x', 'z') ORDER BY id"),
        vec![1, 3, 4]
    );
    assert_eq!(ids("SELECT id FROM t WHERE id IN (2) ORDER BY id"), vec![2]);
}

#[test]
fn null_in_an_in_list_matches_nothing_for_that_entry() {
    // `a IN (NULL, 5)`: the NULL entry never matches, the 5 does.
    assert_eq!(
        ids("SELECT id FROM t WHERE a IN (NULL, 5) ORDER BY id"),
        vec![3, 4]
    );
}

#[test]
fn column_to_column_comparison_with_nulls() {
    // Rows where both a and b are present and a < b: row 1 (1 < 1 is false), row 4 (5 < 9).
    assert_eq!(ids("SELECT id FROM t WHERE a < b ORDER BY id"), vec![4]);
    // a = b on row 1 only.
    assert_eq!(ids("SELECT id FROM t WHERE a = b ORDER BY id"), vec![1]);
}

#[test]
fn column_to_column_mixed_numeric_types_compare_exactly() {
    let t = Table::try_new(vec![
        Column::new("i", vec![Scalar::I64(2), Scalar::I64(3)]),
        Column::new("f", vec![Scalar::F64(2.5), Scalar::F64(3.0)]),
    ])
    .unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    assert_eq!(
        ctx.run("SELECT i FROM t WHERE i < f").unwrap().row_count(),
        1
    );
    assert_eq!(
        ctx.run("SELECT i FROM t WHERE i = f").unwrap().row_count(),
        1
    );
}

#[test]
fn or_in_having_filters_groups() {
    let t = table();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx
        .run("SELECT tag, COUNT(*) AS n FROM t GROUP BY tag HAVING COUNT(*) > 1 OR tag = 'z' ORDER BY tag")
        .unwrap();
    // Groups: x (2 rows), y (1 row), z (1 row). Keep x (count > 1) and z (tag = 'z').
    assert_eq!(out.row_count(), 2);
}

#[test]
fn type_mismatch_in_a_nested_condition_is_reported() {
    let t = table();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let err = ctx
        .run("SELECT id FROM t WHERE (id = 1 OR tag = a)")
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("cannot compare 'tag' (str) with 'a' (i64)"),
        "{err}"
    );
}

// Table: a = [1, NULL, 5, 5], b = [1, 2, NULL, 9], tag = [x, y, x, z].

#[test]
fn not_on_a_comparison_keeps_only_non_null_rows() {
    // Row 2 has a NULL `a`, so `a != 5` is NULL there and the row is dropped.
    assert_eq!(ids("SELECT id FROM t WHERE NOT a = 5"), vec![1]);
    assert_eq!(ids("SELECT id FROM t WHERE NOT a < 5"), vec![3, 4]);
}

#[test]
fn not_over_or_and_and_follows_de_morgan() {
    // Row 2: NULL OR false is NULL, so NOT (...) drops it.
    assert_eq!(ids("SELECT id FROM t WHERE NOT (a = 5 OR b = 9)"), vec![1]);
    // Row 2: NULL AND false is false, so NOT (...) keeps it.
    assert_eq!(
        ids("SELECT id FROM t WHERE NOT (a = 1 AND b = 1)"),
        vec![2, 3, 4]
    );
}

#[test]
fn not_in_and_not_between() {
    assert_eq!(ids("SELECT id FROM t WHERE a NOT IN (1, 9)"), vec![3, 4]);
    assert_eq!(
        ids("SELECT id FROM t WHERE a NOT BETWEEN 2 AND 4"),
        vec![1, 3, 4]
    );
}

#[test]
fn not_is_null_not_like_and_double_not() {
    assert_eq!(ids("SELECT id FROM t WHERE NOT a IS NULL"), vec![1, 3, 4]);
    assert_eq!(ids("SELECT id FROM t WHERE NOT tag LIKE 'x%'"), vec![2, 4]);
    assert_eq!(ids("SELECT id FROM t WHERE NOT NOT a = 1"), vec![1]);
}
