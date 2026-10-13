//! Aggregation over more than one segment. The results must match values
//! computed here, whether one thread or several (`--features parallel`) run it.

use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, Scalar, Table};

const N: usize = 200_000; // several 64k-row segments

fn table() -> Table {
    Table::try_new(vec![
        Column::new("g", (0..N).map(|i| Scalar::I64((i % 7) as i64)).collect()),
        Column::new("v", (0..N).map(|i| Scalar::I64(i as i64)).collect()),
        Column::new(
            "f",
            (0..N).map(|i| Scalar::F64((i % 1000) as f64)).collect(),
        ),
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

#[test]
fn grouped_aggregates_match_values_computed_by_hand() {
    let t = table();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx
        .run("SELECT g, COUNT(*), SUM(v), MIN(v), MAX(v), SUM(f) FROM t GROUP BY g ORDER BY g")
        .unwrap();
    let got = rows(&out);
    assert_eq!(got.len(), 7);
    for (g, row) in got.iter().enumerate() {
        let members: Vec<usize> = (0..N).filter(|i| i % 7 == g).collect();
        let sum: i64 = members.iter().map(|&i| i as i64).sum();
        let sumf: f64 = members.iter().map(|&i| (i % 1000) as f64).sum();
        assert_eq!(row[0], Some(Scalar::I64(g as i64)));
        assert_eq!(row[1], Some(Scalar::I64(members.len() as i64)));
        assert_eq!(row[2], Some(Scalar::I64(sum)));
        assert_eq!(row[3], Some(Scalar::I64(*members.first().unwrap() as i64)));
        assert_eq!(row[4], Some(Scalar::I64(*members.last().unwrap() as i64)));
        assert_eq!(row[5], Some(Scalar::F64(sumf)));
    }
}

#[test]
fn global_aggregates_over_many_segments() {
    let t = table();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx.run("SELECT COUNT(*), SUM(v), MAX(v) FROM t").unwrap();
    let got = rows(&out);
    assert_eq!(got.len(), 1);
    let sum: i64 = (0..N as i64).sum();
    assert_eq!(got[0][0], Some(Scalar::I64(N as i64)));
    assert_eq!(got[0][1], Some(Scalar::I64(sum)));
    assert_eq!(got[0][2], Some(Scalar::I64(N as i64 - 1)));
}

#[test]
fn global_aggregate_over_no_rows_still_returns_one_row() {
    let t = Table::empty(std::sync::Arc::new(tpt_strata::Schema::new(vec![
        tpt_strata::Field::new("v", tpt_strata::DataType::Int64, false),
    ])));
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx.run("SELECT COUNT(*) FROM t").unwrap();
    assert_eq!(rows(&out), vec![vec![Some(Scalar::I64(0))]]);
}
