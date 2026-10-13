//! `strata`: run SQL over Parquet and CSV files from the command line.

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::ExitCode;

use tpt_strata::csv::{parse_csv, to_csv};
use tpt_strata::format::render_table;
use tpt_strata::json::parse_json;
use tpt_strata::sql::SqlContext;
use tpt_strata::Table;

const USAGE: &str = "\
strata: run SQL over Parquet and CSV files

USAGE:
    strata <FILE>... [SQL] [OPTIONS]

Each FILE becomes a table named after the file name without its extension
(characters other than letters, digits and '_' become '_'). With no SQL
argument, strata starts a prompt that reads one query per line.

OPTIONS:
    --csv        print results as CSV instead of a grid
    --explain    print the physical plan instead of running the query
    --check      bind the query against the files without running it
    -h, --help   show this help

EXAMPLES:
    strata sales.parquet \"SELECT region, SUM(amount) AS total FROM sales GROUP BY region\"
    strata orders.csv customers.csv \"SELECT orders.id, customers.name FROM orders JOIN customers ON orders.cust = customers.id\"
";

#[derive(Default)]
struct Options {
    csv: bool,
    explain: bool,
    check: bool,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("strata: {msg}");
            ExitCode::from(2)
        }
    }
}

/// Parse arguments and dispatch. Usage problems return `Err` (exit code 2).
/// Query problems are reported by [`execute`] and set exit code 1.
fn run(args: &[String]) -> Result<ExitCode, String> {
    let mut opts = Options::default();
    let mut files: Vec<&str> = Vec::new();
    let mut queries: Vec<&str> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--csv" => opts.csv = true,
            "--explain" => opts.explain = true,
            "--check" => opts.check = true,
            flag if flag.starts_with('-') && flag.len() > 1 => {
                return Err(format!("unknown option '{flag}'; see --help"));
            }
            path if is_data_file(path) => files.push(path),
            sql => queries.push(sql),
        }
    }
    if queries.len() > 1 {
        return Err("expected at most one SQL query; quote the query as a single argument".into());
    }
    if files.is_empty() {
        return Err("no input files; pass at least one .parquet or .csv file".into());
    }

    let mut tables: Vec<(String, Table)> = Vec::with_capacity(files.len());
    for path in &files {
        let name = table_name(path);
        if tables.iter().any(|(n, _)| *n == name) {
            return Err(format!(
                "two input files would both be named table '{name}'; rename one of them"
            ));
        }
        let table = load(path)?;
        tables.push((name, table));
    }

    let mut ctx = SqlContext::new();
    for (name, table) in &tables {
        ctx.add_table(name, table);
    }

    match queries.first() {
        Some(sql) => match execute(&ctx, sql, &opts) {
            Ok(()) => Ok(ExitCode::SUCCESS),
            Err(msg) => {
                eprintln!("strata: {msg}");
                Ok(ExitCode::from(1))
            }
        },
        None => Ok(repl(&ctx, &opts)),
    }
}

fn is_data_file(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("csv" | "json" | "parquet" | "parq")
    )
}

/// The SQL table name for a file: its stem, made of identifier characters.
fn table_name(path: &str) -> String {
    let stem = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("data");
    let mut name: String = stem
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.chars().next().is_none_or(|c| c.is_ascii_digit()) {
        name.insert_str(0, "t_");
    }
    name
}

fn load(path: &str) -> Result<Table, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "json" {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("cannot read '{path}': {e}"))?;
        parse_json(&text).map_err(|e| format!("{path}: {e}"))
    } else if ext == "csv" {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("cannot read '{path}': {e}"))?;
        parse_csv(&text).map_err(|e| format!("{path}: {e}"))
    } else {
        tpt_strata_parquet::read_parquet(path).map_err(|e| format!("{path}: {e}"))
    }
}

/// Run one statement in the mode the options select. Errors come back as text.
fn execute(ctx: &SqlContext<'_>, sql: &str, opts: &Options) -> Result<(), String> {
    if opts.check {
        ctx.check(sql).map_err(|e| e.to_string())?;
        println!("OK");
    } else if opts.explain {
        let plan = ctx.explain(sql).map_err(|e| e.to_string())?;
        print!("{plan}");
    } else {
        let table = ctx.run(sql).map_err(|e| e.to_string())?;
        if opts.csv {
            print!("{}", to_csv(&table));
        } else {
            print!("{}", render_table(&table));
        }
    }
    Ok(())
}

/// Read queries from stdin, one per line, reporting errors without exiting.
fn repl(ctx: &SqlContext<'_>, opts: &Options) -> ExitCode {
    eprintln!("strata: enter one query per line; 'exit' or Ctrl-D to quit");
    let stdin = io::stdin();
    loop {
        eprint!("strata> ");
        let _ = io::stderr().flush();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                eprintln!("strata: cannot read input: {e}");
                return ExitCode::from(2);
            }
        }
        let sql = line.trim().trim_end_matches(';').trim();
        if sql.is_empty() {
            continue;
        }
        if sql.eq_ignore_ascii_case("exit") || sql.eq_ignore_ascii_case("quit") {
            break;
        }
        if let Err(msg) = execute(ctx, sql, opts) {
            eprintln!("strata: {msg}");
        }
    }
    ExitCode::SUCCESS
}
