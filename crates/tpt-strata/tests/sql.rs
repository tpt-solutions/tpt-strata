use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, Scalar, Table};

fn dept_table() -> Table {
    Table::try_new(vec![
        Column::new(
            "dept",
            vec![
                Scalar::Str("eng".into()),
                Scalar::Str("eng".into()),
                Scalar::Str("sales".into()),
                Scalar::Str("sales".into()),
            ],
        ),
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
            "years",
            vec![
                Scalar::I32(5),
                Scalar::I32(3),
                Scalar::I32(6),
                Scalar::I32(1),
            ],
        ),
        Column::new(
            "city",
            vec![
                Scalar::Str("nyc".into()),
                Scalar::Str("sfo".into()),
                Scalar::Str("nyc".into()),
                Scalar::Str("aus".into()),
            ],
        ),
    ])
    .unwrap()
}

fn city_table() -> Table {
    Table::try_new(vec![
        Column::new(
            "name",
            vec![
                Scalar::Str("nyc".into()),
                Scalar::Str("sfo".into()),
                Scalar::Str("aus".into()),
            ],
        ),
        Column::new(
            "population",
            vec![
                Scalar::I64(8_000_000),
                Scalar::I64(800_000),
                Scalar::I64(1_000_000),
            ],
        ),
    ])
    .unwrap()
}

fn ctx() -> SqlContext<'static> {
    let mut c = SqlContext::new();
    c.add_table("employees", Box::leak(Box::new(dept_table())));
    c.add_table("cities", Box::leak(Box::new(city_table())));
    c
}

fn all_scalars(table: &Table, col: usize) -> Vec<Scalar> {
    table
        .batches()
        .iter()
        .flat_map(|b| {
            (0..b.row_count())
                .map(|r| b.columns[col].get(r).unwrap())
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn select_star() {
    let out = ctx().run("SELECT * FROM employees").unwrap();
    assert_eq!(out.row_count(), 4);
    assert_eq!(out.column_names(), vec!["dept", "salary", "years", "city"]);
}

#[test]
fn select_project_columns() {
    let out = ctx().run("SELECT dept, salary FROM employees").unwrap();
    assert_eq!(out.column_names(), vec!["dept", "salary"]);
    let salaries = all_scalars(&out, 1);
    assert_eq!(salaries[0], Scalar::F64(100.0));
}

#[test]
fn select_where_object() {
    let out = ctx()
        .run("SELECT dept, salary FROM employees WHERE salary > 90")
        .unwrap();
    let salaries = all_scalars(&out, 1);
    assert_eq!(salaries, vec![Scalar::F64(100.0), Scalar::F64(120.0)]);
}

#[test]
fn select_where_and_chain() {
    let out = ctx()
        .run("SELECT dept FROM employees WHERE city = 'nyc' AND salary > 90")
        .unwrap();
    let depts = all_scalars(&out, 0);
    assert_eq!(depts, vec![Scalar::Str("eng".into())]);
}

#[test]
fn select_where_scope_notequiv() {
    let out = ctx()
        .run("SELECT dept FROM employees WHERE city <> 'nyc'")
        .unwrap();
    assert_eq!(out.row_count(), 2);
}

#[test]
fn select_where_numeric_type_coercion() {
    let out = ctx()
        .run("SELECT dept FROM employees WHERE years > 4")
        .unwrap();
    let depts = all_scalars(&out, 0);
    assert_eq!(
        depts,
        vec![Scalar::Str("eng".into()), Scalar::Str("sales".into())]
    );
}

#[test]
fn select_group_by_sum() {
    let out = ctx()
        .run("SELECT dept, SUM(salary) FROM employees GROUP BY dept")
        .unwrap();
    assert_eq!(out.row_count(), 2);
    assert_eq!(
        out.column_names(),
        vec!["dept".to_string(), "SUM(salary)".to_string()]
    );
    let mut sums = std::collections::HashMap::new();
    for i in 0..out.row_count() {
        sums.insert(
            out.batches()[0].columns[0].get(i).unwrap(),
            out.batches()[0].columns[1].get(i).unwrap(),
        );
    }
    assert_eq!(sums[&Scalar::Str("eng".into())], Scalar::F64(220.0));
    assert_eq!(sums[&Scalar::Str("sales".into())], Scalar::F64(170.0));
}

#[test]
fn select_group_by_avg_min_max_count() {
    let out = ctx()
        .run(
            "SELECT dept, AVG(salary), MIN(salary), MAX(salary), COUNT(dept) FROM employees GROUP BY dept",
        )
        .unwrap();
    assert_eq!(out.row_count(), 2);
    assert_eq!(
        out.column_names(),
        vec![
            "dept".to_string(),
            "AVG(salary)".to_string(),
            "MIN(salary)".to_string(),
            "MAX(salary)".to_string(),
            "COUNT(dept)".to_string(),
        ]
    );
    let mut rows = std::collections::HashMap::new();
    for i in 0..out.row_count() {
        let batch = &out.batches()[0];
        rows.insert(
            batch.columns[0].get(i).unwrap(),
            vec![
                batch.columns[1].get(i).unwrap(),
                batch.columns[2].get(i).unwrap(),
                batch.columns[3].get(i).unwrap(),
                batch.columns[4].get(i).unwrap(),
            ],
        );
    }
    let eng = rows[&Scalar::Str("eng".into())].clone();
    assert_eq!(
        eng,
        vec![
            Scalar::F64(110.0),
            Scalar::F64(100.0),
            Scalar::F64(120.0),
            Scalar::I64(2)
        ]
    );
    let sales = rows[&Scalar::Str("sales".into())].clone();
    assert_eq!(
        sales,
        vec![
            Scalar::F64(85.0),
            Scalar::F64(80.0),
            Scalar::F64(90.0),
            Scalar::I64(2)
        ]
    );
}

#[test]
fn select_count_star() {
    let out = ctx().run("SELECT COUNT(*) FROM employees").unwrap();
    assert_eq!(out.row_count(), 1);
    assert_eq!(out.column_names(), vec!["COUNT(*)".to_string()]);
    let counts = all_scalars(&out, 0);
    assert_eq!(counts, vec![Scalar::I64(4)]);
}

#[test]
fn select_global_aggregate() {
    let out = ctx()
        .run("SELECT SUM(salary) AS total FROM employees")
        .unwrap();
    assert_eq!(out.row_count(), 1);
    assert_eq!(out.column_names(), vec!["SUM(salary)".to_string()]);
}

#[test]
fn select_group_by_reordered_projection() {
    // SELECT order differs from GROUP BY output order; binder re-projects.
    let out = ctx()
        .run("SELECT SUM(salary), dept FROM employees GROUP BY dept")
        .unwrap();
    assert_eq!(
        out.column_names(),
        vec!["SUM(salary)".to_string(), "dept".to_string()]
    );
}

#[test]
fn select_order_by_agg_desc() {
    let out = ctx()
        .run("SELECT dept, SUM(salary) FROM employees GROUP BY dept ORDER BY SUM(salary) DESC")
        .unwrap();
    let depts = all_scalars(&out, 0);
    assert_eq!(
        depts,
        vec![Scalar::Str("eng".into()), Scalar::Str("sales".into())]
    );
}

#[test]
fn select_order_by_column() {
    let out = ctx()
        .run("SELECT dept, salary FROM employees ORDER BY salary DESC")
        .unwrap();
    let salaries = all_scalars(&out, 1);
    assert_eq!(
        salaries,
        vec![
            Scalar::F64(120.0),
            Scalar::F64(100.0),
            Scalar::F64(90.0),
            Scalar::F64(80.0)
        ]
    );
}

#[test]
fn select_limit() {
    let out = ctx()
        .run("SELECT dept FROM employees ORDER BY salary DESC LIMIT 2")
        .unwrap();
    assert_eq!(out.row_count(), 2);
}

#[test]
fn select_where_group_by_order_by_limit_combined() {
    let out = ctx()
        .run(
            "SELECT city, COUNT(*) FROM employees WHERE years >= 2 GROUP BY city ORDER BY city DESC LIMIT 2",
        )
        .unwrap();
    assert_eq!(out.row_count(), 2);
    assert_eq!(
        out.column_names(),
        vec!["city".to_string(), "COUNT(*)".to_string()]
    );
    let cities = all_scalars(&out, 0);
    assert_eq!(
        cities,
        vec![Scalar::Str("sfo".into()), Scalar::Str("nyc".into())]
    );
}

#[test]
fn select_join_on() {
    let out = ctx()
        .run("SELECT name, population FROM employees JOIN cities ON city = name")
        .unwrap();
    assert_eq!(out.row_count(), 4);
    let pops = all_scalars(&out, 1);
    assert_eq!(
        pops,
        vec![
            Scalar::I64(8_000_000),
            Scalar::I64(800_000),
            Scalar::I64(8_000_000),
            Scalar::I64(1_000_000)
        ]
    );
}

#[test]
fn select_trailing_semicolon() {
    let out = ctx().run("SELECT * FROM employees;").unwrap();
    assert_eq!(out.row_count(), 4);
}

#[test]
fn select_qualified_columns() {
    let out = ctx().run("SELECT employees.dept FROM employees").unwrap();
    assert_eq!(out.row_count(), 4);
    assert_eq!(out.column_names(), vec!["dept".to_string()]);
}

// === Error cases ===

#[test]
fn error_unknown_table() {
    let err = ctx().run("SELECT * FROM nope").unwrap_err();
    assert!(err.to_string().contains("no table named 'nope'"), "{err}");
}

#[test]
fn error_unsupported_aggregate() {
    let err = ctx()
        .run("SELECT MEDIAN(salary) FROM employees")
        .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("unsupported function 'MEDIAN' in SELECT"),
        "{text}"
    );
}

#[test]
fn error_type_mismatch_literal() {
    let err = ctx()
        .run("SELECT dept FROM employees WHERE salary = 'abc'")
        .unwrap_err();
    let text = err.to_string();
    assert!(text.contains("Type mismatch in 'salary'"), "{text}");
}

#[test]
fn error_group_by_validation() {
    let err = ctx()
        .run("SELECT city FROM employees GROUP BY dept")
        .unwrap_err();
    let text = err.to_string();
    assert!(text.contains("must appear in the GROUP BY"), "{text}");
}

#[test]
fn error_syntax() {
    let err = ctx().run("SELEC * FROM employees").unwrap_err();
    let text = err.to_string();
    assert!(text.contains("SQL error"), "{text}");
}
