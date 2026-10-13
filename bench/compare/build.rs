// DuckDB's bundled build on Windows calls the Windows Restart Manager API,
// which lives in rstrtmgr.lib. Link it explicitly for the benchmark binary.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=rstrtmgr");
    }
}
