//! A filter keeps only the columns the query reads above it. A column that only
//! WHERE reads must still filter correctly, and a column that ORDER BY reads
//! but SELECT does not must still sort.

use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, Scalar, Table};

fn table() -> Table {
    Table::try_new(vec![
        Column::new("id", (1..=6).map(Scalar::I64).collect()),
        Column::new(
            "region",
            ["a", "b", "a", "c", "a", "c"]
                .iter()
                .map(|s| Scalar::Str(s.to_string()))
                .collect(),
        ),
        Column::new(
            "qty",
            vec![
                Scalar::I64(50),
                Scalar::I64(10),
                Scalar::I64(70),
                Scalar::I64(30),
                Scalar::I64(20),
                Scalar::I64(90),
            ],
        ),
    ])
    .unwrap()
}

fn ids(t: &Table) -> Vec<Option<Scalar>> {
    let mut out = Vec::new();
    for b in t.batches() {
        for r in 0..b.row_count() {
            out.push(b.columns[0].get(r));
        }
    }
    out
}

#[test]
fn where_only_column_still_filters() {
    let t = table();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    // `region` is read only by WHERE, so the filter drops it from its output.
    let out = ctx
        .run("SELECT id FROM t WHERE region = 'a' ORDER BY id")
        .unwrap();
    assert_eq!(
        ids(&out),
        vec![
            Some(Scalar::I64(1)),
            Some(Scalar::I64(3)),
            Some(Scalar::I64(5))
        ]
    );
}

#[test]
fn order_by_column_not_selected_still_sorts() {
    let t = table();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    // `qty` is read only by ORDER BY, which runs above the filter.
    let out = ctx
        .run("SELECT id FROM t WHERE region = 'a' ORDER BY qty DESC")
        .unwrap();
    // qty for region 'a' rows: id1=50, id3=70, id5=20 -> descending: 3, 1, 5.
    assert_eq!(
        ids(&out),
        vec![
            Some(Scalar::I64(3)),
            Some(Scalar::I64(1)),
            Some(Scalar::I64(5))
        ]
    );
}

#[test]
fn aggregate_over_filtered_rows_is_unchanged() {
    let t = table();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx
        .run("SELECT COUNT(*) AS n, SUM(qty) AS total FROM t WHERE region LIKE 'c%'")
        .unwrap();
    // Region 'c' rows are id4 (qty 30) and id6 (qty 90).
    assert_eq!(out.row_count(), 1);
    assert_eq!(out.batches()[0].columns[0].get(0), Some(Scalar::I64(2)));
    assert_eq!(out.batches()[0].columns[1].get(0), Some(Scalar::I64(120)));
}

#[test]
fn explain_still_names_the_filter_column() {
    let t = table();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let plan = ctx.explain("SELECT id FROM t WHERE region = 'a'").unwrap();
    assert!(plan.contains("region"), "{plan}");
}
