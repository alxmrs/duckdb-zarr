fn main() {
    // Rtools42's x86_64-w64-mingw32.static.posix toolchain (used by DuckDB CI for
    // windows_amd64_mingw) does not ship libgcc_eh.a, but Rust's target spec for
    // x86_64-pc-windows-gnu unconditionally passes -lgcc_eh to the linker.
    //
    // Fix: create an empty libgcc_eh.a in OUT_DIR and add it to the link search
    // path. With panic=abort (see .cargo/config.toml), no GCC exception-handling
    // symbols are actually referenced, so an empty archive satisfies the linker.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu")
    {
        create_dummy_libgcc_eh();
    }
}

fn create_dummy_libgcc_eh() {
    use std::path::PathBuf;
    use std::process::Command;

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let out = PathBuf::from(&out_dir);

    let cc = std::env::var("CC").unwrap_or_else(|_| "x86_64-w64-mingw32-gcc".into());
    let ar = std::env::var("AR").unwrap_or_else(|_| "x86_64-w64-mingw32-ar".into());

    let src = out.join("gcc_eh_stub.c");
    std::fs::write(&src, "/* empty stub for -lgcc_eh */\n").unwrap();

    let obj = out.join("gcc_eh_stub.o");
    let ok = Command::new(&cc)
        .args(["-c", src.to_str().unwrap(), "-o", obj.to_str().unwrap()])
        .status()
        .expect("C compiler not found")
        .success();
    assert!(ok, "failed to compile gcc_eh stub");

    let lib = out.join("libgcc_eh.a");
    let ok = Command::new(&ar)
        .args(["rcs", lib.to_str().unwrap(), obj.to_str().unwrap()])
        .status()
        .expect("ar not found")
        .success();
    assert!(ok, "failed to create libgcc_eh.a");

    println!("cargo:rustc-link-search=native={out_dir}");
}
