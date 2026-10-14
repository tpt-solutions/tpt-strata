//! LIKE: the byte-wise matcher on multi-byte text, and the string-column
//! filter path, including NULL handling for LIKE and NOT LIKE.

use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, Scalar, Table};

fn like(text: &str, pattern: &str) -> bool {
    // Through SQL, so the same path the engine uses for string columns.
    let t = Table::try_new(vec![Column::new("s", vec![Scalar::Str(text.to_string())])]).unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let sql = format!("SELECT s FROM t WHERE s LIKE '{pattern}'");
    ctx.run(&sql).unwrap().row_count() == 1
}

#[test]
fn underscore_matches_one_character_not_one_byte() {
    assert!(like("café", "caf_"));
    assert!(!like("café", "caf__"));
    assert!(like("naïve", "na_ve"));
    assert!(like("über", "_ber"));
}

#[test]
fn percent_absorbs_whole_characters() {
    assert!(like("über", "ü%"));
    assert!(like("über", "%ber"));
    assert!(like("日本語", "%語"));
    assert!(like("日本語", "日%"));
    assert!(!like("日本語", "%本"));
}

#[test]
fn ascii_patterns_behave_as_before() {
    assert!(like("abc", "%"));
    assert!(like("", "%"));
    assert!(!like("", "_"));
    assert!(like("abc", "a%c"));
    assert!(like("abbc", "a%b%c"));
    assert!(!like("abc", "abcd"));
    assert!(like("abc", "abc"));
}

#[test]
fn null_never_matches_like_or_not_like() {
    let t = Table::try_new(vec![Column::new(
        "s",
        vec![
            Scalar::Str("north".into()),
            Scalar::Null,
            Scalar::Str("coastal".into()),
        ],
    )])
    .unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let like_rows = ctx.run("SELECT s FROM t WHERE s LIKE 'c%'").unwrap();
    assert_eq!(like_rows.row_count(), 1);
    // The NULL row matches neither LIKE nor NOT LIKE.
    let not_like = ctx.run("SELECT s FROM t WHERE s NOT LIKE 'c%'").unwrap();
    assert_eq!(not_like.row_count(), 1);
}

#[test]
fn common_shapes_agree_with_the_general_rules() {
    // Exact, prefix, suffix and substring patterns take the direct string paths.
    let cases = [
        ("abc", "abc", true),
        ("abc", "ab", false),
        ("abc", "ab%", true),
        ("abc", "b%", false),
        ("abc", "%bc", true),
        ("abc", "%ab", false),
        ("abc", "%b%", true),
        ("abc", "%d%", false),
        ("", "%", true),
        ("é", "%é%", true),
        ("naïve", "%ï%", true),
        ("abc", "a%c", true),
    ];
    for (text, pattern, expect) in cases {
        assert_eq!(like(text, pattern), expect, "{text} LIKE {pattern}");
    }
}
