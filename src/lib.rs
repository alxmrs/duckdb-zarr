use duckdb::{duckdb_entrypoint_c_api, Connection, Result};
use std::error::Error;

mod zarr_reader;

#[duckdb_entrypoint_c_api()]
pub unsafe fn extension_entrypoint(_con: Connection) -> Result<(), Box<dyn Error>> {
    Ok(())
}
