//! Python bindings for tpt-strata.
//!
//! ```python
//! import tpt_strata as strata
//!
//! orders = strata.Table.from_csv("orders.csv")
//! result = strata.sql({"orders": orders}, "SELECT region, SUM(quantity) AS units FROM orders GROUP BY region")
//! print(result)            # a text grid
//! print(result.to_rows())  # [(region, units), ...]
//! ```

use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyFileNotFoundError, PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDate, PyDateTime, PyDict, PyInt, PyTuple};
use tpt_strata::csv::{parse_csv, to_csv};
use tpt_strata::format::render_table;
use tpt_strata::sql::SqlContext;
use tpt_strata::{QueryError, Scalar, Table};

create_exception!(
    tpt_strata,
    StrataError,
    PyException,
    "A query, schema or data error from tpt-strata. Its `kind` attribute is the stable error kind."
);

fn to_py_err(py: Python<'_>, e: QueryError) -> PyErr {
    let err = StrataError::new_err(e.to_string());
    // The kind is attached so Python code can branch on it without parsing text.
    let _ = err.value(py).setattr("kind", e.kind());
    err
}

fn io_err(path: &str, e: std::io::Error) -> PyErr {
    match e.kind() {
        std::io::ErrorKind::NotFound => PyFileNotFoundError::new_err(format!("{path}: {e}")),
        _ => PyOSError::new_err(format!("{path}: {e}")),
    }
}

/// Convert a Python value to a tpt-strata scalar. Booleans are checked before
/// integers, because `bool` is a subclass of `int` in Python.
fn py_to_scalar(v: &Bound<'_, PyAny>) -> PyResult<Scalar> {
    if v.is_none() {
        return Ok(Scalar::Null);
    }
    if v.is_instance_of::<PyBool>() {
        return Ok(Scalar::Bool(v.extract()?));
    }
    if v.is_instance_of::<PyInt>() {
        // An int that does not fit in i64 must not fall through to the float
        // branch below, which would round it silently.
        return v
            .extract::<i64>()
            .map(Scalar::I64)
            .map_err(|_| PyValueError::new_err("integer does not fit in a 64-bit signed value"));
    }
    // datetime is a subclass of date, so it is checked first.
    if v.is_instance_of::<PyDateTime>() {
        if !v.getattr("tzinfo")?.is_none() {
            return Err(PyValueError::new_err(
                "datetime has a time zone; tpt-strata timestamps have none, so pass a naive datetime",
            ));
        }
        let epoch = PyDateTime::new(v.py(), 1970, 1, 1, 0, 0, 0, 0, None)?;
        let delta = v.sub(epoch)?;
        let days: i64 = delta.getattr("days")?.extract()?;
        let seconds: i64 = delta.getattr("seconds")?.extract()?;
        let micros: i64 = delta.getattr("microseconds")?.extract()?;
        return Ok(Scalar::Timestamp(
            (days * 86_400 + seconds) * 1_000_000 + micros,
        ));
    }
    if v.is_instance_of::<PyDate>() {
        // 719163 is the proleptic Gregorian ordinal of 1970-01-01.
        let ordinal: i64 = v.call_method0("toordinal")?.extract()?;
        return Ok(Scalar::Date((ordinal - 719_163) as i32));
    }
    if let Ok(f) = v.extract::<f64>() {
        return Ok(Scalar::F64(f));
    }
    if let Ok(s) = v.extract::<String>() {
        return Ok(Scalar::Str(s));
    }
    Err(PyTypeError::new_err(format!(
        "cannot store a value of type {} in a table; use None, bool, int, float, str, date or datetime",
        v.get_type().name()?
    )))
}

fn scalar_to_py(py: Python<'_>, s: Scalar) -> PyResult<Py<PyAny>> {
    Ok(match s {
        Scalar::Null => py.None(),
        Scalar::Bool(b) => b.into_pyobject(py)?.to_owned().into_any().unbind(),
        Scalar::I32(n) => n.into_pyobject(py)?.into_any().unbind(),
        Scalar::I64(n) => n.into_pyobject(py)?.into_any().unbind(),
        Scalar::F64(f) => f.into_pyobject(py)?.into_any().unbind(),
        Scalar::Str(s) => s.into_pyobject(py)?.into_any().unbind(),
        // Built by adding a timedelta to the epoch, so dates before 1970 work
        // and a value outside Python's range raises OverflowError.
        Scalar::Date(days) => {
            let dt = py.import("datetime")?;
            let epoch = dt.getattr("date")?.call1((1970, 1, 1))?;
            let delta = dt.getattr("timedelta")?.call1((days,))?;
            epoch.add(delta)?.unbind()
        }
        // A naive datetime, because a tpt-strata timestamp has no time zone.
        Scalar::Timestamp(micros) => {
            let dt = py.import("datetime")?;
            let epoch = dt.getattr("datetime")?.call1((1970, 1, 1))?;
            let delta = dt.getattr("timedelta")?.call1((0, 0, micros))?;
            epoch.add(delta)?.unbind()
        }
    })
}

/// A table of typed columns. Create one from CSV, Parquet, or Python rows.
#[pyclass(name = "Table", module = "tpt_strata", skip_from_py_object)]
#[derive(Clone)]
pub struct PyTable {
    inner: Table,
}

#[pymethods]
impl PyTable {
    /// Read a CSV file. The first row is the header. Types are inferred.
    #[staticmethod]
    fn from_csv(py: Python<'_>, path: &str) -> PyResult<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| io_err(path, e))?;
        let inner = parse_csv(&text).map_err(|e| to_py_err(py, e))?;
        Ok(Self { inner })
    }

    /// Parse CSV text that is already in memory.
    #[staticmethod]
    fn from_csv_text(py: Python<'_>, text: &str) -> PyResult<Self> {
        let inner = parse_csv(text).map_err(|e| to_py_err(py, e))?;
        Ok(Self { inner })
    }

    /// Read a Parquet file. Pass `columns` to read only some of them.
    #[staticmethod]
    #[pyo3(signature = (path, columns=None))]
    fn from_parquet(py: Python<'_>, path: &str, columns: Option<Vec<String>>) -> PyResult<Self> {
        let inner = match columns {
            None => tpt_strata_parquet::read_parquet(path),
            Some(names) => {
                let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                tpt_strata_parquet::read_parquet_columns(path, &refs)
            }
        }
        .map_err(|e| to_py_err(py, e))?;
        Ok(Self { inner })
    }

    /// Build a table from column names and a list of rows. Every row must have
    /// one value per name. A column with only None values cannot be typed.
    #[staticmethod]
    fn from_rows(py: Python<'_>, names: Vec<String>, rows: Vec<Vec<Py<PyAny>>>) -> PyResult<Self> {
        let mut data = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut values = Vec::with_capacity(row.len());
            for v in row {
                values.push(py_to_scalar(v.bind(py))?);
            }
            data.push(values);
        }
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let inner = Table::from_rows(&refs, data).map_err(|e| to_py_err(py, e))?;
        Ok(Self { inner })
    }

    /// The column names, in order.
    #[getter]
    fn columns(&self) -> Vec<String> {
        self.inner
            .column_names()
            .into_iter()
            .map(String::from)
            .collect()
    }

    /// The number of rows.
    #[getter]
    fn row_count(&self) -> usize {
        self.inner.row_count()
    }

    /// The rows as a list of tuples. NULL becomes None.
    fn to_rows(&self, py: Python<'_>) -> PyResult<Vec<Py<PyTuple>>> {
        let mut out = Vec::with_capacity(self.inner.row_count());
        for batch in self.inner.batches() {
            for r in 0..batch.row_count() {
                let values: Vec<Py<PyAny>> = batch
                    .columns
                    .iter()
                    .map(|c| scalar_to_py(py, c.get(r).unwrap_or(Scalar::Null)))
                    .collect::<PyResult<_>>()?;
                out.push(PyTuple::new(py, values)?.unbind());
            }
        }
        Ok(out)
    }

    /// The table as CSV text, with a header row.
    fn to_csv(&self) -> String {
        to_csv(&self.inner)
    }

    /// Write the table to a Parquet file.
    fn to_parquet(&self, py: Python<'_>, path: &str) -> PyResult<()> {
        tpt_strata_parquet::write_parquet(&self.inner, path).map_err(|e| to_py_err(py, e))
    }

    fn __len__(&self) -> usize {
        self.inner.row_count()
    }

    fn __repr__(&self) -> String {
        render_table(&self.inner)
    }

    fn __str__(&self) -> String {
        render_table(&self.inner)
    }
}

/// Collect `{name: Table}` into owned tables, so the query can run without the GIL.
fn tables_from_dict(tables: &Bound<'_, PyDict>) -> PyResult<Vec<(String, Table)>> {
    let mut out = Vec::with_capacity(tables.len());
    for (name, value) in tables.iter() {
        let name: String = name.extract()?;
        let table: PyRef<'_, PyTable> = value.extract().map_err(|_| {
            PyTypeError::new_err(format!("table '{name}' must be a tpt_strata.Table"))
        })?;
        out.push((name, table.inner.clone()));
    }
    Ok(out)
}

fn with_context<T: Send>(
    py: Python<'_>,
    tables: &Bound<'_, PyDict>,
    f: impl FnOnce(&SqlContext<'_>) -> Result<T, QueryError> + Send,
) -> PyResult<T> {
    let owned = tables_from_dict(tables)?;
    py.detach(move || {
        let mut ctx = SqlContext::new();
        for (name, table) in &owned {
            ctx.add_table(name, table);
        }
        f(&ctx)
    })
    .map_err(|e| to_py_err(py, e))
}

/// Run a SQL query over `tables`, a dict of name to `Table`. Returns a `Table`.
#[pyfunction]
fn sql(py: Python<'_>, tables: &Bound<'_, PyDict>, query: &str) -> PyResult<PyTable> {
    let inner = with_context(py, tables, |ctx| ctx.run(query))?;
    Ok(PyTable { inner })
}

/// Return the physical plan for a query as text, without running it.
#[pyfunction]
fn explain(py: Python<'_>, tables: &Bound<'_, PyDict>, query: &str) -> PyResult<String> {
    with_context(py, tables, |ctx| ctx.explain(query))
}

/// Check a query against the table schemas without reading the data.
/// Raises StrataError if it does not parse or bind.
#[pyfunction]
fn check(py: Python<'_>, tables: &Bound<'_, PyDict>, query: &str) -> PyResult<()> {
    with_context(py, tables, |ctx| ctx.check(query))
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyTable>()?;
    m.add("StrataError", m.py().get_type::<StrataError>())?;
    m.add_function(wrap_pyfunction!(sql, m)?)?;
    m.add_function(wrap_pyfunction!(explain, m)?)?;
    m.add_function(wrap_pyfunction!(check, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
