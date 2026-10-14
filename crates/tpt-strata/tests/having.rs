//! HAVING is applied inside the aggregate. These tests check the groups it
//! keeps against a hand-computed answer, including conditions that read a
//! group key, an aggregate that is not selected, and AND/OR together.

use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, Scalar, Table};

fn rows(t: &Table) -> Vec<Vec<Option<Scalar>>> {
    let mut out = Vec::new();
    for b in t.batches() {
        for r in 0..b.row_count() {
            out.push(b.columns.iter().map(|c| c.get(r)).collect());
        }
    }
    out
}

fn table() -> Table {
    // k: 1, 2, 1, 3, 2, 3, 4 with v: 10, 20, 30, 40, 50, 60, 70.
    let k = [1, 2, 1, 3, 2, 3, 4];
    let v = [10, 20, 30, 40, 50, 60, 70];
    Table::try_new(vec![
        Column::new("k", k.iter().map(|&x| Scalar::I64(x)).collect()),
        Column::new("v", v.iter().map(|&x| Scalar::I64(x)).collect()),
    ])
    .unwrap()
}

#[test]
fn having_on_aggregate_and_group_key_with_and_or() {
    let t = table();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    // Groups in first-seen order: 1 (sum 40), 2 (sum 70), 3 (sum 100), 4 (sum 70).
    // Keep SUM > 60 and k <> 3, or k = 4. That keeps 2 and 4.
    let out = ctx
        .run("SELECT k, SUM(v) AS s FROM t GROUP BY k HAVING SUM(v) > 60 AND k <> 3 OR k = 4")
        .unwrap();
    assert_eq!(
        rows(&out),
        vec![
            vec![Some(Scalar::I64(2)), Some(Scalar::I64(70))],
            vec![Some(Scalar::I64(4)), Some(Scalar::I64(70))],
        ]
    );
}

#[test]
fn having_on_an_aggregate_not_in_select() {
    let t = table();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    // COUNT(*) is not selected. Groups with at least two rows are 1, 2 and 3.
    let out = ctx
        .run("SELECT k, SUM(v) AS s FROM t GROUP BY k HAVING COUNT(*) >= 2")
        .unwrap();
    assert_eq!(
        rows(&out),
        vec![
            vec![Some(Scalar::I64(1)), Some(Scalar::I64(40))],
            vec![Some(Scalar::I64(2)), Some(Scalar::I64(70))],
            vec![Some(Scalar::I64(3)), Some(Scalar::I64(100))],
        ]
    );
}

#[test]
fn having_that_keeps_nothing_gives_no_rows() {
    let t = table();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx
        .run("SELECT k, SUM(v) AS s FROM t GROUP BY k HAVING SUM(v) > 1000")
        .unwrap();
    assert_eq!(out.row_count(), 0);
}

#[test]
fn having_matches_the_same_query_with_order_and_limit() {
    let t = table();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx
        .run("SELECT k, SUM(v) AS s FROM t GROUP BY k HAVING COUNT(*) >= 2 ORDER BY s DESC LIMIT 2")
        .unwrap();
    // Sums of the kept groups: 3 -> 100, 2 -> 70, 1 -> 40. Top two by sum.
    assert_eq!(
        rows(&out),
        vec![
            vec![Some(Scalar::I64(3)), Some(Scalar::I64(100))],
            vec![Some(Scalar::I64(2)), Some(Scalar::I64(70))],
        ]
    );
}
