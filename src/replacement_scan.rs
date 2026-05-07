use std::ffi::{CStr, CString};
use std::path::Path;

use duckdb::ffi::{
    duckdb_add_replacement_scan, duckdb_create_varchar, duckdb_database,
    duckdb_replacement_scan_add_parameter, duckdb_replacement_scan_info,
    duckdb_replacement_scan_set_function_name,
};

/// Register the replacement scan on `db` so that bare paths ending in `.zarr`
/// (or containing a `zarr.json` / `.zgroup` at the root) are rewritten to
/// `read_zarr('<path>')`.
pub unsafe fn register(db: duckdb_database) {
    unsafe {
        duckdb_add_replacement_scan(db, Some(zarr_replacement_scan), std::ptr::null_mut(), None);
    }
}

unsafe extern "C" fn zarr_replacement_scan(
    info: duckdb_replacement_scan_info,
    table_name: *const std::os::raw::c_char,
    _data: *mut std::os::raw::c_void,
) {
    unsafe {
        let name = match CStr::from_ptr(table_name).to_str() {
            Ok(s) => s,
            Err(_) => return,
        };

        if !looks_like_zarr(name) {
            return;
        }

        let fn_name = c"read_zarr";
        duckdb_replacement_scan_set_function_name(info, fn_name.as_ptr());

        let path_cstr = match CString::new(name) {
            Ok(s) => s,
            Err(_) => return,
        };
        let val = duckdb_create_varchar(path_cstr.as_ptr());
        duckdb_replacement_scan_add_parameter(info, val);
    }
}

fn looks_like_zarr(name: &str) -> bool {
    // Strip trailing slashes for the suffix check.
    let trimmed = name.trim_end_matches('/');

    // Accept explicit .zarr suffix (case-insensitive).
    if trimmed.to_ascii_lowercase().ends_with(".zarr") {
        return true;
    }

    // Also accept bare directory paths that contain a Zarr root marker.
    let p = Path::new(trimmed);
    p.join("zarr.json").exists() || p.join(".zgroup").exists()
}
