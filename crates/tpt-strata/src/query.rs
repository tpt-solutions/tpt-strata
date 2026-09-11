use std::sync::Arc;

use crate::batch::Batch;
use crate::engine::{
    AggFunc as EngAggFunc, Aggregate, AggregateExec, Comparison, DataSource, Expr, FilterExec,
    JoinExec, LimitExec, PhysicalPlan, Predicate, ProjectExec, SortExec, SortExpr,
};
use crate::error::QueryError;
use crate::format::Table;
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

/// A builder for constructing and executing queries over a [`Table`].
///
/// Zero trait implementations are required for the common case.
#[derive(Debug)]
pub struct QueryBuilder<'a> {
    table: &'a Table,
    predicates: Vec<Predicate>,
    projections: Option<Vec<String>>,
    group_by: Vec<String>,
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
    /// Supported ops: `=`, `!=`, `>`, `>=`, `<`, `<=`.
    /// Type mismatch is checked eagerly at build time.
    pub fn filter(mut self, column: &str, op: &str, value: Scalar) -> Result<Self, QueryError> {
        let idx = self.table.column_index(column)?;
        let field = &self.table.schema().fields[idx];

        // Type check at build time.
        let field_type = field.data_type.name();
        if !matches!(value, Scalar::Null) && value.type_name() != field_type {
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
    pub fn project(mut self, columns: Vec<&str>) -> Result<Self, QueryError> {
        for col in &columns {
            self.table.column_index(col)?;
        }
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

    /// Order results by the specified column (ascending unless `desc`).
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

        let mut plan: Arc<dyn PhysicalPlan> =
            Arc::new(DataSource::new(input_schema, self.table.batches().to_vec()));

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
                    self.table.column_index(col).map(|idx| Aggregate {
                        func: to_engine_func(*func),
                        column: idx,
                        out_name: format!("{func}({col})"),
                    })
                })
                .collect::<Result<_, _>>()?;
            plan = Arc::new(AggregateExec::new(plan, group_indices, aggregates)?);
        } else if let Some(projections) = self.projections.as_ref() {
            let exprs: Vec<Expr> = projections
                .iter()
                .map(|name| {
                    self.table
                        .column_index(name)
                        .map(|idx| Expr { column: idx })
                })
                .collect::<Result<_, _>>()?;
            let out_names = projections.clone();
            plan = Arc::new(ProjectExec::new(plan, exprs, out_names)?);
        }

        if let Some((col, desc)) = self.order_by.as_ref() {
            // The order column must be resolved against the current output schema.
            let schema = plan.schema();
            let out_idx = if let Some(i) = schema.index_of(col) {
                Some(i)
            } else {
                // Allow matching against aggregate output names / suffix matches.
                schema
                    .fields
                    .iter()
                    .position(|f| f.name == *col || f.name.contains(col.as_str()))
            };
            if let Some(idx) = out_idx {
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

        let batches: Vec<Batch> = plan.execute()?;
        Ok(Table::from_batches(
            Arc::new(plan.schema().clone()),
            batches,
        ))
    }
}

/// Build and execute an equi-join between two tables.
pub fn join(
    left: &Table,
    right: &Table,
    left_column: &str,
    right_column: &str,
) -> Result<Table, QueryError> {
    let left_idx = left.column_index(left_column)?;
    let right_idx = right.column_index(right_column)?;

    let left_plan: Arc<dyn PhysicalPlan> = Arc::new(DataSource::new(
        Arc::new(left.schema().clone()),
        left.batches().to_vec(),
    ));
    let right_plan: Arc<dyn PhysicalPlan> = Arc::new(DataSource::new(
        Arc::new(right.schema().clone()),
        right.batches().to_vec(),
    ));

    let plan = JoinExec::new(left_plan, right_plan, left_idx, right_idx);
    let batches = plan.execute()?;
    Ok(Table::from_batches(
        Arc::new(plan.schema().clone()),
        batches,
    ))
}
