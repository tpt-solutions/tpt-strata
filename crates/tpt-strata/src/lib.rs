//! # tpt-strata
//!
//! An embeddable columnar query engine with zero external dependencies.
//!
//! tpt-strata provides a builder-style API and SQL surface for running
//! analytical queries over in-memory columnar data, without requiring
//! callers to implement execution-plan traits for common cases.
//!
//! ## Quick Start
//!
//! ```rust
//! use tpt_strata::{Table, Scalar, QueryBuilder, AggFunc, print_table};
//!
//! let table = Table::try_new(vec![
//!     tpt_strata::Column::new("department", vec![Scalar::Str("eng".into()), Scalar::Str("sales".into())]),
//!     tpt_strata::Column::new("salary", vec![Scalar::F64(100000.0), Scalar::F64(80000.0)]),
//! ]).unwrap();
//!
//! let result = QueryBuilder::new(&table)
//!     .group_by(vec!["department"]).unwrap()
//!     .aggregate("salary", AggFunc::Avg).unwrap()
//!     .execute().unwrap();
//!
//! print_table(&result);
//! ```

pub mod array;
pub mod batch;
pub mod engine;
pub mod error;
pub mod format;
pub mod query;
pub mod schema;
pub mod sql;
pub mod types;

pub use array::{Array, ArrayRef, BoolArray, F64Array, I32Array, I64Array, StrArray};
pub use batch::Batch;
pub use engine::AggFunc as EngineAggFunc;
pub use engine::PhysicalPlan;
pub use error::QueryError;
pub use format::{print_table, Column, Table};
pub use query::{join, AggFunc, QueryBuilder};
pub use schema::{DataType, Field, Schema};
pub use types::Scalar;
