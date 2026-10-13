//! Edge cases for the typed execution paths: grouping, aggregation, sorting and
//! filtering on borrowed keys and typed accumulators.

use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, Scalar, Table};

fn s(v: &str) -> Scalar {
    Scalar::Str(v.to_string())
}

fn values(t: &Table, col: usize) -> Vec<Option<Scalar>> {
    let mut out = Vec::new();
    for b in t.batches() {
        for r in 0..b.row_count() {
            out.push(b.columns[col].get(r));
        }
    }
    out
}

fn run(t: &Table, sql: &str) -> Table {
    let mut ctx = SqlContext::new();
    ctx.add_table("t", t);
    ctx.run(sql).unwrap()
}

#[test]
fn min_and_max_return_the_extreme_values_not_the_first() {
    let t = Table::try_new(vec![
        Column::new("g", vec![s("a"), s("a"), s("a"), s("b")]),
        Column::new(
            "name",
            vec![s("mango"), Scalar::Null, s("apple"), s("kiwi")],
        ),
        Column::new(
            "x",
            vec![
                Scalar::F64(2.5),
                Scalar::F64(-1.0),
                Scalar::F64(9.0),
                Scalar::Null,
            ],
        ),
    ])
    .unwrap();
    let out = run(
        &t,
        "SELECT g, MIN(name) AS lo, MAX(name) AS hi, MIN(x) AS xlo, MAX(x) AS xhi \
         FROM t GROUP BY g ORDER BY g",
    );
    assert_eq!(values(&out, 1), vec![Some(s("apple")), Some(s("kiwi"))]);
    assert_eq!(values(&out, 2), vec![Some(s("mango")), Some(s("kiwi"))]);
    assert_eq!(values(&out, 3), vec![Some(Scalar::F64(-1.0)), None]);
    assert_eq!(values(&out, 4), vec![Some(Scalar::F64(9.0)), None]);
}

#[test]
fn null_group_key_forms_its_own_group() {
    let t = Table::try_new(vec![
        Column::new("g", vec![s("a"), Scalar::Null, Scalar::Null, s("a")]),
        Column::new(
            "v",
            vec![
                Scalar::I64(1),
                Scalar::I64(2),
                Scalar::I64(4),
                Scalar::I64(8),
            ],
        ),
    ])
    .unwrap();
    let out = run(&t, "SELECT g, SUM(v) AS total FROM t GROUP BY g ORDER BY g");
    assert_eq!(out.row_count(), 2);
    // NULL sorts first; its two rows sum to 6, and the "a" rows sum to 9.
    assert_eq!(values(&out, 0), vec![None, Some(s("a"))]);
    assert_eq!(
        values(&out, 1),
        vec![Some(Scalar::I64(6)), Some(Scalar::I64(9))]
    );
}

#[test]
fn global_aggregate_over_zero_rows_gives_one_row() {
    let t = Table::try_new(vec![Column::new("v", vec![Scalar::I64(1)])]).unwrap();
    let empty = run(&t, "SELECT v FROM t WHERE v > 100");
    assert_eq!(empty.row_count(), 0);
    let mut ctx = SqlContext::new();
    ctx.add_table("e", &empty);
    let out = ctx
        .run("SELECT COUNT(*) AS n, SUM(v) AS total, MIN(v) AS lo, AVG(v) AS mean FROM e")
        .unwrap();
    assert_eq!(out.row_count(), 1);
    assert_eq!(values(&out, 0), vec![Some(Scalar::I64(0))]);
    assert_eq!(values(&out, 1), vec![None]);
    assert_eq!(values(&out, 2), vec![None]);
    assert_eq!(values(&out, 3), vec![None]);
}

#[test]
fn average_of_large_integers_is_exact_enough() {
    let big = 9_007_199_254_740_993i64; // 2^53 + 1, not representable as f64
    let t = Table::try_new(vec![Column::new(
        "v",
        vec![Scalar::I64(big), Scalar::I64(big)],
    )])
    .unwrap();
    let out = run(&t, "SELECT AVG(v) AS m FROM t");
    assert_eq!(values(&out, 0), vec![Some(Scalar::F64(big as f64))]);
}

#[test]
fn sort_puts_nan_last_and_treats_negative_zero_as_zero() {
    let t = Table::try_new(vec![
        Column::new(
            "x",
            vec![
                Scalar::F64(f64::NAN),
                Scalar::F64(-0.0),
                Scalar::F64(3.0),
                Scalar::F64(0.0),
            ],
        ),
        Column::new(
            "id",
            vec![
                Scalar::I64(1),
                Scalar::I64(2),
                Scalar::I64(3),
                Scalar::I64(4),
            ],
        ),
    ])
    .unwrap();
    let out = run(&t, "SELECT id FROM t ORDER BY x");
    let ids = values(&out, 0);
    // The two zeros first (in their original order, so the sort is stable),
    // then 3.0, then NaN.
    assert_eq!(
        ids,
        vec![
            Some(Scalar::I64(2)),
            Some(Scalar::I64(4)),
            Some(Scalar::I64(3)),
            Some(Scalar::I64(1)),
        ]
    );
}

#[test]
fn filter_on_strings_and_booleans_uses_the_same_rules_as_before() {
    let t = Table::try_new(vec![
        Column::new("name", vec![s("ann"), Scalar::Null, s("bob")]),
        Column::new(
            "ok",
            vec![Scalar::Bool(true), Scalar::Bool(false), Scalar::Null],
        ),
    ])
    .unwrap();
    assert_eq!(
        run(&t, "SELECT name FROM t WHERE name >= 'b'").row_count(),
        1
    );
    assert_eq!(
        run(&t, "SELECT name FROM t WHERE name != 'ann'").row_count(),
        1
    );
    assert_eq!(
        run(&t, "SELECT name FROM t WHERE name LIKE 'a%'").row_count(),
        1
    );
    assert_eq!(run(&t, "SELECT name FROM t WHERE ok = TRUE").row_count(), 1);
    assert_eq!(
        run(&t, "SELECT name FROM t WHERE ok = FALSE").row_count(),
        1
    );
}

#[test]
fn join_on_string_keys_matches_exactly() {
    let left = Table::try_new(vec![
        Column::new("k", vec![s("x"), s("xy"), Scalar::Null]),
        Column::new("a", vec![Scalar::I64(1), Scalar::I64(2), Scalar::I64(3)]),
    ])
    .unwrap();
    let right = Table::try_new(vec![
        Column::new("code", vec![s("x"), s("xyz"), Scalar::Null]),
        Column::new("b", vec![Scalar::I64(10), Scalar::I64(20), Scalar::I64(30)]),
    ])
    .unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("l", &left);
    ctx.add_table("r", &right);
    let out = ctx
        .run("SELECT l.a, r.b FROM l JOIN r ON l.k = r.code")
        .unwrap();
    // Only "x" matches "x". "xy" is not "xyz", and NULL keys never match.
    assert_eq!(values(&out, 0), vec![Some(Scalar::I64(1))]);
    assert_eq!(values(&out, 1), vec![Some(Scalar::I64(10))]);
}
