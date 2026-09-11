//! Deliberately trigger each `QueryError` variant and print it.
//!
//! Diagnostics are a headline feature: every error names the caller's own
//! problem in the caller's own terms. Run with
//! `cargo run -p tpt-strata --example diagnostics`.

use tpt_strata::{sql::SqlContext, AggFunc, Column, QueryBuilder, QueryError, Scalar, Table};

fn main() {
    let employees = Table::try_new(vec![
        Column::new(
            "department",
            vec![Scalar::Str("eng".into()), Scalar::Str("sales".into())],
        ),
        Column::new("salary", vec![Scalar::F64(100000.0), Scalar::F64(80000.0)]),
    ])
    .unwrap();

    // Missing column — names the column and lists the schema's real columns.
    let err = employees.column_index("nonexistent").unwrap_err();
    print_case("missing column (builder)", &err);

    // Type mismatch — names the column, expected type, actual type.
    let err = QueryBuilder::new(&employees)
        .filter("department", ">", Scalar::I32(42))
        .unwrap_err();
    print_case("type mismatch (builder)", &err);

    // Unsupported operation — SUM over a string column.
    let err = QueryBuilder::new(&employees)
        .aggregate("department", AggFunc::Sum)
        .unwrap()
        .execute()
        .unwrap_err();
    print_case("unsupported operation (builder)", &err);

    // The same diagnosis through the SQL surface.
    let mut ctx = SqlContext::new();
    ctx.add_table("employees", Box::leak(Box::new(employees)));
    let err = ctx.run("SELECT nonexistent FROM employees").unwrap_err();
    print_case("sql: missing column (with did-you-mean)", &err);
    let err = ctx
        .run("SELECT department FROM employees WHERE salary = 'abc'")
        .unwrap_err();
    print_case("sql: type mismatch", &err);
    let err = ctx
        .run("SELECT * FROM employees WHERE department IS NULL")
        .unwrap_err();
    print_case("sql: deliberate v1.1 scope gap", &err);
}

fn print_case(label: &str, err: &QueryError) {
    println!("== {label} ==");
    println!("{err}");
    println!();
}
