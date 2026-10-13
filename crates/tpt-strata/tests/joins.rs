//! LEFT, RIGHT and FULL JOIN, chains of joins, and ON clauses that refer to earlier tables.

use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, Scalar, Table};

fn s(v: &str) -> Scalar {
    Scalar::Str(v.to_string())
}

fn tables() -> (Table, Table, Table) {
    let people = Table::try_new(vec![
        Column::new("pid", vec![Scalar::I64(1), Scalar::I64(2), Scalar::I64(3)]),
        Column::new("name", vec![s("ann"), s("ben"), s("cy")]),
        Column::new(
            "city_id",
            vec![Scalar::I64(10), Scalar::I64(20), Scalar::I64(99)],
        ),
    ])
    .unwrap();
    let cities = Table::try_new(vec![
        Column::new("cid", vec![Scalar::I64(10), Scalar::I64(20)]),
        Column::new("city", vec![s("oslo"), s("rome")]),
        Column::new("country_id", vec![Scalar::I64(1), Scalar::I64(2)]),
    ])
    .unwrap();
    let countries = Table::try_new(vec![
        Column::new("kid", vec![Scalar::I64(1), Scalar::I64(2)]),
        Column::new("country", vec![s("norway"), s("italy")]),
    ])
    .unwrap();
    (people, cities, countries)
}

fn col(t: &Table, c: usize) -> Vec<Option<Scalar>> {
    let mut out = Vec::new();
    for b in t.batches() {
        for r in 0..b.row_count() {
            out.push(b.columns[c].get(r));
        }
    }
    out
}

#[test]
fn left_join_keeps_unmatched_rows_with_nulls() {
    let (people, cities, _) = tables();
    let mut ctx = SqlContext::new();
    ctx.add_table("people", &people);
    ctx.add_table("cities", &cities);
    let out = ctx
        .run("SELECT people.name, cities.city FROM people LEFT JOIN cities ON people.city_id = cities.cid ORDER BY name")
        .unwrap();
    assert_eq!(out.row_count(), 3);
    assert_eq!(
        col(&out, 0),
        vec![Some(s("ann")), Some(s("ben")), Some(s("cy"))]
    );
    // cy's city_id 99 has no match, so the city is NULL.
    assert_eq!(col(&out, 1), vec![Some(s("oslo")), Some(s("rome")), None]);
}

#[test]
fn inner_join_drops_unmatched_rows() {
    let (people, cities, _) = tables();
    let mut ctx = SqlContext::new();
    ctx.add_table("people", &people);
    ctx.add_table("cities", &cities);
    let out = ctx
        .run("SELECT people.name FROM people JOIN cities ON people.city_id = cities.cid")
        .unwrap();
    assert_eq!(out.row_count(), 2);
}

#[test]
fn three_table_chain_with_on_clause_on_an_earlier_table() {
    let (people, cities, countries) = tables();
    let mut ctx = SqlContext::new();
    ctx.add_table("people", &people);
    ctx.add_table("cities", &cities);
    ctx.add_table("countries", &countries);
    let out = ctx
        .run(
            "SELECT people.name, countries.country FROM people \
             JOIN cities ON people.city_id = cities.cid \
             JOIN countries ON cities.country_id = countries.kid ORDER BY name",
        )
        .unwrap();
    assert_eq!(col(&out, 0), vec![Some(s("ann")), Some(s("ben"))]);
    assert_eq!(col(&out, 1), vec![Some(s("norway")), Some(s("italy"))]);
}

#[test]
fn left_join_chained_after_an_inner_join() {
    let (people, cities, countries) = tables();
    let mut ctx = SqlContext::new();
    ctx.add_table("people", &people);
    ctx.add_table("cities", &cities);
    ctx.add_table("countries", &countries);
    let out = ctx
        .run(
            "SELECT people.name, cities.city, countries.country FROM people \
             LEFT JOIN cities ON people.city_id = cities.cid \
             LEFT JOIN countries ON cities.country_id = countries.kid ORDER BY name",
        )
        .unwrap();
    assert_eq!(out.row_count(), 3);
    assert_eq!(
        col(&out, 2),
        vec![Some(s("norway")), Some(s("italy")), None]
    );
}

#[test]
fn on_clause_must_reference_the_table_being_joined() {
    let (people, cities, countries) = tables();
    let mut ctx = SqlContext::new();
    ctx.add_table("people", &people);
    ctx.add_table("cities", &cities);
    ctx.add_table("countries", &countries);
    // Both sides of this ON clause are already joined, so there is no new table to join.
    let err = ctx
        .run(
            "SELECT people.name FROM people JOIN cities ON people.city_id = cities.cid \
             JOIN countries ON people.pid = cities.country_id",
        )
        .unwrap_err();
    assert!(err.to_string().contains("must compare"), "{err}");
}

#[test]
fn right_join_keeps_unmatched_right_rows_with_nulls() {
    let (people, cities, _) = tables();
    let mut ctx = SqlContext::new();
    ctx.add_table("people", &people);
    ctx.add_table("cities", &cities);
    // Person 3 lives in city 99, which matches nothing, so it survives with a
    // NULL city.
    let out = ctx
        .run(
            "SELECT cities.city, people.name FROM cities \
             RIGHT JOIN people ON cities.cid = people.city_id ORDER BY name",
        )
        .unwrap();
    assert_eq!(out.row_count(), 3);
    assert_eq!(col(&out, 0), vec![Some(s("oslo")), Some(s("rome")), None]);
    assert_eq!(
        col(&out, 1),
        vec![Some(s("ann")), Some(s("ben")), Some(s("cy"))]
    );
}

#[test]
fn full_join_keeps_unmatched_rows_from_both_sides() {
    let (people, _, _) = tables();
    // Paris has no resident, so the right side has an unmatched row too.
    let cities = Table::try_new(vec![
        Column::new(
            "cid",
            vec![Scalar::I64(10), Scalar::I64(20), Scalar::I64(30)],
        ),
        Column::new("city", vec![s("oslo"), s("rome"), s("paris")]),
        Column::new(
            "country_id",
            vec![Scalar::I64(1), Scalar::I64(2), Scalar::I64(2)],
        ),
    ])
    .unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("people", &people);
    ctx.add_table("cities", &cities);
    let out = ctx
        .run(
            "SELECT people.name, cities.city FROM people \
             FULL OUTER JOIN cities ON people.city_id = cities.cid ORDER BY city",
        )
        .unwrap();
    // ann-oslo, ben-rome, cy-NULL (no city), NULL-paris (no resident).
    assert_eq!(out.row_count(), 4);
    let names = col(&out, 0);
    let cities_col = col(&out, 1);
    assert_eq!(names.iter().filter(|v| v.is_none()).count(), 1);
    assert_eq!(cities_col.iter().filter(|v| v.is_none()).count(), 1);
    assert!(cities_col.contains(&Some(s("paris"))));
    assert!(names.contains(&Some(s("cy"))));
}

#[test]
fn join_kinds_are_reported_in_explain() {
    let (people, cities, _) = tables();
    let mut ctx = SqlContext::new();
    ctx.add_table("people", &people);
    ctx.add_table("cities", &cities);
    let plan = ctx
        .explain("SELECT people.name FROM people FULL JOIN cities ON people.city_id = cities.cid")
        .unwrap();
    assert!(plan.contains("Full HashJoin"), "{plan}");
}
