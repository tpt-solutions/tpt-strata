use tpt_strata::csv::{parse_csv, to_csv};
use tpt_strata::{DataType, Scalar};

fn value(t: &tpt_strata::Table, col: usize, row: usize) -> Option<Scalar> {
    t.batches()[0].columns[col].get(row)
}

#[test]
fn infers_types_in_order_of_narrowness() {
    let t = parse_csv("i,f,b,s\n1,1.5,true,x\n2,2,FALSE,y\n").unwrap();
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
    assert_eq!(value(&t, 1, 1), Some(Scalar::F64(2.0)));
}

#[test]
fn quoted_fields_keep_commas_quotes_and_newlines() {
    let t = parse_csv("name,note\n\"Smith, J\",\"said \"\"hi\"\"\"\n\"two\nlines\",x\n").unwrap();
    assert_eq!(t.row_count(), 2);
    assert_eq!(value(&t, 0, 0), Some(Scalar::Str("Smith, J".into())));
    assert_eq!(value(&t, 1, 0), Some(Scalar::Str("said \"hi\"".into())));
    assert_eq!(value(&t, 0, 1), Some(Scalar::Str("two\nlines".into())));
}

#[test]
fn empty_unquoted_is_null_and_quoted_empty_is_a_string() {
    let t = parse_csv("a,b\n1,\n,\"\"\n").unwrap();
    assert_eq!(value(&t, 1, 0), None);
    assert_eq!(value(&t, 0, 1), None);
    assert_eq!(value(&t, 1, 1), Some(Scalar::Str(String::new())));
    assert!(t.schema().fields[1].nullable);
}

#[test]
fn all_null_column_is_a_string_column() {
    let t = parse_csv("a,b\n1,\n2,\n").unwrap();
    assert_eq!(t.schema().fields[1].data_type, DataType::Utf8);
    assert_eq!(value(&t, 1, 0), None);
}

#[test]
fn crlf_and_blank_lines_are_handled() {
    let t = parse_csv("a,b\r\n1,2\r\n\r\n3,4\r\n").unwrap();
    assert_eq!(t.row_count(), 2);
    assert_eq!(value(&t, 0, 1), Some(Scalar::I64(3)));
}

#[test]
fn ragged_record_is_reported_with_its_line() {
    let err = parse_csv("a,b\n1,2\n3\n").unwrap_err();
    assert!(
        err.to_string()
            .contains("record 3 has 1 fields but the header has 2"),
        "{err}"
    );
}

#[test]
fn unterminated_quote_is_an_error() {
    assert!(parse_csv("a\n\"oops\n").is_err());
}

#[test]
fn empty_input_needs_a_header() {
    assert!(parse_csv("")
        .unwrap_err()
        .to_string()
        .contains("header row"));
}

#[test]
fn to_csv_round_trips_through_parse() {
    let original = parse_csv("name,score\n\"a, b\",1\nplain,\n").unwrap();
    let text = to_csv(&original);
    let again = parse_csv(&text).unwrap();
    assert_eq!(again.row_count(), 2);
    assert_eq!(value(&again, 0, 0), Some(Scalar::Str("a, b".into())));
    assert_eq!(value(&again, 1, 1), None);
}
