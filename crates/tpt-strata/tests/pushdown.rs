//! Grouped inner joins are rewritten to aggregate the left side first. The
//! rewrite must give the same rows as joining first. These tests check that
//! against a brute-force answer, and against the same query forced onto the
//! ordinary plan with a WHERE clause that every row passes.

use std::collections::BTreeMap;

use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, Scalar, Table};

/// `(dim_id, quantity)` rows of the left table. `None` is a NULL key.
fn facts_rows() -> Vec<(Option<i64>, i64)> {
    vec![
        (Some(1), 5),
        (Some(1), -3),
        (Some(2), 7),
        (Some(3), 2),
        (None, 9),
        (Some(4), 4),
        (Some(2), 10),
        (Some(99), 6),
        (Some(3), -8),
    ]
}

/// `(id, label)` rows of the right table. Id 1 appears twice, so each left row
/// with dim_id 1 joins twice.
fn dims_rows() -> Vec<(Option<i64>, &'static str)> {
    vec![
        (Some(1), "a"),
        (Some(1), "b"),
        (Some(2), "a"),
        (Some(3), "c"),
        (Some(4), "d"),
        (None, "e"),
    ]
}

fn tables() -> (Table, Table) {
    let facts = facts_rows();
    let dims = dims_rows();
    let key = |k: Option<i64>| k.map_or(Scalar::Null, Scalar::I64);
    let left = Table::try_new(vec![
        Column::new("dim_id", facts.iter().map(|(d, _)| key(*d)).collect()),
        Column::new(
            "quantity",
            facts.iter().map(|(_, q)| Scalar::I64(*q)).collect(),
        ),
    ])
    .unwrap();
    let right = Table::try_new(vec![
        Column::new("id", dims.iter().map(|(i, _)| key(*i)).collect()),
        Column::new(
            "label",
            dims.iter()
                .map(|(_, l)| Scalar::Str(l.to_string()))
                .collect(),
        ),
    ])
    .unwrap();
    (left, right)
}

/// The answer by brute force: nested loops over the join, NULL keys excluded.
/// Each row is `(label, sum, count, min, max)`, sorted by label.
fn expected() -> Vec<Vec<Option<Scalar>>> {
    // label -> (sum, count, min, max)
    let mut groups: BTreeMap<&str, (i64, i64, i64, i64)> = BTreeMap::new();
    for (fd, q) in facts_rows() {
        let Some(fd) = fd else { continue };
        for (dd, label) in dims_rows() {
            if dd != Some(fd) {
                continue;
            }
            let g = groups.entry(label).or_insert((0, 0, i64::MAX, i64::MIN));
            g.0 += q;
            g.1 += 1;
            g.2 = g.2.min(q);
            g.3 = g.3.max(q);
        }
    }
    groups
        .into_iter()
        .map(|(label, (sum, n, lo, hi))| {
            vec![
                Some(Scalar::Str(label.to_string())),
                Some(Scalar::I64(sum)),
                Some(Scalar::I64(n)),
                Some(Scalar::I64(lo)),
                Some(Scalar::I64(hi)),
            ]
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

const GROUPED: &str = "SELECT dims.label, SUM(facts.quantity) AS units, COUNT(*) AS n, \
     MIN(facts.quantity) AS lo, MAX(facts.quantity) AS hi \
     FROM facts JOIN dims ON facts.dim_id = dims.id \
     GROUP BY dims.label ORDER BY label";

#[test]
fn pushed_down_join_matches_brute_force() {
    let (facts, dims) = tables();
    let mut ctx = SqlContext::new();
    ctx.add_table("facts", &facts);
    ctx.add_table("dims", &dims);
    let out = ctx.run(GROUPED).unwrap();
    assert_eq!(rows(&out), expected());
}

#[test]
fn pushed_down_join_matches_the_ordinary_plan() {
    let (facts, dims) = tables();
    let mut ctx = SqlContext::new();
    ctx.add_table("facts", &facts);
    ctx.add_table("dims", &dims);
    // A WHERE that every row passes forces the ordinary plan.
    let forced = GROUPED.replace(
        "GROUP BY dims.label",
        "WHERE facts.quantity >= -1000 GROUP BY dims.label",
    );
    let fast = ctx.run(GROUPED).unwrap();
    let plain = ctx.run(&forced).unwrap();
    assert_eq!(rows(&fast), rows(&plain));
}

#[test]
fn explain_shows_the_partial_aggregate_under_the_join() {
    let (facts, dims) = tables();
    let mut ctx = SqlContext::new();
    ctx.add_table("facts", &facts);
    ctx.add_table("dims", &dims);
    let plan = ctx.explain(GROUPED).unwrap();
    // Two aggregates: the partial one sits below the join, the final one above.
    assert_eq!(plan.matches("Aggregate").count(), 2, "{plan}");
}

#[test]
fn average_is_not_pushed_down_and_still_answers() {
    let (facts, dims) = tables();
    let mut ctx = SqlContext::new();
    ctx.add_table("facts", &facts);
    ctx.add_table("dims", &dims);
    let out = ctx
        .run("SELECT dims.label, AVG(facts.quantity) AS m FROM facts JOIN dims ON facts.dim_id = dims.id GROUP BY dims.label ORDER BY label")
        .unwrap();
    assert_eq!(out.row_count(), 4);
}
