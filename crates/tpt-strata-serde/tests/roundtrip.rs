use serde::{Deserialize, Serialize};
use tpt_strata::sql::SqlContext;
use tpt_strata::{DataType, Table};
use tpt_strata_serde::{from_table, to_table};

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
struct Order {
    region: String,
    quantity: i64,
    price: Option<f64>,
    gift: bool,
}

fn orders() -> Vec<Order> {
    vec![
        Order {
            region: "north".into(),
            quantity: 4,
            price: Some(2.5),
            gift: false,
        },
        Order {
            region: "south".into(),
            quantity: 1,
            price: None,
            gift: true,
        },
        Order {
            region: "north".into(),
            quantity: 6,
            price: Some(3.0),
            gift: false,
        },
    ]
}

#[test]
fn round_trips_a_flat_struct() {
    let table = to_table(&orders()).unwrap();
    assert_eq!(
        table.column_names(),
        vec!["region", "quantity", "price", "gift"]
    );
    assert_eq!(table.row_count(), 3);
    let back: Vec<Order> = from_table(&table).unwrap();
    assert_eq!(back, orders());
}

#[test]
fn infers_column_types_from_rust_types() {
    let table = to_table(&orders()).unwrap();
    let dts: Vec<DataType> = table.schema().fields.iter().map(|f| f.data_type).collect();
    assert_eq!(
        dts,
        vec![
            DataType::Utf8,
            DataType::Int64,
            DataType::Float64,
            DataType::Boolean
        ]
    );
    assert!(table.schema().fields[2].nullable);
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct SmallInts {
    a: i8,
    b: u16,
    c: u32,
}

#[test]
fn small_integers_widen_to_native_types() {
    let rows = vec![SmallInts {
        a: -3,
        b: 60000,
        c: 4_000_000_000,
    }];
    let table = to_table(&rows).unwrap();
    assert_eq!(table.schema().fields[0].data_type, DataType::Int32);
    assert_eq!(table.schema().fields[1].data_type, DataType::Int32);
    assert_eq!(table.schema().fields[2].data_type, DataType::Int64);
    let back: Vec<SmallInts> = from_table(&table).unwrap();
    assert_eq!(back, rows);
}

#[test]
fn query_results_deserialize_into_a_smaller_struct() {
    let table = to_table(&orders()).unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("orders", &table);
    let totals = ctx
        .run("SELECT region, SUM(quantity) AS quantity FROM orders GROUP BY region ORDER BY region")
        .unwrap();

    #[derive(Deserialize, Debug, PartialEq)]
    struct Total {
        region: String,
        quantity: i64,
    }
    let rows: Vec<Total> = from_table(&totals).unwrap();
    assert_eq!(
        rows,
        vec![
            Total {
                region: "north".into(),
                quantity: 10
            },
            Total {
                region: "south".into(),
                quantity: 1
            },
        ]
    );
}

#[test]
fn missing_option_becomes_none_and_missing_required_field_is_named() {
    #[derive(Deserialize, Debug, PartialEq)]
    struct Partial {
        region: String,
        price: Option<f64>,
        note: Option<String>,
    }
    let table = to_table(&orders()).unwrap();
    let rows: Vec<Partial> = from_table(&table).unwrap();
    assert_eq!(rows[0].note, None);

    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    struct NeedsNote {
        region: String,
        note: String,
    }
    let err = from_table::<NeedsNote>(&table).unwrap_err();
    assert!(err.to_string().contains("missing field `note`"), "{err}");
}

#[test]
fn nested_structs_and_lists_are_rejected_with_the_field_name() {
    #[derive(Serialize)]
    struct Inner {
        x: i64,
    }
    #[derive(Serialize)]
    struct Outer {
        tags: Vec<String>,
    }
    #[derive(Serialize)]
    struct Nested {
        inner: Inner,
    }
    let err = to_table(&[Outer {
        tags: vec!["a".into()],
    }])
    .unwrap_err();
    assert!(err.to_string().contains("field 'tags' is a list"), "{err}");
    let err = to_table(&[Nested {
        inner: Inner { x: 1 },
    }])
    .unwrap_err();
    assert!(
        err.to_string().contains("field 'inner' is a nested struct"),
        "{err}"
    );
}

#[test]
fn empty_input_is_an_error_not_an_empty_table() {
    let rows: Vec<Order> = Vec::new();
    let err = to_table(&rows).unwrap_err();
    assert!(err.to_string().contains("zero rows"), "{err}");
}

#[test]
fn unsigned_values_beyond_i64_are_rejected() {
    #[derive(Serialize)]
    struct Big {
        v: u64,
    }
    let err = to_table(&[Big { v: u64::MAX }]).unwrap_err();
    assert!(err.to_string().contains("does not fit in i64"), "{err}");
}

#[test]
fn table_built_by_hand_deserializes() {
    use tpt_strata::{Column, Scalar};
    let t = Table::try_new(vec![
        Column::new("region", vec![Scalar::Str("east".into())]),
        Column::new("quantity", vec![Scalar::I64(9)]),
        Column::new("price", vec![Scalar::F64(1.25)]),
        Column::new("gift", vec![Scalar::Bool(false)]),
    ])
    .unwrap();
    let rows: Vec<Order> = from_table(&t).unwrap();
    assert_eq!(rows[0].region, "east");
    assert_eq!(rows[0].price, Some(1.25));
}
