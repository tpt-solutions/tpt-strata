//! DATE and TIMESTAMP survive a Parquet round trip as native types.

use tpt_strata::{Column, DataType, Scalar, Table};
use tpt_strata_parquet::{read_parquet, write_parquet};

fn d(s: &str) -> Scalar {
    Scalar::Date(tpt_strata::temporal::parse_date(s).unwrap())
}

fn ts(s: &str) -> Scalar {
    Scalar::Timestamp(tpt_strata::temporal::parse_timestamp(s).unwrap())
}

#[test]
fn dates_and_timestamps_round_trip_as_native_types() {
    let table = Table::try_new(vec![
        Column::new("day", vec![d("2026-10-13"), Scalar::Null, d("1969-12-31")]),
        Column::new(
            "at",
            vec![ts("2026-10-13 08:05:09.25"), ts("1970-01-01"), Scalar::Null],
        ),
    ])
    .unwrap();

    let path = std::env::temp_dir().join(format!("strata-temporal-{}.parquet", std::process::id()));
    write_parquet(&table, &path).unwrap();
    let back = read_parquet(&path).unwrap();
    let _ = std::fs::remove_file(&path);

    assert_eq!(back.schema().fields[0].data_type, DataType::Date);
    assert_eq!(back.schema().fields[1].data_type, DataType::Timestamp);
    let day = &back.batches()[0].columns[0];
    let at = &back.batches()[0].columns[1];
    assert_eq!(day.get(0), Some(d("2026-10-13")));
    assert_eq!(day.get(1), None);
    assert_eq!(day.get(2), Some(d("1969-12-31")));
    assert_eq!(at.get(0), Some(ts("2026-10-13 08:05:09.25")));
    assert_eq!(at.get(1), Some(ts("1970-01-01")));
    assert_eq!(at.get(2), None);
}
