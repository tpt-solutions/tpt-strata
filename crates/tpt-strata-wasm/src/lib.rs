//! WebAssembly entry points for tpt-strata.
//!
//! Every function takes the tables as one JSON object, mapping each table name
//! to its CSV text, and a SQL string:
//!
//! ```js
//! const tables = JSON.stringify({ orders: "id,amount\n1,2.5\n2,4\n" });
//! const result = JSON.parse(run(tables, "SELECT SUM(amount) AS total FROM orders"));
//! // result = { columns: ["total"], rows: [[6.5]], rowCount: 1 }
//! ```
//!
//! Errors are thrown as strings of the form `"[kind] message"`, where `kind` is
//! the stable [`QueryError::kind`] name (`missing-column`, `sql`, and so on).
//!
//! The logic lives in plain functions (`run_json`, `render`, `explain`,
//! `check`) so it can be tested on the host. The `#[wasm_bindgen]` wrappers only
//! convert errors for JavaScript.

use std::collections::BTreeMap;

use serde_json::{json, Value};
use tpt_strata::csv::parse_csv;
use tpt_strata::format::render_table;
use tpt_strata::sql::SqlContext;
use tpt_strata::{QueryError, Scalar, Table};
use wasm_bindgen::prelude::*;

/// Tagged error text: `[kind] message`.
fn tagged(e: &QueryError) -> String {
    format!("[{}] {e}", e.kind())
}

/// Parse the JSON map of table name to CSV text into tables.
fn load_tables(tables_json: &str) -> Result<Vec<(String, Table)>, String> {
    let map: BTreeMap<String, String> = serde_json::from_str(tables_json).map_err(|e| {
        format!("[input] tables must be a JSON object of table name to CSV text: {e}")
    })?;
    let mut out = Vec::with_capacity(map.len());
    for (name, csv) in map {
        let table = parse_csv(&csv).map_err(|e| format!("[{}] table '{name}': {e}", e.kind()))?;
        out.push((name, table));
    }
    Ok(out)
}

/// Run `f` with a [`SqlContext`] over the tables described by `tables_json`.
fn with_context<T>(
    tables_json: &str,
    f: impl FnOnce(&SqlContext<'_>) -> Result<T, QueryError>,
) -> Result<T, String> {
    let tables = load_tables(tables_json)?;
    let mut ctx = SqlContext::new();
    for (name, table) in &tables {
        ctx.add_table(name, table);
    }
    f(&ctx).map_err(|e| tagged(&e))
}

fn scalar_json(v: Option<Scalar>) -> Value {
    match v {
        None | Some(Scalar::Null) => Value::Null,
        Some(Scalar::Bool(b)) => Value::Bool(b),
        Some(Scalar::I32(n)) => json!(n),
        Some(Scalar::I64(n)) => json!(n),
        Some(Scalar::F64(f)) => serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Some(Scalar::Str(s)) => Value::String(s),
        Some(Scalar::Date(d)) => Value::String(tpt_strata::temporal::format_date(d)),
        Some(Scalar::Timestamp(t)) => Value::String(tpt_strata::temporal::format_timestamp(t)),
    }
}

/// A query result as JSON: `{ columns, rows, rowCount }`.
pub fn table_to_json(table: &Table) -> Value {
    let columns: Vec<&str> = table.column_names();
    let mut rows = Vec::with_capacity(table.row_count());
    for batch in table.batches() {
        for r in 0..batch.row_count() {
            let row: Vec<Value> = batch
                .columns
                .iter()
                .map(|c| scalar_json(c.get(r)))
                .collect();
            rows.push(Value::Array(row));
        }
    }
    json!({ "columns": columns, "rows": rows, "rowCount": table.row_count() })
}

/// Run a query and return its result as JSON text.
pub fn run_json(tables_json: &str, sql: &str) -> Result<String, String> {
    let table = with_context(tables_json, |ctx| ctx.run(sql))?;
    Ok(table_to_json(&table).to_string())
}

/// Run a query and return the result as a text grid.
pub fn render(tables_json: &str, sql: &str) -> Result<String, String> {
    let table = with_context(tables_json, |ctx| ctx.run(sql))?;
    Ok(render_table(&table))
}

/// Return the physical plan as an indented tree, without running the query.
pub fn explain(tables_json: &str, sql: &str) -> Result<String, String> {
    with_context(tables_json, |ctx| ctx.explain(sql))
}

/// Check that the query parses and binds, without reading the data.
pub fn check(tables_json: &str, sql: &str) -> Result<(), String> {
    with_context(tables_json, |ctx| ctx.check(sql))
}

/// The version of the tpt-strata engine this module was built from.
pub fn engine_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// === JavaScript entry points ===

fn to_js(result: Result<String, String>) -> Result<String, JsError> {
    result.map_err(|e| JsError::new(&e))
}

/// Run a query. Returns JSON text: `{ columns, rows, rowCount }`.
#[wasm_bindgen]
pub fn run(tables_json: &str, sql: &str) -> Result<String, JsError> {
    to_js(run_json(tables_json, sql))
}

/// Run a query. Returns the result as a text grid.
#[wasm_bindgen]
pub fn render_query(tables_json: &str, sql: &str) -> Result<String, JsError> {
    to_js(render(tables_json, sql))
}

/// Return the physical plan for a query, without running it.
#[wasm_bindgen]
pub fn explain_query(tables_json: &str, sql: &str) -> Result<String, JsError> {
    to_js(explain(tables_json, sql))
}

/// Check a query against the tables without running it. Throws on error.
#[wasm_bindgen]
pub fn check_query(tables_json: &str, sql: &str) -> Result<(), JsError> {
    check(tables_json, sql).map_err(|e| JsError::new(&e))
}

/// The engine version, for showing in the playground.
#[wasm_bindgen]
pub fn version() -> String {
    engine_version().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDERS: &str = "id,cust,amount\n1,1,2.5\n2,2,4\n3,1,\n";
    const CUSTOMERS: &str = "cid,name\n1,ann\n2,ben\n";

    fn tables() -> String {
        json!({ "orders": ORDERS, "customers": CUSTOMERS }).to_string()
    }

    #[test]
    fn run_returns_columns_rows_and_nulls() {
        let out: Value = serde_json::from_str(
            &run_json(&tables(), "SELECT id, amount FROM orders ORDER BY id").unwrap(),
        )
        .unwrap();
        assert_eq!(out["columns"], json!(["id", "amount"]));
        assert_eq!(out["rowCount"], json!(3));
        assert_eq!(out["rows"][2], json!([3, null]));
    }

    #[test]
    fn join_across_two_tables() {
        let out = run_json(
            &tables(),
            "SELECT customers.name, SUM(orders.amount) AS total FROM orders \
             JOIN customers ON orders.cust = customers.cid GROUP BY customers.name ORDER BY name",
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["rows"], json!([["ann", 2.5], ["ben", 4.0]]));
    }

    #[test]
    fn errors_carry_their_kind_tag() {
        let err = run_json(&tables(), "SELECT nope FROM orders").unwrap_err();
        assert!(err.starts_with("[missing-column]"), "{err}");
        let err = run_json(&tables(), "SELECT FROM").unwrap_err();
        assert!(err.starts_with("[sql]"), "{err}");
    }

    #[test]
    fn bad_table_json_is_an_input_error() {
        let err = run_json("not json", "SELECT 1 FROM t").unwrap_err();
        assert!(err.starts_with("[input]"), "{err}");
    }

    #[test]
    fn explain_and_check_do_not_run_the_query() {
        let plan = explain(&tables(), "SELECT id FROM orders WHERE amount > 1").unwrap();
        assert!(plan.contains("Filter amount > 1"), "{plan}");
        check(&tables(), "SELECT id FROM orders").unwrap();
        assert!(check(&tables(), "SELECT id FROM missing_table").is_err());
    }

    #[test]
    fn render_produces_a_grid() {
        let grid = render(&tables(), "SELECT name FROM customers ORDER BY name").unwrap();
        assert!(grid.contains("| ann"), "{grid}");
    }
}
