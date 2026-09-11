//! Read a Parquet file into a tpt-strata table and print it.
//!
//! With a path argument it reads that file; without one it writes a small
//! sample file to the system temp directory and loads that, so the example is
//! runnable out of the box:
//!
//! ```sh
//! cargo run -p tpt-strata-parquet --example read_parquet
//! cargo run -p tpt-strata-parquet --example read_parquet -- path/to/file.parquet
//! ```

use std::sync::Arc;

use arrow::array::{Float64Array, StringArray};
use arrow::datatypes::{DataType as ArrowType, Field as ArrowField, Schema as ArrowSchema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;

use tpt_strata::print_table;
use tpt_strata_parquet::read_parquet;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (path, remove_after) = match std::env::args().nth(1) {
        Some(p) => (std::path::PathBuf::from(p), false),
        None => {
            let tmp = std::env::temp_dir().join("tpt-strata-example.parquet");
            write_sample(&tmp)?;
            (tmp, true)
        }
    };

    let table = read_parquet(&path)?;
    if remove_after {
        let _ = std::fs::remove_file(&path);
    }

    println!(
        "Loaded {} rows x {} columns from {}",
        table.row_count(),
        table.schema().len(),
        path.display()
    );
    print_table(&table);
    Ok(())
}

/// Write a tiny Parquet file so the example has something to read.
fn write_sample(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let schema = Arc::new(ArrowSchema::new(vec![
        ArrowField::new("service", ArrowType::Utf8, false),
        ArrowField::new("region", ArrowType::Utf8, true),
        ArrowField::new("latency_ms", ArrowType::Float64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["auth", "auth", "billing"])),
            Arc::new(StringArray::from(vec![Some("east"), None, Some("west")])),
            Arc::new(Float64Array::from(vec![Some(12.5), Some(18.0), None])),
        ],
    )?;

    let file = std::fs::File::create(path)?;
    let mut writer = ArrowWriter::try_new(file, schema, None)?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}
