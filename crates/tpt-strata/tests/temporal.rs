//! DATE and TIMESTAMP as native types: literals, comparisons, grouping,
//! joins, ordering, aggregates, and the diagnostics for bad values.

use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, QueryBuilder, Scalar, Table};

fn d(s: &str) -> Scalar {
    Scalar::Date(tpt_strata::temporal::parse_date(s).unwrap())
}

fn ts(s: &str) -> Scalar {
    Scalar::Timestamp(tpt_strata::temporal::parse_timestamp(s).unwrap())
}

fn events() -> Table {
    Table::try_new(vec![
        Column::new(
            "id",
            vec![
                Scalar::I64(1),
                Scalar::I64(2),
                Scalar::I64(3),
                Scalar::I64(4),
            ],
        ),
        Column::new(
            "day",
            vec![
                d("2026-01-05"),
                d("2026-02-10"),
                d("2026-02-10"),
                d("2025-12-31"),
            ],
        ),
        Column::new(
            "at",
            vec![
                ts("2026-01-05 08:00:00"),
                ts("2026-02-10 12:30:00.5"),
                ts("2026-02-10 23:59:59"),
                Scalar::Null,
            ],
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

fn ctx_run(sql: &str) -> Result<Table, tpt_strata::QueryError> {
    let t = events();
    let mut ctx = SqlContext::new();
    ctx.add_table("events", &t);
    ctx.run(sql)
}

#[test]
fn columns_take_the_date_and_timestamp_types() {
    let t = events();
    assert_eq!(t.schema().fields[1].data_type, tpt_strata::DataType::Date);
    assert_eq!(
        t.schema().fields[2].data_type,
        tpt_strata::DataType::Timestamp
    );
}

#[test]
fn sql_date_literal_compares_with_ordering() {
    let out = ctx_run("SELECT id FROM events WHERE day >= DATE '2026-01-01' ORDER BY id").unwrap();
    assert_eq!(ids(&out), vec![1, 2, 3]);
}

#[test]
fn a_plain_string_literal_is_read_as_a_date() {
    let out = ctx_run("SELECT id FROM events WHERE day = '2026-02-10' ORDER BY id").unwrap();
    assert_eq!(ids(&out), vec![2, 3]);
}

#[test]
fn timestamp_literal_and_between() {
    let out = ctx_run(
        "SELECT id FROM events WHERE at BETWEEN TIMESTAMP '2026-02-10 12:00:00' AND TIMESTAMP '2026-02-11' ORDER BY id",
    )
    .unwrap();
    assert_eq!(ids(&out), vec![2, 3]);
}

#[test]
fn builder_filters_with_a_typed_scalar_and_a_string() {
    let t = events();
    let typed = QueryBuilder::new(&t)
        .filter("day", "<", d("2026-01-06"))
        .unwrap()
        .execute()
        .unwrap();
    assert_eq!(typed.row_count(), 2);
    let text = QueryBuilder::new(&t)
        .filter("day", "<", Scalar::Str("2026-01-06".into()))
        .unwrap()
        .execute()
        .unwrap();
    assert_eq!(text.row_count(), 2);
}

#[test]
fn an_invalid_date_string_is_a_targeted_error() {
    let err = ctx_run("SELECT id FROM events WHERE day = '2026-02-30'").unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("'2026-02-30' is not a valid date for column 'day'"),
        "{text}"
    );
    assert!(text.contains("YYYY-MM-DD"), "{text}");
}

#[test]
fn an_invalid_date_literal_names_its_format() {
    let err = ctx_run("SELECT id FROM events WHERE day = DATE '02/10/2026'").unwrap_err();
    assert!(
        err.to_string()
            .contains("invalid date '02/10/2026'; use YYYY-MM-DD"),
        "{err}"
    );
}

#[test]
fn a_date_is_not_compared_with_a_number_or_another_type() {
    let err = ctx_run("SELECT id FROM events WHERE day = 5").unwrap_err();
    assert!(err.to_string().contains("date"), "{err}");
}

#[test]
fn group_by_a_date_and_order_by_it() {
    let out = ctx_run("SELECT day, COUNT(*) AS n FROM events GROUP BY day ORDER BY day").unwrap();
    assert_eq!(out.row_count(), 3);
    let col = &out.batches()[0].columns[0];
    let days: Vec<Option<Scalar>> = (0..col.len()).map(|i| col.get(i)).collect();
    assert_eq!(
        days,
        vec![
            Some(d("2025-12-31")),
            Some(d("2026-01-05")),
            Some(d("2026-02-10"))
        ]
    );
}

#[test]
fn min_and_max_work_on_dates_and_timestamps_but_sum_does_not() {
    let out = ctx_run("SELECT MIN(day) AS first, MAX(day) AS last FROM events").unwrap();
    let b = &out.batches()[0];
    assert_eq!(b.columns[0].get(0), Some(d("2025-12-31")));
    assert_eq!(b.columns[1].get(0), Some(d("2026-02-10")));
    let ts_out = ctx_run("SELECT MAX(at) AS latest FROM events").unwrap();
    assert_eq!(
        ts_out.batches()[0].columns[0].get(0),
        Some(ts("2026-02-10 23:59:59"))
    );
    let err = ctx_run("SELECT SUM(day) FROM events").unwrap_err();
    assert!(
        err.to_string().contains("requires a numeric column"),
        "{err}"
    );
}

#[test]
fn join_on_a_date_key() {
    let t = events();
    let days = Table::try_new(vec![
        Column::new("when", vec![d("2026-02-10"), d("2026-01-05")]),
        Column::new(
            "label",
            vec![Scalar::Str("feb".into()), Scalar::Str("jan".into())],
        ),
    ])
    .unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("events", &t);
    ctx.add_table("days", &days);
    let out = ctx
        .run("SELECT events.id, days.label FROM events JOIN days ON events.day = days.when ORDER BY id")
        .unwrap();
    assert_eq!(ids(&out), vec![1, 2, 3]);
}

#[test]
fn dates_print_as_iso_text() {
    let out = ctx_run("SELECT day, at FROM events WHERE id = 2").unwrap();
    let text = tpt_strata::format::render_table(&out);
    assert!(text.contains("2026-02-10"), "{text}");
    assert!(text.contains("2026-02-10 12:30:00.5"), "{text}");
}

#[test]
fn csv_text_stays_text_until_asked_for_a_date() {
    // CSV and JSON never infer dates from text; a DATE column stays text
    // unless the caller builds the table with Scalar::Date values.
    let csv = tpt_strata::csv::parse_csv("day\n2026-01-05\n").unwrap();
    assert_eq!(csv.schema().fields[0].data_type, tpt_strata::DataType::Utf8);
}
