use duckdb::{Connection, Result, duckdb_entrypoint_c_api};
use std::error::Error;

mod read_zarr;
mod read_zarr_groups;
mod read_zarr_metadata;
mod zarr_reader;

#[duckdb_entrypoint_c_api()]
pub unsafe fn extension_entrypoint(con: Connection) -> Result<(), Box<dyn Error>> {
    con.register_table_function::<read_zarr::ReadZarrVTab>("read_zarr")?;
    con.register_table_function::<read_zarr_metadata::ReadZarrMetaVTab>("read_zarr_metadata")?;
    con.register_table_function::<read_zarr_groups::ReadZarrGroupsVTab>("read_zarr_groups")?;
    Ok(())
}
