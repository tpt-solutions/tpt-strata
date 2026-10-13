//! `LIMIT` pulls batches lazily: once it has enough rows, it stops reading its
//! input. Operators that need all their input still return the full result.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tpt_strata::array::{ArrayRef, I64Array};
use tpt_strata::batch::{Batch, BatchRef};
use tpt_strata::engine::{
    BatchStream, Comparison, Expr, FilterExec, LimitExec, PhysicalPlan, Predicate, ProjectExec,
};
use tpt_strata::error::QueryError;
use tpt_strata::schema::{DataType, Field, Schema};
use tpt_strata::types::Scalar;

/// An input that counts how many batches have been pulled from it.
struct Counted {
    schema: Arc<Schema>,
    batches: Vec<BatchRef>,
    pulled: Arc<AtomicUsize>,
}

impl PhysicalPlan for Counted {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn open(&self) -> Result<BatchStream, QueryError> {
        let pulled = self.pulled.clone();
        Ok(Box::new(self.batches.clone().into_iter().map(move |b| {
            pulled.fetch_add(1, Ordering::SeqCst);
            Ok(b)
        })))
    }
}

/// Five batches of two rows each: values 0..10.
fn input() -> (Arc<Counted>, Arc<AtomicUsize>) {
    let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Int64, false)]));
    let batches = (0..5)
        .map(|i| {
            let col = ArrayRef::Int64(I64Array::from_nulls(vec![Some(2 * i), Some(2 * i + 1)]));
            Arc::new(Batch::new_unchecked(schema.clone(), vec![col]))
        })
        .collect();
    let pulled = Arc::new(AtomicUsize::new(0));
    (
        Arc::new(Counted {
            schema,
            batches,
            pulled: pulled.clone(),
        }),
        pulled,
    )
}

fn values(batches: &[BatchRef]) -> Vec<i64> {
    let mut v = Vec::new();
    for b in batches {
        for r in 0..b.row_count() {
            if let Some(Scalar::I64(n)) = b.columns[0].get(r) {
                v.push(n);
            }
        }
    }
    v
}

#[test]
fn limit_stops_pulling_once_it_has_enough_rows() {
    let (src, pulled) = input();
    let plan = LimitExec::new(src, 3);
    let out = plan.execute().unwrap();
    assert_eq!(values(&out), vec![0, 1, 2]);
    // Three rows need two batches; the third batch is never read.
    assert_eq!(pulled.load(Ordering::SeqCst), 2);
}

#[test]
fn limit_through_a_filter_and_projection_still_short_circuits() {
    let (src, pulled) = input();
    // Drop the value 1, project, then take 2 rows: 0 from batch one, 2 from batch two.
    let filtered = FilterExec::new(
        src,
        Predicate {
            column: 0,
            op: Comparison::NotEq,
            value: Scalar::I64(1),
        },
    );
    let projected = ProjectExec::new(
        Arc::new(filtered),
        vec![Expr { column: 0 }],
        vec!["v".into()],
    )
    .unwrap();
    let plan = LimitExec::new(Arc::new(projected), 2);
    let out = plan.execute().unwrap();
    assert_eq!(values(&out), vec![0, 2]);
    assert_eq!(pulled.load(Ordering::SeqCst), 2);
}

#[test]
fn limit_zero_reads_nothing_and_returns_an_empty_batch() {
    let (src, pulled) = input();
    let out = LimitExec::new(src, 0).execute().unwrap();
    assert_eq!(out.iter().map(|b| b.row_count()).sum::<usize>(), 0);
    assert_eq!(pulled.load(Ordering::SeqCst), 0);
}

#[test]
fn offset_skips_rows_across_batches() {
    let (src, _) = input();
    let out = LimitExec::with_offset(src, 3, 4).execute().unwrap();
    assert_eq!(values(&out), vec![3, 4, 5, 6]);
}
