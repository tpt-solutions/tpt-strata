//! Regression tests for the v1.0.2 correctness fixes. Each test pins one
//! behaviour that was broken before: string/bool ordering, aliases, join name
//! resolution, NaN and -0.0 keys, empty-batch paths, NULL semantics, exact
//! integer comparison, SUM typing, and parser/lexer gaps.

use tpt_strata::query::{join, AggFunc};
use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, QueryBuilder, QueryError, Scalar, Table};

fn s(v: &str) -> Scalar {
    Scalar::Str(v.to_string())
}

fn people() -> Table {
    Table::try_new(vec![
        Column::new("name", vec![s("alice"), s("bob"), s("carol"), s("nina")]),
        Column::new("dept", vec![s("eng"), s("eng"), s("ops"), s("ops")]),
        Column::new(
            "salary",
            vec![
                Scalar::F64(100.0),
                Scalar::F64(120.0),
                Scalar::F64(80.0),
                Scalar::F64(90.0),
            ],
        ),
        Column::new(
            "active",
            vec![
                Scalar::Bool(true),
                Scalar::Bool(false),
                Scalar::Bool(true),
                Scalar::Bool(true),
            ],
        ),
    ])
    .unwrap()
}

fn run(table: &Table, sql: &str) -> Result<Table, QueryError> {
    let mut ctx = SqlContext::new();
    ctx.add_table("people", table);
    ctx.run(sql)
}

fn col_values(t: &Table, col: usize) -> Vec<Option<Scalar>> {
    let mut out = Vec::new();
    for b in t.batches() {
        for r in 0..b.row_count() {
            out.push(b.columns[col].get(r));
        }
    }
    out
}

// --- string and boolean ordering (was always false) ---

#[test]
fn string_range_predicate_filters_by_order() {
    let t = people();
    let out = run(&t, "SELECT name FROM people WHERE name > 'b' ORDER BY name").unwrap();
    assert_eq!(
        col_values(&out, 0),
        vec![Some(s("bob")), Some(s("carol")), Some(s("nina"))]
    );
}

#[test]
fn boolean_range_predicate_orders_false_before_true() {
    let t = people();
    let out = run(&t, "SELECT name FROM people WHERE active >= FALSE").unwrap();
    assert_eq!(out.row_count(), 4);
    let out = run(&t, "SELECT name FROM people WHERE active > FALSE").unwrap();
    assert_eq!(out.row_count(), 3);
}

// --- aliases ---

#[test]
fn alias_names_the_output_column() {
    let t = people();
    let out = run(
        &t,
        "SELECT dept, SUM(salary) AS total FROM people GROUP BY dept ORDER BY dept",
    )
    .unwrap();
    assert_eq!(out.column_names(), vec!["dept", "total"]);
}

#[test]
fn order_by_alias_sorts_on_the_aliased_aggregate() {
    let t = people();
    let out = run(
        &t,
        "SELECT dept, SUM(salary) AS total FROM people GROUP BY dept ORDER BY total DESC",
    )
    .unwrap();
    assert_eq!(col_values(&out, 0), vec![Some(s("eng")), Some(s("ops"))]);
}

#[test]
fn alias_on_plain_column() {
    let t = people();
    let out = run(&t, "SELECT name AS who FROM people ORDER BY who LIMIT 1").unwrap();
    assert_eq!(out.column_names(), vec!["who"]);
    assert_eq!(col_values(&out, 0), vec![Some(s("alice"))]);
}

// --- joins: overlapping names, qualification, key side ---

fn orders_and_customers() -> (Table, Table) {
    let customers = Table::try_new(vec![
        Column::new("id", vec![Scalar::I64(1), Scalar::I64(2)]),
        Column::new("name", vec![s("ann"), s("ben")]),
    ])
    .unwrap();
    let orders = Table::try_new(vec![
        Column::new(
            "id",
            vec![Scalar::I64(10), Scalar::I64(11), Scalar::I64(12)],
        ),
        Column::new("cust", vec![Scalar::I64(2), Scalar::I64(1), Scalar::I64(2)]),
    ])
    .unwrap();
    (customers, orders)
}

#[test]
fn join_with_shared_column_name_resolves_qualified_references() {
    let (customers, orders) = orders_and_customers();
    let mut ctx = SqlContext::new();
    ctx.add_table("customers", &customers);
    ctx.add_table("orders", &orders);
    let out = ctx
        .run("SELECT orders.id, customers.name FROM orders JOIN customers ON orders.cust = customers.id ORDER BY orders.id")
        .unwrap();
    assert_eq!(out.column_names(), vec!["id", "name"]);
    assert_eq!(
        col_values(&out, 0),
        vec![
            Some(Scalar::I64(10)),
            Some(Scalar::I64(11)),
            Some(Scalar::I64(12))
        ]
    );
    assert_eq!(
        col_values(&out, 1),
        vec![Some(s("ben")), Some(s("ann")), Some(s("ben"))]
    );
}

#[test]
fn join_on_clause_accepts_either_side_order() {
    let (customers, orders) = orders_and_customers();
    let mut ctx = SqlContext::new();
    ctx.add_table("customers", &customers);
    ctx.add_table("orders", &orders);
    let forward = ctx
        .run("SELECT orders.id FROM orders JOIN customers ON orders.cust = customers.id")
        .unwrap();
    let reversed = ctx
        .run("SELECT orders.id FROM orders JOIN customers ON customers.id = orders.cust")
        .unwrap();
    assert_eq!(forward.row_count(), reversed.row_count());
    assert_eq!(forward.row_count(), 3);
}

#[test]
fn ambiguous_unqualified_join_column_is_an_error() {
    let (customers, orders) = orders_and_customers();
    let mut ctx = SqlContext::new();
    ctx.add_table("customers", &customers);
    ctx.add_table("orders", &orders);
    let err = ctx
        .run("SELECT id FROM orders JOIN customers ON orders.cust = customers.id")
        .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("'id' is ambiguous")
            && text.contains("orders.id")
            && text.contains("customers.id"),
        "unexpected: {text}"
    );
}

#[test]
fn join_keys_with_different_types_are_rejected() {
    let a = Table::try_new(vec![Column::new("k", vec![Scalar::I32(1)])]).unwrap();
    let b = Table::try_new(vec![Column::new("j", vec![Scalar::I64(1)])]).unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("a", &a);
    ctx.add_table("b", &b);
    let err = ctx.run("SELECT * FROM a JOIN b ON a.k = b.j").unwrap_err();
    assert!(err.to_string().contains("must have the same type"), "{err}");
}

#[test]
fn builder_join_rejects_duplicate_column_names() {
    let (customers, orders) = orders_and_customers();
    let err = join(&customers, &orders, "id", "cust").unwrap_err();
    assert!(err.to_string().contains("two columns named 'id'"), "{err}");
}

#[test]
fn self_join_is_rejected_with_a_clear_message() {
    let t = people();
    let mut ctx = SqlContext::new();
    ctx.add_table("people", &t);
    let err = ctx
        .run("SELECT * FROM people JOIN people ON people.name = people.name")
        .unwrap_err();
    assert!(
        err.to_string().contains("self-joins are not supported"),
        "{err}"
    );
}

// --- float keys: NaN and -0.0 ---

#[test]
fn nan_and_negative_zero_group_together() {
    let t = Table::try_new(vec![
        Column::new(
            "k",
            vec![
                Scalar::F64(f64::NAN),
                Scalar::F64(f64::NAN),
                Scalar::F64(0.0),
                Scalar::F64(-0.0),
            ],
        ),
        Column::new(
            "v",
            vec![
                Scalar::I64(1),
                Scalar::I64(2),
                Scalar::I64(4),
                Scalar::I64(8),
            ],
        ),
    ])
    .unwrap();
    let out = QueryBuilder::new(&t)
        .group_by(vec!["k"])
        .unwrap()
        .aggregate("v", AggFunc::Sum)
        .unwrap()
        .execute()
        .unwrap();
    // Two groups: NaN (1+2) and zero (4+8). Before the fix NaN split into two and -0.0 apart from 0.0.
    assert_eq!(out.row_count(), 2);
    let mut sums: Vec<i64> = col_values(&out, 1)
        .into_iter()
        .map(|v| match v {
            Some(Scalar::I64(n)) => n,
            other => panic!("expected an i64 sum, found {other:?}"),
        })
        .collect();
    sums.sort_unstable();
    assert_eq!(sums, vec![3, 12]);
}

// --- empty batches ---

#[test]
fn limit_zero_then_order_and_project_does_not_panic() {
    let t = people();
    let out = run(
        &t,
        "SELECT name, salary FROM people ORDER BY salary LIMIT 0",
    )
    .unwrap();
    assert_eq!(out.row_count(), 0);
    assert_eq!(out.column_names(), vec!["name", "salary"]);
    assert!(!out.batches().is_empty());
}

#[test]
fn join_with_no_matches_yields_typed_empty_result() {
    let (customers, orders) = orders_and_customers();
    let mut ctx = SqlContext::new();
    ctx.add_table("customers", &customers);
    ctx.add_table("orders", &orders);
    let out = ctx
        .run("SELECT orders.id, customers.name FROM orders JOIN customers ON orders.cust = customers.id WHERE customers.name = 'nobody'")
        .unwrap();
    assert_eq!(out.row_count(), 0);
    assert_eq!(out.column_names(), vec!["id", "name"]);
}

// --- NULL semantics ---

#[test]
fn comparisons_with_null_match_nothing() {
    let t = Table::try_new(vec![Column::new(
        "v",
        vec![Scalar::I64(1), Scalar::Null, Scalar::I64(3)],
    )])
    .unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    assert_eq!(
        ctx.run("SELECT v FROM t WHERE v != NULL")
            .unwrap()
            .row_count(),
        0
    );
    assert_eq!(
        ctx.run("SELECT v FROM t WHERE v = NULL")
            .unwrap()
            .row_count(),
        0
    );
}

#[test]
fn is_null_and_is_not_null_select_nulls() {
    let t = Table::try_new(vec![Column::new(
        "v",
        vec![Scalar::I64(1), Scalar::Null, Scalar::I64(3)],
    )])
    .unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    assert_eq!(
        ctx.run("SELECT v FROM t WHERE v IS NULL")
            .unwrap()
            .row_count(),
        1
    );
    assert_eq!(
        ctx.run("SELECT v FROM t WHERE v IS NOT NULL")
            .unwrap()
            .row_count(),
        2
    );
}

// --- numeric comparison: exact, mixed literals ---

#[test]
fn fractional_float_literal_compares_against_integer_column() {
    let t = Table::try_new(vec![Column::new(
        "id",
        vec![Scalar::I32(1), Scalar::I32(2), Scalar::I32(3)],
    )])
    .unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx.run("SELECT id FROM t WHERE id > 2.5").unwrap();
    assert_eq!(col_values(&out, 0), vec![Some(Scalar::I32(3))]);
}

#[test]
fn large_integers_compare_exactly_not_through_f64() {
    // 2^53 and 2^53 + 1 are indistinguishable as f64, so the old code matched both.
    let big = 9_007_199_254_740_992i64;
    let t = Table::try_new(vec![Column::new(
        "v",
        vec![Scalar::I64(big), Scalar::I64(big + 1)],
    )])
    .unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx
        .run(&format!("SELECT v FROM t WHERE v > {big}"))
        .unwrap();
    assert_eq!(col_values(&out, 0), vec![Some(Scalar::I64(big + 1))]);
}

#[test]
fn sum_over_integers_is_exact_int64() {
    let t = Table::try_new(vec![Column::new(
        "v",
        vec![Scalar::I64(9_007_199_254_740_993), Scalar::I64(1)],
    )])
    .unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx.run("SELECT SUM(v) AS total FROM t").unwrap();
    assert_eq!(
        out.schema().fields[0].data_type,
        tpt_strata::DataType::Int64
    );
    assert_eq!(
        col_values(&out, 0),
        vec![Some(Scalar::I64(9_007_199_254_740_994))]
    );
}

#[test]
fn sum_overflow_is_an_error_not_a_wrap() {
    let t = Table::try_new(vec![Column::new(
        "v",
        vec![Scalar::I64(i64::MAX), Scalar::I64(1)],
    )])
    .unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let err = ctx.run("SELECT SUM(v) FROM t").unwrap_err();
    assert!(err.to_string().contains("overflowed i64"), "{err}");
}

// --- qualified ORDER BY and SELECT * mixing ---

#[test]
fn order_by_qualified_column() {
    let t = people();
    let out = run(
        &t,
        "SELECT name FROM people ORDER BY people.name DESC LIMIT 1",
    )
    .unwrap();
    assert_eq!(col_values(&out, 0), vec![Some(s("nina"))]);
}

#[test]
fn wildcard_mixed_with_a_column_keeps_both() {
    let t = people();
    let out = run(&t, "SELECT *, name AS n FROM people LIMIT 1").unwrap();
    assert_eq!(out.column_names().len(), 5);
    assert_eq!(out.column_names().last().copied(), Some("n"));
}

#[test]
fn quoted_identifier_with_dot_is_one_column() {
    let t = Table::try_new(vec![Column::new("http.status", vec![Scalar::I64(200)])]).unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx.run("SELECT \"http.status\" FROM t").unwrap();
    assert_eq!(out.column_names(), vec!["http.status"]);
    assert_eq!(col_values(&out, 0), vec![Some(Scalar::I64(200))]);
}

#[test]
fn unknown_table_qualifier_is_reported() {
    let t = people();
    let err = run(&t, "SELECT nosuch.name FROM people").unwrap_err();
    assert!(
        err.to_string().contains("No column named 'nosuch.name'"),
        "{err}"
    );
}

// --- LIKE, BETWEEN, OFFSET, multi-key ORDER BY ---

#[test]
fn like_supports_percent_and_underscore() {
    let t = people();
    // Names with an 'o' second-from-last: bob and carol.
    let out = run(&t, "SELECT name FROM people WHERE name LIKE '%o_'").unwrap();
    assert_eq!(out.row_count(), 2);
}

#[test]
fn like_patterns_match_expected_rows() {
    let t = people();
    assert_eq!(
        run(&t, "SELECT name FROM people WHERE name LIKE 'c%'")
            .unwrap()
            .row_count(),
        1
    );
    assert_eq!(
        run(&t, "SELECT name FROM people WHERE name LIKE '_ob'")
            .unwrap()
            .row_count(),
        1
    );
    assert_eq!(
        run(&t, "SELECT name FROM people WHERE name NOT LIKE '%a%'")
            .unwrap()
            .row_count(),
        1
    );
}

#[test]
fn between_is_inclusive() {
    let t = people();
    let out = run(
        &t,
        "SELECT name FROM people WHERE salary BETWEEN 80 AND 100 ORDER BY name",
    )
    .unwrap();
    assert_eq!(
        col_values(&out, 0),
        vec![Some(s("alice")), Some(s("carol")), Some(s("nina"))]
    );
}

#[test]
fn offset_skips_rows_after_order() {
    let t = people();
    let out = run(&t, "SELECT name FROM people ORDER BY name LIMIT 2 OFFSET 1").unwrap();
    assert_eq!(col_values(&out, 0), vec![Some(s("bob")), Some(s("carol"))]);
}

#[test]
fn multi_column_order_by() {
    let t = people();
    let out = run(&t, "SELECT name FROM people ORDER BY dept DESC, name ASC").unwrap();
    assert_eq!(
        col_values(&out, 0),
        vec![
            Some(s("carol")),
            Some(s("nina")),
            Some(s("alice")),
            Some(s("bob"))
        ]
    );
}

#[test]
fn distinct_removes_duplicate_rows() {
    let t = people();
    let out = run(&t, "SELECT DISTINCT dept FROM people ORDER BY dept").unwrap();
    assert_eq!(col_values(&out, 0), vec![Some(s("eng")), Some(s("ops"))]);
}

#[test]
fn count_distinct_is_rejected_with_a_targeted_message() {
    let t = people();
    let err = run(&t, "SELECT COUNT(DISTINCT dept) FROM people").unwrap_err();
    assert!(
        err.to_string()
            .contains("'COUNT(DISTINCT ...)' is not supported"),
        "{err}"
    );
}

#[test]
fn aggregate_in_where_is_rejected() {
    let t = people();
    let err = run(&t, "SELECT name FROM people WHERE SUM(salary) > 1").unwrap_err();
    assert!(err.to_string().contains("use HAVING"), "{err}");
}

// --- lexer: comments, exponents, escaped quotes ---

#[test]
fn line_and_block_comments_are_ignored() {
    let t = people();
    let out = run(
        &t,
        "SELECT name -- trailing comment\nFROM people /* block\ncomment */ WHERE salary > 1e2 ORDER BY name",
    )
    .unwrap();
    assert_eq!(col_values(&out, 0), vec![Some(s("bob"))]);
}

#[test]
fn doubled_quote_in_identifier_is_one_quote() {
    let t = Table::try_new(vec![Column::new("say \"hi\"", vec![Scalar::I64(1)])]).unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("t", &t);
    let out = ctx.run("SELECT \"say \"\"hi\"\"\" FROM t").unwrap();
    assert_eq!(out.column_names(), vec!["say \"hi\""]);
}

// --- builder parity ---

#[test]
fn builder_count_all_counts_rows_including_nulls() {
    let t = Table::try_new(vec![
        Column::new("g", vec![s("a"), s("a"), s("b")]),
        Column::new("v", vec![Scalar::I64(1), Scalar::Null, Scalar::I64(2)]),
    ])
    .unwrap();
    let out = QueryBuilder::new(&t)
        .group_by(vec!["g"])
        .unwrap()
        .count_all()
        .execute()
        .unwrap();
    assert_eq!(out.column_names(), vec!["g", "COUNT(*)"]);
    assert_eq!(
        col_values(&out, 1),
        vec![Some(Scalar::I64(2)), Some(Scalar::I64(1))]
    );
}

#[test]
fn builder_project_after_aggregate_selects_and_orders_columns() {
    let t = people();
    let out = QueryBuilder::new(&t)
        .group_by(vec!["dept"])
        .unwrap()
        .aggregate("salary", AggFunc::Sum)
        .unwrap()
        .project(vec!["SUM(salary)", "dept"])
        .unwrap()
        .execute()
        .unwrap();
    assert_eq!(out.column_names(), vec!["SUM(salary)", "dept"]);
}

#[test]
fn builder_accepts_integer_literal_against_float_column() {
    let t = people();
    let out = QueryBuilder::new(&t)
        .filter("salary", ">", Scalar::I64(100))
        .unwrap()
        .execute()
        .unwrap();
    assert_eq!(out.row_count(), 1);
}

// --- table construction and concat ---

#[test]
fn from_rows_builds_a_table() {
    let t = Table::from_rows(
        &["city", "pop"],
        vec![
            vec![s("nyc"), Scalar::I64(8)],
            vec![s("sfo"), Scalar::I64(1)],
        ],
    )
    .unwrap();
    assert_eq!(t.row_count(), 2);
    assert_eq!(t.column_names(), vec!["city", "pop"]);
}

#[test]
fn from_rows_rejects_ragged_rows() {
    let err = Table::from_rows(&["a", "b"], vec![vec![Scalar::I64(1)]]).unwrap_err();
    assert!(err.to_string().contains("1 values but 2 columns"), "{err}");
}

#[test]
fn concat_accepts_differing_nullability() {
    let a = Table::try_new(vec![Column::new("v", vec![Scalar::I64(1)])]).unwrap();
    let b = Table::try_new(vec![Column::new("v", vec![Scalar::I64(2), Scalar::Null])]).unwrap();
    assert!(!a.schema().fields[0].nullable);
    let both = a.concat(&b).unwrap();
    assert_eq!(both.row_count(), 3);
    assert!(both.schema().fields[0].nullable);
}

// --- explain and check ---

#[test]
fn explain_shows_the_operator_tree() {
    let t = people();
    let mut ctx = SqlContext::new();
    ctx.add_table("people", &t);
    let text = ctx
        .explain(
            "SELECT dept, SUM(salary) FROM people WHERE salary > 1 GROUP BY dept ORDER BY dept",
        )
        .unwrap();
    assert!(text.contains("Sort"), "{text}");
    assert!(text.contains("Aggregate"), "{text}");
    assert!(text.contains("Filter"), "{text}");
    assert!(text.contains("Scan rows=4"), "{text}");
}

#[test]
fn check_binds_without_running() {
    let t = people();
    let mut ctx = SqlContext::new();
    ctx.add_table("people", &t);
    ctx.check("SELECT name FROM people WHERE name = 'x'")
        .unwrap();
    assert!(ctx.check("SELECT nope FROM people").is_err());
}

// --- diagnostics as data ---

#[test]
fn error_kind_is_stable_and_machine_readable() {
    let t = people();
    let missing = run(&t, "SELECT nope FROM people").unwrap_err();
    assert_eq!(missing.kind(), "missing-column");
    let sql = run(&t, "SELECT name FROM people WHERE name =").unwrap_err();
    assert_eq!(sql.kind(), "sql");
    let unsupported = run(&t, "SELECT name FROM people WHERE name IN ('a')").unwrap_err();
    assert_eq!(unsupported.kind(), "sql");
}

// --- rendering ---

#[test]
fn zero_row_result_still_shows_its_header() {
    let t = people();
    let out = run(&t, "SELECT name FROM people WHERE name = 'nobody'").unwrap();
    let text = tpt_strata::format::render_table(&out);
    assert!(text.contains("| name"), "{text}");
    assert!(text.contains("(0 rows)"), "{text}");
}

#[test]
fn unicode_values_align_by_character_count() {
    let t = Table::try_new(vec![Column::new("city", vec![s("zürich"), s("ny")])]).unwrap();
    let text = tpt_strata::format::render_table(&t);
    let widths: Vec<usize> = text
        .lines()
        .filter(|l| l.starts_with('|'))
        .map(|l| l.chars().count())
        .collect();
    assert!(widths.windows(2).all(|w| w[0] == w[1]), "{text}");
}
