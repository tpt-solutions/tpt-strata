use tpt_strata::json::parse_json;
use tpt_strata::{DataType, Scalar};

fn value(t: &tpt_strata::Table, col: usize, row: usize) -> Option<Scalar> {
    t.batches()[0].columns[col].get(row)
}

#[test]
fn reads_an_array_of_objects_with_inferred_types() {
    let t = parse_json(
        r#"[{"id": 1, "score": 2.5, "ok": true, "name": "ann"},
                           {"id": 2, "score": 3, "ok": false, "name": "ben"}]"#,
    )
    .unwrap();
    let dts: Vec<DataType> = t.schema().fields.iter().map(|f| f.data_type).collect();
    assert_eq!(
        dts,
        vec![
            DataType::Int64,
            DataType::Float64,
            DataType::Boolean,
            DataType::Utf8
        ]
    );
    assert_eq!(value(&t, 0, 1), Some(Scalar::I64(2)));
    assert_eq!(value(&t, 1, 1), Some(Scalar::F64(3.0)));
    assert_eq!(value(&t, 3, 0), Some(Scalar::Str("ann".into())));
}

#[test]
fn newline_delimited_objects_and_a_single_object() {
    let lines = parse_json("{\"a\": 1}\n{\"a\": 2}\n").unwrap();
    assert_eq!(lines.row_count(), 2);
    let one = parse_json(r#"{"a": "x"}"#).unwrap();
    assert_eq!(one.row_count(), 1);
}

#[test]
fn missing_keys_and_nulls_become_null_and_keys_keep_first_seen_order() {
    let t = parse_json(r#"[{"a": 1}, {"b": "x", "a": null}, {"a": 3, "c": true}]"#).unwrap();
    assert_eq!(t.column_names(), vec!["a", "b", "c"]);
    assert_eq!(value(&t, 0, 1), None);
    assert_eq!(value(&t, 1, 0), None);
    assert_eq!(value(&t, 2, 0), None);
    assert_eq!(value(&t, 0, 2), Some(Scalar::I64(3)));
}

#[test]
fn string_escapes_and_surrogate_pairs_decode() {
    let t = parse_json(r#"[{"s": "tab\there \"q\" \u00e9 \ud83d\ude00"}]"#).unwrap();
    assert_eq!(
        value(&t, 0, 0),
        Some(Scalar::Str("tab\there \"q\" é 😀".into()))
    );
}

#[test]
fn mixed_numbers_and_text_fall_back_to_text() {
    let t = parse_json(r#"[{"v": 1}, {"v": "two"}]"#).unwrap();
    assert_eq!(t.schema().fields[0].data_type, DataType::Utf8);
    assert_eq!(value(&t, 0, 0), Some(Scalar::Str("1".into())));
}

#[test]
fn nested_values_are_rejected_with_the_column_name() {
    let err = parse_json(r#"[{"tags": ["a", "b"]}]"#).unwrap_err();
    assert!(
        err.to_string()
            .contains("column 'tags' holds a nested array"),
        "{err}"
    );
}

#[test]
fn malformed_input_is_an_import_error_with_a_line_number() {
    let err = parse_json("[{\"a\": 1},\n {\"a\" 2}]").unwrap_err();
    assert_eq!(err.kind(), "import");
    assert!(err.to_string().contains("line 2"), "{err}");
    assert!(parse_json("[]").is_err());
}
