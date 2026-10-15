//! The dense single-key path (integer SUM and COUNT over an `Int64` key) must
//! give the same groups, values and first-seen order as a plain pass. The
//! input here spans several batches and many segments, with NULL keys, NULL
//! values, negative keys, and both `COUNT(*)` and `COUNT(col)`.

use std::collections::HashMap;
use std::sync::Arc;

use tpt_strata::array::I64Array;
use tpt_strata::sql::SqlContext;
use tpt_strata::{ArrayRef, Batch, DataType, Field, Scalar, Schema, Table};

const BATCH_ROWS: usize = 90_000;
const BATCHES: usize = 3;

fn key_at(i: usize) -> Option<i64> {
    if i % 997 == 0 {
        None
    } else {
        Some(((i * 7919) % 5000) as i64 - 2000)
    }
}

fn value_at(i: usize) -> Option<i64> {
    if i % 11 == 0 {
        None
    } else {
        Some((i % 13) as i64 + 1)
    }
}

fn table() -> Table {
    let schema = Arc::new(Schema::new(vec![
        Field::new("k", DataType::Int64, true),
        Field::new("v", DataType::Int64, true),
    ]));
    let batches = (0..BATCHES)
        .map(|b| {
            let range = b * BATCH_ROWS..(b + 1) * BATCH_ROWS;
            let keys: Vec<Option<i64>> = range.clone().map(key_at).collect();
            let values: Vec<Option<i64>> = range.map(value_at).collect();
            Batch::try_new(
                schema.clone(),
                vec![
                    ArrayRef::Int64(I64Array::from_nulls(keys)),
                    ArrayRef::Int64(I64Array::from_nulls(values)),
                ],
            )
            .unwrap()
        })
        .collect();
    Table::from_batches(schema, batches)
}

/// The expected rows, computed by a plain pass over the same input: groups in
/// first-seen order, each as `[key, COUNT(*), COUNT(v), SUM(v)]`.
fn expected(min_count_star: Option<i64>) -> Vec<Vec<Option<Scalar>>> {
    let mut order: Vec<Option<i64>> = Vec::new();
    let mut acc: HashMap<Option<i64>, (i64, i64, Option<i64>)> = HashMap::new();
    for i in 0..BATCH_ROWS * BATCHES {
        let k = key_at(i);
        let entry = acc.entry(k).or_insert_with(|| {
            order.push(k);
            (0, 0, None)
        });
        entry.0 += 1;
        if let Some(v) = value_at(i) {
            entry.1 += 1;
            entry.2 = Some(entry.2.unwrap_or(0) + v);
        }
    }
    order
        .into_iter()
        .filter_map(|k| {
            let (n, c, total) = acc[&k];
            if min_count_star.is_some_and(|m| n <= m) {
                return None;
            }
            Some(vec![
                k.map(Scalar::I64),
                Some(Scalar::I64(n)),
                Some(Scalar::I64(c)),
                total.map(Scalar::I64),
            ])
        })
        .collect()
}

fn rows(t: &Table) -> Vec<Vec<Option<Scalar>>> {
    let mut out = Vec::new();
    for b in t.batches() {
        for r in 0..b.row_count() {
            out.push(b.columns.iter().map(|c| c.get(r)).collect());
        }
    }
    out
}

#[test]
fn dense_sum_and_counts_match_a_plain_pass_across_segments() {
    let t = table();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx
        .run("SELECT k, COUNT(*) AS n, COUNT(v) AS c, SUM(v) AS total FROM t GROUP BY k")
        .unwrap();
    assert_eq!(rows(&out), expected(None));
}

#[test]
fn dense_having_keeps_the_same_groups_as_a_plain_pass() {
    let t = table();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx
        .run(
            "SELECT k, COUNT(*) AS n, COUNT(v) AS c, SUM(v) AS total FROM t \
             GROUP BY k HAVING COUNT(*) > 1000",
        )
        .unwrap();
    assert_eq!(rows(&out), expected(Some(1000)));
}

#[test]
fn dense_having_on_a_sum_keeps_the_same_groups_as_a_plain_pass() {
    let t = table();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx
        .run(
            "SELECT k, COUNT(*) AS n, COUNT(v) AS c, SUM(v) AS total FROM t \
             GROUP BY k HAVING SUM(v) > 380",
        )
        .unwrap();
    // The reference keeps the groups whose SUM is above 380. A NULL sum is
    // never above it.
    let want: Vec<_> = expected(None)
        .into_iter()
        .filter(|row| matches!(row[3], Some(Scalar::I64(s)) if s > 380))
        .collect();
    let all = expected(None);
    assert!(!want.is_empty() && want.len() < all.len());
    assert_eq!(rows(&out), want);
}
