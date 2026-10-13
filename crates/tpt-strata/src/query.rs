use std::sync::Arc;

use crate::engine::{
    AggFunc as EngAggFunc, Aggregate, AggregateExec, Comparison, DataSource, Expr, FilterExec,
    JoinExec, LimitExec, PhysicalPlan, Predicate, ProjectExec, SortExec, SortExpr,
};
use crate::error::QueryError;
use crate::format::Table;
use crate::schema::{DataType, Field};
use crate::types::Scalar;

/// Supported aggregate functions exposed by the builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggFunc {
    Sum,
    Count,
    Min,
    Max,
    Avg,
}

impl std::fmt::Display for AggFunc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AggFunc::Sum => write!(f, "SUM"),
            AggFunc::Count => write!(f, "COUNT"),
            AggFunc::Min => write!(f, "MIN"),
            AggFunc::Max => write!(f, "MAX"),
            AggFunc::Avg => write!(f, "AVG"),
        }
    }
}

fn to_engine_func(func: AggFunc) -> EngAggFunc {
    match func {
        AggFunc::Sum => EngAggFunc::Sum,
        AggFunc::Count => EngAggFunc::Count,
        AggFunc::Min => EngAggFunc::Min,
        AggFunc::Max => EngAggFunc::Max,
        AggFunc::Avg => EngAggFunc::Avg,
    }
}

fn to_engine_op(op: &str) -> Option<Comparison> {
    match op {
        "=" => Some(Comparison::Eq),
        "!=" => Some(Comparison::NotEq),
        ">" => Some(Comparison::Gt),
        ">=" => Some(Comparison::Gte),
        "<" => Some(Comparison::Lt),
        "<=" => Some(Comparison::Lte),
        _ => None,
    }
}

/// Coerce a literal to the type of the column it is compared against.
///
/// Integer and float literals may be compared with any numeric column, and
/// the comparison is exact (see [`crate::types::numeric_cmp`]), so the literal
/// keeps its own type. Other mismatched types are rejected.
pub(crate) fn coerce_literal(
    value: Scalar,
    field: &Field,
    column: &str,
) -> Result<Scalar, QueryError> {
    let numeric_column = matches!(
        field.data_type,
        DataType::Int32 | DataType::Int64 | DataType::Float64
    );
    let compatible = match &value {
        Scalar::Null => true,
        Scalar::I32(_) | Scalar::I64(_) | Scalar::F64(_) => numeric_column,
        other => other.type_name() == field.data_type.name(),
    };
    if compatible {
        Ok(value)
    } else {
        Err(QueryError::TypeMismatch {
            column: column.to_string(),
            expected: field.data_type.name(),
            actual: value.type_name(),
            context: format!(
                "Cannot compare '{column}' ({}) with a literal of {} type",
                field.data_type,
                value.type_name()
            ),
        })
    }
}

/// A builder for constructing and executing queries over a [`Table`].
///
/// Zero trait implementations are required for the common case.
#[derive(Debug)]
pub struct QueryBuilder<'a> {
    table: &'a Table,
    predicates: Vec<Predicate>,
    projections: Option<Vec<String>>,
    group_by: Vec<String>,
    /// `(column, func)` pairs. The column `"*"` stands for `COUNT(*)`.
    aggregates: Vec<(String, AggFunc)>,
    order_by: Option<(String, bool)>,
    limit: Option<usize>,
}

impl<'a> QueryBuilder<'a> {
    /// Start a query over the given table.
    pub fn new(table: &'a Table) -> Self {
        Self {
            table,
            predicates: Vec::new(),
            projections: None,
            group_by: Vec::new(),
            aggregates: Vec::new(),
            order_by: None,
            limit: None,
        }
    }

    /// Filter rows where `column` `op` `value`.
    ///
    /// Supported ops: `=`, `!=`, `>`, `>=`, `<`, `<=`. Integer and float
    /// values may be mixed with numeric columns; other type mismatches are
    /// rejected at build time.
    pub fn filter(mut self, column: &str, op: &str, value: Scalar) -> Result<Self, QueryError> {
        let idx = self.table.column_index(column)?;
        let field = &self.table.schema().fields[idx];

        if !matches!(value, Scalar::Null) && coerce_literal(value.clone(), field, column).is_err() {
            let field_type = field.data_type.name();
            return Err(QueryError::TypeMismatch {
                column: column.to_string(),
                expected: field_type,
                actual: value.type_name(),
                context: format!(
                    "Cannot compare '{column}' ({field_type}) with a value of {} type",
                    value.type_name()
                ),
            });
        }

        let eng_op = to_engine_op(op).ok_or_else(|| QueryError::UnsupportedOperation {
            message: format!(
                "unsupported comparison operator '{op}'; use one of =, !=, >, >=, <, <="
            ),
        })?;

        self.predicates.push(Predicate {
            column: idx,
            op: eng_op,
            value,
        });
        Ok(self)
    }

    /// Project only the specified columns.
    ///
    /// Names refer to the output of the query: input columns when there is no
    /// aggregate, or group-by columns and aggregate names such as
    /// `"SUM(salary)"` when there is one. Names are checked when the query
    /// executes, because aggregate names only exist once aggregation is planned.
    pub fn project(mut self, columns: Vec<&str>) -> Result<Self, QueryError> {
        self.projections = Some(columns.into_iter().map(String::from).collect());
        Ok(self)
    }

    /// Group by the specified columns.
    pub fn group_by(mut self, columns: Vec<&str>) -> Result<Self, QueryError> {
        for col in &columns {
            self.table.column_index(col)?;
        }
        self.group_by = columns.into_iter().map(String::from).collect();
        Ok(self)
    }

    /// Add an aggregate over the specified column.
    pub fn aggregate(mut self, column: &str, func: AggFunc) -> Result<Self, QueryError> {
        self.table.column_index(column)?;
        self.aggregates.push((column.to_string(), func));
        Ok(self)
    }

    /// Add `COUNT(*)`, which counts every row including rows with nulls.
    pub fn count_all(mut self) -> Self {
        self.aggregates.push(("*".to_string(), AggFunc::Count));
        self
    }

    /// Order results by the specified column (ascending unless `desc`).
    ///
    /// The column is checked against the query's output when the query is
    /// executed, since aggregate names only exist once aggregation is planned.
    pub fn order_by(mut self, column: &str, desc: bool) -> Result<Self, QueryError> {
        self.order_by = Some((column.to_string(), desc));
        Ok(self)
    }

    /// Limit the result to at most `n` rows.
    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    /// Execute the query and return the result table.
    pub fn execute(self) -> Result<Table, QueryError> {
        let input_schema = Arc::new(self.table.schema().clone());
        let mut plan: Arc<dyn PhysicalPlan> = Arc::new(DataSource::from_shared(
            input_schema,
            self.table.batches().to_vec(),
        ));

        for predicate in self.predicates {
            plan = Arc::new(FilterExec::new(plan, predicate));
        }

        if !self.group_by.is_empty() || !self.aggregates.is_empty() {
            let group_indices: Vec<usize> = self
                .group_by
                .iter()
                .map(|name| self.table.column_index(name))
                .collect::<Result<_, _>>()?;
            let aggregates: Vec<Aggregate> = self
                .aggregates
                .iter()
                .map(|(col, func)| {
                    if col == "*" {
                        return Ok(Aggregate {
                            func: to_engine_func(*func),
                            column: 0,
                            out_name: format!("{func}(*)"),
                            count_star: true,
                        });
                    }
                    self.table.column_index(col).map(|idx| Aggregate {
                        func: to_engine_func(*func),
                        column: idx,
                        out_name: format!("{func}({col})"),
                        count_star: false,
                    })
                })
                .collect::<Result<_, _>>()?;
            plan = Arc::new(AggregateExec::new(plan, group_indices, aggregates)?);
        }

        // Projection applies to the query output: the input columns when there
        // is no aggregation, otherwise the group-by and aggregate columns.
        if let Some(projections) = self.projections.as_ref() {
            let schema = plan.schema();
            let exprs: Vec<Expr> = projections
                .iter()
                .map(|name| {
                    schema
                        .index_of(name)
                        .map(|idx| Expr { column: idx })
                        .ok_or_else(|| QueryError::MissingColumn {
                            column: name.clone(),
                            available: schema.fields.iter().map(|f| f.name.clone()).collect(),
                        })
                })
                .collect::<Result<_, _>>()?;
            plan = Arc::new(ProjectExec::new(plan, exprs, projections.clone())?);
        }

        if let Some((col, desc)) = self.order_by.as_ref() {
            // The order column must be resolved against the current output schema.
            let schema = plan.schema();
            if let Some(idx) = schema.index_of(col) {
                plan = Arc::new(SortExec::new(
                    plan,
                    vec![SortExpr {
                        column: idx,
                        desc: *desc,
                    }],
                ));
            } else {
                return Err(QueryError::MissingColumn {
                    column: col.clone(),
                    available: schema.fields.iter().map(|f| f.name.clone()).collect(),
                });
            }
        }

        if let Some(n) = self.limit {
            plan = Arc::new(LimitExec::new(plan, n));
        }

        let batches = plan.execute()?;
        Ok(Table::from_shared_batches(
            Arc::new(plan.schema().clone()),
            batches,
        ))
    }
}

/// Build and execute an inner equi-join between two tables.
///
/// The two tables must not share a column name, because the output must have
/// unambiguous column names. Rename one side first (see
/// [`Table::rename_columns`]).
pub fn join(
    left: &Table,
    right: &Table,
    left_column: &str,
    right_column: &str,
) -> Result<Table, QueryError> {
    let left_idx = left.column_index(left_column)?;
    let right_idx = right.column_index(right_column)?;

    let left_plan: Arc<dyn PhysicalPlan> = Arc::new(DataSource::from_shared(
        Arc::new(left.schema().clone()),
        left.batches().to_vec(),
    ));
    let right_plan: Arc<dyn PhysicalPlan> = Arc::new(DataSource::from_shared(
        Arc::new(right.schema().clone()),
        right.batches().to_vec(),
    ));

    let plan = JoinExec::try_new(left_plan, right_plan, left_idx, right_idx)?;
    let batches = plan.execute()?;
    Ok(Table::from_shared_batches(
        Arc::new(plan.schema().clone()),
        batches,
    ))
}
