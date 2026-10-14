//! GROUP BY on one `Int64` key uses direct slots when the keys are dense, and
//! hashing otherwise. Both give the same groups, NULL group included, in
//! first-seen order.

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

fn table(keys: Vec<Scalar>) -> Table {
    let n = keys.len();
    Table::try_new(vec![
        Column::new("k", keys),
        Column::new("v", (1..=n as i64).map(|x| Scalar::I64(x * 10)).collect()),
    ])
    .unwrap()
}

#[test]
fn dense_keys_group_with_null_in_first_seen_order() {
    // Keys span 1..=3, so the direct slots are used.
    let t = table(vec![
        Scalar::I64(3),
        Scalar::I64(1),
        Scalar::I64(3),
        Scalar::Null,
        Scalar::I64(1),
        Scalar::I64(2),
    ]);
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx
        .run("SELECT k, COUNT(*) AS n, SUM(v) AS total FROM t GROUP BY k")
        .unwrap();
    // v = 10, 20, 30, 40, 50, 60. Groups in first-seen order: 3, 1, NULL, 2.
    assert_eq!(
        rows(&out),
        vec![
            vec![
                Some(Scalar::I64(3)),
                Some(Scalar::I64(2)),
                Some(Scalar::I64(40))
            ],
            vec![
                Some(Scalar::I64(1)),
                Some(Scalar::I64(2)),
                Some(Scalar::I64(70))
            ],
            vec![None, Some(Scalar::I64(1)), Some(Scalar::I64(40))],
            vec![
                Some(Scalar::I64(2)),
                Some(Scalar::I64(1)),
                Some(Scalar::I64(60))
            ],
        ]
    );
}

#[test]
fn wide_key_range_falls_back_to_hashing_with_the_same_result() {
    let big = 1_000_000_000_000i64;
    let t = table(vec![
        Scalar::I64(big),
        Scalar::I64(-7),
        Scalar::I64(big),
        Scalar::Null,
    ]);
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx
        .run("SELECT k, COUNT(*) AS n, SUM(v) AS total FROM t GROUP BY k")
        .unwrap();
    assert_eq!(
        rows(&out),
        vec![
            vec![
                Some(Scalar::I64(big)),
                Some(Scalar::I64(2)),
                Some(Scalar::I64(40))
            ],
            vec![
                Some(Scalar::I64(-7)),
                Some(Scalar::I64(1)),
                Some(Scalar::I64(20))
            ],
            vec![None, Some(Scalar::I64(1)), Some(Scalar::I64(40))],
        ]
    );
}
