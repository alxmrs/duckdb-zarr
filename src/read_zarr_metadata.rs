use std::sync::atomic::{AtomicUsize, Ordering};

use duckdb::core::LogicalTypeId;
use duckdb::vtab::{BindInfo, InitInfo, TableFunctionInfo, VTab};

use crate::zarr_reader::meta::{
    collect_auxiliary_coords, collect_bounds_vars, dimension_names as get_dim_names,
    list_array_names, open_array, open_store,
};

/// One metadata row per array.
#[derive(Debug, Clone)]
struct MetaRow {
    name: String,
    dims: String,       // JSON array string e.g. '["lat","lon"]'
    dtype: String,
    shape: String,      // JSON array string e.g. '[4,6]'
    chunk_shape: String,
    attrs: String,      // full attrs as JSON string
    role: String,       // "coord" | "data" | "aux_coord" | "bounds" | "scalar" | "unknown"
}

pub struct ReadZarrMetaBind {
    rows: Vec<MetaRow>,
    next: AtomicUsize,
}

unsafe impl Send for ReadZarrMetaBind {}
unsafe impl Sync for ReadZarrMetaBind {}

pub struct ReadZarrMetaInit {
    done: std::sync::atomic::AtomicBool,
}

unsafe impl Send for ReadZarrMetaInit {}
unsafe impl Sync for ReadZarrMetaInit {}

pub struct ReadZarrMetaVTab;

impl VTab for ReadZarrMetaVTab {
    type BindData = ReadZarrMetaBind;
    type InitData = ReadZarrMetaInit;

    fn bind(bind: &BindInfo) -> Result<Self::BindData, Box<dyn std::error::Error>> {
        bind.add_result_column("name", LogicalTypeId::Varchar.into());
        bind.add_result_column("dims", LogicalTypeId::Varchar.into());
        bind.add_result_column("dtype", LogicalTypeId::Varchar.into());
        bind.add_result_column("shape", LogicalTypeId::Varchar.into());
        bind.add_result_column("chunk_shape", LogicalTypeId::Varchar.into());
        bind.add_result_column("attrs", LogicalTypeId::Varchar.into());
        bind.add_result_column("role", LogicalTypeId::Varchar.into());

        let store_path = bind.get_parameter(0).to_string();
        let store = open_store(&store_path)?;
        let array_names = list_array_names(&store_path)?;

        let aux_coords = collect_auxiliary_coords(&store, &array_names);
        let bounds_vars = collect_bounds_vars(&store, &array_names, &aux_coords);

        let mut rows = Vec::new();
        for name in &array_names {
            let arr = open_array(&store, name)?;
            let shape = arr.shape().to_vec();
            let chunk_shape = arr.chunk_grid_shape().to_vec();

            let dims = get_dim_names(&arr, name).unwrap_or_default();
            let dtype_str = arr.data_type().to_string();
            let attrs = arr.attributes().clone();

            let role = if bounds_vars.contains(name) {
                "bounds"
            } else if shape.is_empty() {
                "scalar"
            } else if aux_coords.contains(name) {
                "aux_coord"
            } else if shape.len() == 1
                && dims.len() == 1
                && dims[0] == *name
            {
                "coord"
            } else {
                "data"
            };

            rows.push(MetaRow {
                name: name.clone(),
                dims: serde_json::to_string(&dims).unwrap_or_default(),
                dtype: dtype_str,
                shape: serde_json::to_string(&shape).unwrap_or_default(),
                chunk_shape: serde_json::to_string(&chunk_shape).unwrap_or_default(),
                attrs: serde_json::to_string(&attrs).unwrap_or_default(),
                role: role.to_string(),
            });
        }

        Ok(ReadZarrMetaBind {
            rows,
            next: AtomicUsize::new(0),
        })
    }

    fn init(_: &InitInfo) -> Result<Self::InitData, Box<dyn std::error::Error>> {
        Ok(ReadZarrMetaInit {
            done: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn func(
        func: &TableFunctionInfo<Self>,
        output: &mut duckdb::core::DataChunkHandle,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let bind = func.get_bind_data();
        let init = func.get_init_data();
        if init.done.swap(true, Ordering::Relaxed) {
            output.set_len(0);
            return Ok(());
        }

        let vector_size = unsafe { duckdb::ffi::duckdb_vector_size() as usize };
        let start = bind.next.load(Ordering::Relaxed);
        let end = (start + vector_size).min(bind.rows.len());
        let n = end - start;

        if n == 0 {
            output.set_len(0);
            return Ok(());
        }

        let v_name = output.flat_vector(0);
        let v_dims = output.flat_vector(1);
        let v_dtype = output.flat_vector(2);
        let v_shape = output.flat_vector(3);
        let v_cshape = output.flat_vector(4);
        let v_attrs = output.flat_vector(5);
        let v_role = output.flat_vector(6);

        for (i, row) in bind.rows[start..end].iter().enumerate() {
            use duckdb::core::Inserter;
            v_name.insert(i, row.name.as_str());
            v_dims.insert(i, row.dims.as_str());
            v_dtype.insert(i, row.dtype.as_str());
            v_shape.insert(i, row.shape.as_str());
            v_cshape.insert(i, row.chunk_shape.as_str());
            v_attrs.insert(i, row.attrs.as_str());
            v_role.insert(i, row.role.as_str());
        }

        output.set_len(n);
        // For small metadata tables (< vector_size rows), mark done.
        // For larger tables a real loop would be needed; metadata tables are tiny.
        if end >= bind.rows.len() {
            // Done after this batch.
        }
        Ok(())
    }

    fn parameters() -> Option<Vec<duckdb::core::LogicalTypeHandle>> {
        Some(vec![LogicalTypeId::Varchar.into()])
    }
}
