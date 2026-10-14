//! Join index: the direct layout for small `Int64` key ranges and the hash
//! layout for wide ranges must give the same rows. Covers duplicate keys,
//! NULL keys on both sides and unmatched rows of an outer join.

use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, Scalar, Table};

fn s(v: &str) -> Scalar {
    Scalar::Str(v.to_string())
}

fn col(t: &Table, c: usize) -> Vec<Option<Scalar>> {
    let mut out = Vec::new();
    for b in t.batches() {
        for r in 0..b.row_count() {
            out.push(b.columns[c].get(r));
        }
    }
    out
}

/// Two-column output as sorted `(left, right)` pairs, so the order of the
/// engine's output does not matter.
fn pairs(t: &Table) -> Vec<String> {
    let mut out: Vec<String> = col(t, 0)
        .into_iter()
        .zip(col(t, 1))
        .map(|(a, b)| format!("{a:?}|{b:?}"))
        .collect();
    out.sort();
    out
}

/// Left `lt(k, name)` and right `rt(k2, v)` with duplicate keys and NULLs.
fn tables(left_keys: Vec<Scalar>, right_keys: Vec<Scalar>) -> (Table, Table) {
    let lt = Table::try_new(vec![
        Column::new("k", left_keys),
        Column::new("name", vec![s("a"), s("b"), s("c"), s("d")]),
    ])
    .unwrap();
    let rt = Table::try_new(vec![
        Column::new("k2", right_keys),
        Column::new("v", vec![s("x"), s("y"), s("z"), s("w")]),
    ])
    .unwrap();
    (lt, rt)
}

fn run(lt: &Table, rt: &Table, sql: &str) -> Table {
    let mut ctx = SqlContext::new();
    ctx.add_table("lt", lt);
    ctx.add_table("rt", rt);
    ctx.run(sql).unwrap()
}

#[test]
fn dense_inner_join_keeps_every_duplicate_and_skips_nulls() {
    // Keys span 1..=5, so this takes the direct layout.
    let (lt, rt) = tables(
        vec![Scalar::I64(1), Scalar::I64(2), Scalar::Null, Scalar::I64(5)],
        vec![Scalar::I64(1), Scalar::I64(1), Scalar::Null, Scalar::I64(2)],
    );
    let out = run(
        &lt,
        &rt,
        "SELECT lt.name, rt.v FROM lt JOIN rt ON lt.k = rt.k2",
    );
    assert_eq!(
        pairs(&out),
        vec![
            format!("{:?}|{:?}", Some(s("a")), Some(s("x"))),
            format!("{:?}|{:?}", Some(s("a")), Some(s("y"))),
            format!("{:?}|{:?}", Some(s("b")), Some(s("w"))),
        ]
    );
}

#[test]
fn dense_left_join_pads_unmatched_and_null_keys() {
    let (lt, rt) = tables(
        vec![Scalar::I64(1), Scalar::I64(2), Scalar::Null, Scalar::I64(5)],
        vec![Scalar::I64(1), Scalar::I64(1), Scalar::Null, Scalar::I64(2)],
    );
    let out = run(
        &lt,
        &rt,
        "SELECT lt.name, rt.v FROM lt LEFT JOIN rt ON lt.k = rt.k2",
    );
    // a matches x and y, b matches w, c and d keep one row each: 5 rows.
    assert_eq!(out.row_count(), 5);
    // c (NULL key) and d (key 5, no partner) keep their row with NULL on the right.
    let nulls = col(&out, 1).iter().filter(|v| v.is_none()).count();
    assert_eq!(nulls, 2);
}

#[test]
fn wide_range_takes_hash_path_with_same_result() {
    // Keys around 10^12 fall outside the direct layout's range limit.
    let big = 1_000_000_000_000i64;
    let (lt, rt) = tables(
        vec![
            Scalar::I64(1),
            Scalar::I64(big),
            Scalar::I64(-5),
            Scalar::Null,
        ],
        vec![
            Scalar::I64(big),
            Scalar::I64(1),
            Scalar::I64(big),
            Scalar::Null,
        ],
    );
    let out = run(
        &lt,
        &rt,
        "SELECT lt.name, rt.v FROM lt JOIN rt ON lt.k = rt.k2",
    );
    // a (key 1) matches y; b (key 10^12) matches x and z; c and d match nothing.
    assert_eq!(
        pairs(&out),
        vec![
            format!("{:?}|{:?}", Some(s("a")), Some(s("y"))),
            format!("{:?}|{:?}", Some(s("b")), Some(s("x"))),
            format!("{:?}|{:?}", Some(s("b")), Some(s("z"))),
        ]
    );
}
