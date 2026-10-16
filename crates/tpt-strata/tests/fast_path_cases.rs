//! Cases for the fast paths: the dense integer aggregate, HAVING on sums and
//! counts, and the fused, typed filter. Each result is checked against a plain
//! computation over the same generated data.

use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, Scalar, Table};

const N: usize = 300_000;
const REGIONS: [&str; 4] = ["north", "central", "coastal", "south"];

/// A LIKE pattern and the plain test it should agree with.
type Shape = (&'static str, fn(&str) -> bool);

/// A comparison between a sum and a literal.
type IntTest = fn(i64, i64) -> bool;

fn region_at(i: usize) -> &'static str {
    REGIONS[i % 4]
}

fn quantity_at(i: usize) -> Option<i64> {
    if i % 13 == 0 {
        None
    } else {
        Some((i % 97) as i64)
    }
}

/// A table of `N` rows: a string `region`, a nullable integer `quantity`, and
/// an integer `k` in 0..6 that is dense.
fn sales() -> Table {
    Table::try_new(vec![
        Column::new(
            "region",
            (0..N).map(|i| Scalar::Str(region_at(i).into())).collect(),
        ),
        Column::new(
            "quantity",
            (0..N)
                .map(|i| quantity_at(i).map_or(Scalar::Null, Scalar::I64))
                .collect(),
        ),
        Column::new("k", (0..N).map(|i| Scalar::I64((i % 6) as i64)).collect()),
    ])
    .unwrap()
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

fn run(sql: &str) -> Table {
    let t = sales();
    let mut ctx = SqlContext::new();
    ctx.add_table("sales", &t);
    ctx.run(sql).unwrap()
}

#[test]
fn group_whose_values_are_all_null_has_null_sum_and_zero_count() {
    // Key 2 has no non-null value, so its SUM is NULL and its COUNT(v) is 0.
    let t = Table::try_new(vec![
        Column::new(
            "k",
            vec![
                Scalar::I64(1),
                Scalar::I64(2),
                Scalar::I64(2),
                Scalar::I64(1),
            ],
        ),
        Column::new(
            "v",
            vec![Scalar::I64(5), Scalar::Null, Scalar::Null, Scalar::I64(7)],
        ),
    ])
    .unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx
        .run("SELECT k, COUNT(*) AS n, COUNT(v) AS c, SUM(v) AS total FROM t GROUP BY k")
        .unwrap();
    assert_eq!(
        rows(&out),
        vec![
            vec![
                Some(Scalar::I64(1)),
                Some(Scalar::I64(2)),
                Some(Scalar::I64(2)),
                Some(Scalar::I64(12)),
            ],
            vec![
                Some(Scalar::I64(2)),
                Some(Scalar::I64(2)),
                Some(Scalar::I64(0)),
                None,
            ],
        ]
    );
}

#[test]
fn having_with_and_of_sum_and_count_matches_a_plain_pass() {
    let out = run(
        "SELECT k, SUM(quantity) AS total, COUNT(*) AS n FROM sales \
         GROUP BY k HAVING SUM(quantity) > 7000 AND COUNT(*) > 40000",
    );
    // Plain pass: per key, the NULL-skipping sum and the row count.
    let mut sums = [0i64; 6];
    let mut counts = [0i64; 6];
    for i in 0..N {
        let k = i % 6;
        counts[k] += 1;
        if let Some(q) = quantity_at(i) {
            sums[k] += q;
        }
    }
    let want: Vec<Vec<Option<Scalar>>> = (0..6)
        .filter(|&k| sums[k] > 7000 && counts[k] > 40000)
        .map(|k| {
            vec![
                Some(Scalar::I64(k as i64)),
                Some(Scalar::I64(sums[k])),
                Some(Scalar::I64(counts[k])),
            ]
        })
        .collect();
    assert!(!want.is_empty());
    assert_eq!(rows(&out), want);
}

#[test]
fn like_with_another_predicate_matches_a_plain_pass() {
    let out = run("SELECT COUNT(*) AS n, SUM(quantity) AS total FROM sales \
         WHERE region LIKE 'c%' AND quantity > 50");
    let (mut n, mut total) = (0i64, 0i64);
    for i in 0..N {
        if region_at(i).starts_with('c') {
            if let Some(q) = quantity_at(i) {
                if q > 50 {
                    n += 1;
                    total += q;
                }
            }
        }
    }
    assert_eq!(
        rows(&out),
        vec![vec![Some(Scalar::I64(n)), Some(Scalar::I64(total))]]
    );
}

#[test]
fn every_like_shape_matches_a_plain_pass() {
    // Exact, prefix, suffix, contains, and the general shape with `_`.
    let cases: [Shape; 5] = [
        ("south", |s| s == "south"),
        ("c%", |s| s.starts_with('c')),
        ("%th", |s| s.ends_with("th")),
        ("%o%", |s| s.contains('o')),
        ("n_rth", |s| {
            s.len() == 5 && s.starts_with('n') && &s[2..] == "rth"
        }),
    ];
    for (pattern, plain) in cases {
        let out = run(&format!(
            "SELECT COUNT(*) AS n FROM sales WHERE region LIKE '{pattern}'"
        ));
        let want = (0..N).filter(|&i| plain(region_at(i))).count() as i64;
        assert_eq!(
            rows(&out),
            vec![vec![Some(Scalar::I64(want))]],
            "LIKE '{pattern}'"
        );
        let out = run(&format!(
            "SELECT COUNT(*) AS n FROM sales WHERE region NOT LIKE '{pattern}'"
        ));
        let want = (0..N).filter(|&i| !plain(region_at(i))).count() as i64;
        assert_eq!(
            rows(&out),
            vec![vec![Some(Scalar::I64(want))]],
            "NOT LIKE '{pattern}'"
        );
    }
}

#[test]
fn filter_with_a_null_comparison_drops_the_null_rows() {
    // A NULL on either side is unknown, so the row is filtered out. That holds
    // for every predicate of a conjunction.
    let out = run("SELECT COUNT(*) AS n FROM sales WHERE quantity > 50 AND region = 'north'");
    let want = (0..N)
        .filter(|&i| quantity_at(i).is_some_and(|q| q > 50) && region_at(i) == "north")
        .count() as i64;
    assert_eq!(rows(&out), vec![vec![Some(Scalar::I64(want))]]);
}

#[test]
fn having_sum_overflow_is_reported_on_the_dense_path() {
    let big = i64::MAX / 2;
    let t = Table::try_new(vec![
        Column::new("k", (0..12).map(|i| Scalar::I64(i % 4)).collect()),
        Column::new("v", vec![Scalar::I64(big); 12]),
    ])
    .unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    assert!(ctx
        .run("SELECT k FROM t GROUP BY k HAVING SUM(v) > 0")
        .is_err());
}

#[test]
fn single_sum_having_keeps_the_same_groups_for_every_operator() {
    // One comparison on a SUM takes the dense path, which decides HAVING on the
    // accumulators before building the output. Each operator must keep the same
    // groups, in the same order, as a plain pass over the data.
    let mut sums = [0i64; 6];
    let mut seen = [false; 6];
    for i in 0..N {
        let k = i % 6;
        if let Some(q) = quantity_at(i) {
            sums[k] += q;
            seen[k] = true;
        }
    }
    let threshold = sums[2];
    let ops: [(&str, IntTest); 6] = [
        ("=", |a, b| a == b),
        ("!=", |a, b| a != b),
        (">", |a, b| a > b),
        (">=", |a, b| a >= b),
        ("<", |a, b| a < b),
        ("<=", |a, b| a <= b),
    ];
    for (op, test) in ops {
        let out = run(&format!(
            "SELECT k, SUM(quantity) AS total FROM sales GROUP BY k HAVING SUM(quantity) {op} {threshold}"
        ));
        let want: Vec<Vec<Option<Scalar>>> = (0..6)
            .filter(|&k| seen[k] && test(sums[k], threshold))
            .map(|k| vec![Some(Scalar::I64(k as i64)), Some(Scalar::I64(sums[k]))])
            .collect();
        assert!(!want.is_empty(), "operator {op} keeps nothing");
        assert_eq!(rows(&out), want, "HAVING SUM(quantity) {op} {threshold}");
    }
}

#[test]
fn sum_only_group_by_spanning_segments_matches_a_plain_pass() {
    // SUM with no COUNT: the dense path keeps no row counts, so this checks the
    // sums across segments with NULLs, with groups in first-seen order.
    let out = run("SELECT k, SUM(quantity) AS total FROM sales GROUP BY k");
    let mut sums = [0i64; 6];
    let mut seen = [false; 6];
    for i in 0..N {
        if let Some(q) = quantity_at(i) {
            sums[i % 6] += q;
            seen[i % 6] = true;
        }
    }
    let want: Vec<Vec<Option<Scalar>>> = (0..6)
        .map(|k| {
            let total = if seen[k] {
                Some(Scalar::I64(sums[k]))
            } else {
                None
            };
            vec![Some(Scalar::I64(k as i64)), total]
        })
        .collect();
    assert_eq!(rows(&out), want);
}
