fn main() {
    // Rtools42's x86_64-w64-mingw32.static.posix toolchain (used by DuckDB CI for
    // windows_amd64_mingw) does not ship libgcc_eh.a, but Rust's target spec for
    // x86_64-pc-windows-gnu unconditionally passes -lgcc_eh to the linker.
    //
    // Fix: write an empty libgcc_eh.a (just the ar magic header) into OUT_DIR and
    // add it to the link search path. With panic=abort (see .cargo/config.toml),
    // no GCC exception-handling symbols are referenced, so an empty archive works.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu")
    {
        let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
        let lib = std::path::Path::new(&out_dir).join("libgcc_eh.a");
        // A valid GNU ar archive consists of exactly this 8-byte magic string
        // followed by zero entries. GNU ld accepts it as an empty library.
        std::fs::write(&lib, b"!<arch>\n").expect("failed to write dummy libgcc_eh.a");
        println!("cargo:rustc-link-search=native={out_dir}");
    }
}
