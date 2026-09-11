use std::sync::Arc;

use tpt_strata::array::{ArrayRef, F64Array, I32Array, I64Array, StrArray};
use tpt_strata::batch::Batch;
use tpt_strata::engine::{
    Aggregate, AggregateExec, Comparison, DataSource, Expr, FilterExec, JoinExec, LimitExec,
    PhysicalPlan, Predicate, ProjectExec, SortExec, SortExpr,
};
use tpt_strata::format::Table;
use tpt_strata::schema::{DataType, Field, Schema};
use tpt_strata::types::Scalar;

fn emp_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("department", DataType::Utf8, false),
        Field::new("salary", DataType::Float64, false),
        Field::new("years", DataType::Int32, true),
    ]))
}

fn emp_batch() -> Batch {
    let schema = emp_schema();
    Batch::new_unchecked(
        schema,
        vec![
            ArrayRef::Utf8(StrArray::from_nulls(vec![
                Some("eng".into()),
                Some("eng".into()),
                Some("sales".into()),
                Some("sales".into()),
                Some("eng".into()),
            ])),
            ArrayRef::Float64(F64Array::new(vec![
                100000.0, 120000.0, 80000.0, 90000.0, 110000.0,
            ])),
            ArrayRef::Int32(I32Array::from_nulls(vec![
                Some(5),
                None,
                Some(3),
                Some(6),
                Some(7),
            ])),
        ],
    )
}

fn sample_scalar(batch: &Batch, col: usize, row: usize) -> Scalar {
    batch.columns[col].get(row).unwrap()
}

#[test]
fn filter_keeps_matching_rows_only() {
    let input: Arc<dyn PhysicalPlan> = Arc::new(DataSource::from_batch(emp_batch()));
    let pred = Predicate {
        column: 2, // years
        op: Comparison::Gt,
        value: Scalar::I32(4),
    };
    let plan = FilterExec::new(input, pred);
    let out = plan.execute().unwrap();
    let batch = &out[0];
    assert_eq!(batch.row_count(), 3);
    // years values 5, 6, 7 should remain (null filtered out)
    let years = batch.columns[2].get(0).unwrap();
    assert_eq!(years, Scalar::I32(5));
}

#[test]
fn project_selects_columns() {
    let input: Arc<dyn PhysicalPlan> = Arc::new(DataSource::from_batch(emp_batch()));
    let plan = ProjectExec::new(
        input,
        vec![Expr { column: 0 }, Expr { column: 1 }],
        vec!["department".to_string(), "salary".to_string()],
    )
    .unwrap();
    let out = plan.execute().unwrap();
    let batch = &out[0];
    assert_eq!(batch.num_columns(), 2);
    assert_eq!(batch.schema.len(), 2);
    assert_eq!(batch.columns[0].data_type(), DataType::Utf8);
    assert_eq!(batch.columns[1].data_type(), DataType::Float64);
}

#[test]
fn aggregate_group_by_sum_count() {
    let input: Arc<dyn PhysicalPlan> = Arc::new(DataSource::from_batch(emp_batch()));
    let plan = AggregateExec::new(
        input,
        vec![0], // department
        vec![
            Aggregate {
                func: tpt_strata::engine::AggFunc::Sum,
                column: 1,
                out_name: "SUM(salary)".into(),
            },
            Aggregate {
                func: tpt_strata::engine::AggFunc::Count,
                column: 0,
                out_name: "COUNT(department)".into(),
            },
        ],
    )
    .unwrap();
    let out = plan.execute().unwrap();
    let batch = &out[0];
    assert_eq!(batch.row_count(), 2); // eng + sales
                                      // Locate each group's row by its key value.
    let depts = &batch.columns[0];
    let sums = &batch.columns[1];
    let counts = &batch.columns[2];
    let mut eng_sum = None;
    let mut sales_sum = None;
    let mut eng_count = None;
    let mut sales_count = None;
    for i in 0..batch.row_count() {
        match depts.get(i).unwrap() {
            Scalar::Str(s) if s == "eng" => {
                eng_sum = sums.get(i);
                eng_count = counts.get(i);
            }
            Scalar::Str(s) if s == "sales" => {
                sales_sum = sums.get(i);
                sales_count = counts.get(i);
            }
            other => panic!("unexpected group key {other:?}"),
        }
    }
    assert_eq!(eng_sum, Some(Scalar::F64(330000.0)));
    assert_eq!(sales_sum, Some(Scalar::F64(170000.0)));
    assert_eq!(eng_count, Some(Scalar::I64(3)));
    assert_eq!(sales_count, Some(Scalar::I64(2)));
    // sample_scalar unused-guard
    let _ = sample_scalar;
}

#[test]
fn aggregate_global_avg() {
    let input: Arc<dyn PhysicalPlan> = Arc::new(DataSource::from_batch(emp_batch()));
    let plan = AggregateExec::new(
        input,
        vec![],
        vec![Aggregate {
            func: tpt_strata::engine::AggFunc::Avg,
            column: 1,
            out_name: "AVG(salary)".into(),
        }],
    )
    .unwrap();
    let out = plan.execute().unwrap();
    let batch = &out[0];
    assert_eq!(batch.row_count(), 1);
    assert_eq!(batch.columns[0].get(0).unwrap(), Scalar::F64(100000.0));
}

#[test]
fn aggregate_min_max_preserve_source_type() {
    let input: Arc<dyn PhysicalPlan> = Arc::new(DataSource::from_batch(emp_batch()));
    let plan = AggregateExec::new(
        input,
        vec![0],
        vec![
            Aggregate {
                func: tpt_strata::engine::AggFunc::Min,
                column: 1,
                out_name: "MIN(salary)".into(),
            },
            Aggregate {
                func: tpt_strata::engine::AggFunc::Max,
                column: 1,
                out_name: "MAX(salary)".into(),
            },
        ],
    )
    .unwrap();
    let out = plan.execute().unwrap();
    let batch = &out[0];
    let mins = &batch.columns[1];
    let maxs = &batch.columns[2];
    let mut found_eng = false;
    let mut found_sales = false;
    for i in 0..batch.row_count() {
        match batch.columns[0].get(i).unwrap() {
            Scalar::Str(s) if s == "eng" => {
                assert_eq!(mins.get(i).unwrap(), Scalar::F64(100000.0));
                assert_eq!(maxs.get(i).unwrap(), Scalar::F64(120000.0));
                found_eng = true;
            }
            Scalar::Str(s) if s == "sales" => {
                assert_eq!(mins.get(i).unwrap(), Scalar::F64(80000.0));
                assert_eq!(maxs.get(i).unwrap(), Scalar::F64(90000.0));
                found_sales = true;
            }
            _ => {}
        }
    }
    assert!(found_eng && found_sales);
}

#[test]
fn join_equi_join_produces_concatenated_schema() {
    let left_schema = Arc::new(Schema::new(vec![
        Field::new("dept_id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, false),
    ]));
    let left = Batch::new_unchecked(
        left_schema,
        vec![
            ArrayRef::Int32(I32Array::new(vec![1, 2])),
            ArrayRef::Utf8(StrArray::new(vec!["alice".into(), "bob".into()])),
        ],
    );

    let right_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("dept_name", DataType::Utf8, false),
    ]));
    let right = Batch::new_unchecked(
        right_schema,
        vec![
            ArrayRef::Int32(I32Array::new(vec![1, 2, 3])),
            ArrayRef::Utf8(StrArray::new(vec![
                "engineering".into(),
                "sales".into(),
                "ops".into(),
            ])),
        ],
    );

    let left_plan: Arc<dyn PhysicalPlan> = Arc::new(DataSource::from_batch(left));
    let right_plan: Arc<dyn PhysicalPlan> = Arc::new(DataSource::from_batch(right));
    let plan = JoinExec::new(left_plan, right_plan, 0, 0);

    assert_eq!(plan.schema().len(), 4);

    let out = plan.execute().unwrap();
    // Build-up: 2 left rows each match one right row.
    let total_rows: usize = out.iter().map(|b| b.row_count()).sum();
    assert_eq!(total_rows, 2);
    // First output row: dept_id=1, name=alice, id=1, dept_name=engineering
    let first = &out[0];
    assert_eq!(first.columns[0].get(0).unwrap(), Scalar::I32(1));
    assert_eq!(
        first.columns[1].get(0).unwrap(),
        Scalar::Str("alice".into())
    );
    assert_eq!(
        first.columns[3].get(0).unwrap(),
        Scalar::Str("engineering".into())
    );
}

#[test]
fn sort_orders_ascending_and_descending() {
    let input: Arc<dyn PhysicalPlan> = Arc::new(DataSource::from_batch(emp_batch()));
    let plan = SortExec::new(
        input,
        vec![SortExpr {
            column: 1,
            desc: true,
        }],
    );
    let out = plan.execute().unwrap();
    let batch = &out[0];
    assert_eq!(sample_scalar(batch, 1, 0), Scalar::F64(120000.0));
    assert_eq!(sample_scalar(batch, 1, 1), Scalar::F64(110000.0));
    assert_eq!(sample_scalar(batch, 1, 2), Scalar::F64(100000.0));
}

#[test]
fn limit_truncates_rows() {
    let input: Arc<dyn PhysicalPlan> = Arc::new(DataSource::from_batch(emp_batch()));
    let plan = LimitExec::new(input, 2);
    let out = plan.execute().unwrap();
    let batch = &out[0];
    assert_eq!(batch.row_count(), 2);
}

#[test]
fn chained_filter_sort_limit() {
    let input: Arc<dyn PhysicalPlan> = Arc::new(DataSource::from_batch(emp_batch()));
    let filter = FilterExec::new(
        input,
        Predicate {
            column: 1,
            op: Comparison::Gt,
            value: Scalar::F64(90000.0),
        },
    );
    let sort = SortExec::new(
        Arc::new(filter),
        vec![SortExpr {
            column: 1,
            desc: false,
        }],
    );
    let limit = LimitExec::new(Arc::new(sort), 2);
    let out = limit.execute().unwrap();
    let batch = &out[0];
    assert_eq!(batch.row_count(), 2);
    assert_eq!(sample_scalar(batch, 1, 0), Scalar::F64(100000.0));
    assert_eq!(sample_scalar(batch, 1, 1), Scalar::F64(110000.0));
}

#[test]
fn builder_matches_engine_results() {
    let table = Table::try_new(vec![
        tpt_strata::format::Column::new(
            "department",
            vec![Scalar::Str("eng".into()), Scalar::Str("sales".into())],
        ),
        tpt_strata::format::Column::new("salary", vec![Scalar::F64(100.0), Scalar::F64(50.0)]),
    ])
    .unwrap();

    let result = tpt_strata::query::QueryBuilder::new(&table)
        .aggregate("salary", tpt_strata::query::AggFunc::Sum)
        .unwrap()
        .execute()
        .unwrap();
    assert_eq!(result.row_count(), 1);
    assert_eq!(
        result.batches()[0].columns[0].get(0).unwrap(),
        Scalar::F64(150.0)
    );
}

#[test]
fn i64_arrays_round_trip() {
    let arr: ArrayRef = I64Array::from_nulls(vec![Some(7), None, Some(9)]).into();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr.get(1), None);
    assert_eq!(arr.get(2), Some(Scalar::I64(9)));
    let taken = arr.take(&[0, 2]);
    assert_eq!(taken.len(), 2);
    assert_eq!(Scalar::I64(7), taken.get(0).unwrap());
}
