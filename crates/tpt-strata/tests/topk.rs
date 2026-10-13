//! ORDER BY with LIMIT keeps only the top rows at the sort. The result must
//! match a full sort followed by the same slice, including ties and OFFSET.

use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, Scalar, Table};

fn table() -> Table {
    // Many duplicate keys, so tie handling is exercised.
    let n = 1000;
    Table::try_new(vec![
        Column::new("id", (0..n).map(|i| Scalar::I64(i as i64)).collect()),
        Column::new(
            "k",
            (0..n).map(|i| Scalar::I64((i * 7 % 13) as i64)).collect(),
        ),
    ])
    .unwrap()
}

fn ids(t: &Table) -> Vec<i64> {
    let mut v = Vec::new();
    for b in t.batches() {
        for r in 0..b.row_count() {
            if let Some(Scalar::I64(n)) = b.columns[0].get(r) {
                v.push(n);
            }
        }
    }
    v
}

fn run(sql: &str) -> Vec<i64> {
    let t = table();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    ids(&ctx.run(sql).unwrap())
}

#[test]
fn limit_matches_a_full_sort_sliced_by_hand_with_ties() {
    let full = run("SELECT id FROM t ORDER BY k");
    for (n, off) in [
        (1, 0),
        (10, 0),
        (25, 0),
        (10, 7),
        (500, 3),
        (1000, 0),
        (2000, 0),
    ] {
        let want: Vec<i64> = full.iter().copied().skip(off).take(n).collect();
        let got = run(&format!(
            "SELECT id FROM t ORDER BY k LIMIT {n} OFFSET {off}"
        ));
        assert_eq!(got, want, "LIMIT {n} OFFSET {off}");
    }
}

#[test]
fn descending_limit_matches_a_full_sort() {
    let full = run("SELECT id FROM t ORDER BY k DESC, id");
    let got = run("SELECT id FROM t ORDER BY k DESC, id LIMIT 17");
    assert_eq!(got, full[..17]);
}

#[test]
fn limit_zero_returns_no_rows() {
    assert!(run("SELECT id FROM t ORDER BY k LIMIT 0").is_empty());
}

#[test]
fn builder_order_by_then_limit_matches_sql() {
    let t = table();
    let built = tpt_strata::QueryBuilder::new(&t)
        .order_by("k", false)
        .unwrap()
        .limit(9)
        .execute()
        .unwrap();
    assert_eq!(ids(&built), run("SELECT id FROM t ORDER BY k LIMIT 9"));
}
