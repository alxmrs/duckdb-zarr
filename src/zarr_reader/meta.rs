use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use zarrs::array::Array;
use zarrs::filesystem::FilesystemStore;

use super::types::{
    ColumnDef, ColumnEncoding, CoordArray, DimGroup, FillSentinel, WorkUnit, ZarrDtype,
};

pub type ZarrStore = Arc<FilesystemStore>;
pub type ZarrArray = Array<FilesystemStore>;

pub fn open_store(path: &str) -> Result<ZarrStore, Box<dyn std::error::Error>> {
    Ok(Arc::new(FilesystemStore::new(path)?))
}

/// List the names of all top-level arrays in the Zarr store root.
/// A top-level array is a direct child directory that contains `zarr.json`.
pub fn list_array_names(store_path: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let root = Path::new(store_path);
    let mut names = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        if !ft.is_dir() {
            continue;
        }
        let child_path = entry.path();
        if child_path.join("zarr.json").exists() {
            if let Some(name) = child_path.file_name().and_then(|n| n.to_str()) {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

/// Open one array by name from the store.
pub fn open_array(
    store: &ZarrStore,
    name: &str,
) -> Result<ZarrArray, Box<dyn std::error::Error>> {
    let path = format!("/{name}");
    Ok(Array::open(store.clone(), &path)?)
}

/// Resolve dimension names for an array.
/// Priority: zarr v3 `dimension_names` field → `_ARRAY_DIMENSIONS` attr → error.
pub fn dimension_names(
    array: &ZarrArray,
    name: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    // Zarr v3: dimension_names is a first-class field in zarr.json.
    if let Some(dim_names) = array.dimension_names() {
        return Ok(dim_names
            .iter()
            .enumerate()
            .map(|(i, d)| d.as_deref().unwrap_or(&format!("dim_{i}")).to_string())
            .collect());
    }
    // Zarr v2 / OME-Zarr fallback: _ARRAY_DIMENSIONS in attrs.
    let attrs = array.attributes();
    if let Some(serde_json::Value::Array(arr)) = attrs.get("_ARRAY_DIMENSIONS") {
        return Ok(arr
            .iter()
            .enumerate()
            .map(|(i, v)| {
                v.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("dim_{i}"))
            })
            .collect());
    }
    Err(format!("array '{name}' has no dimension_names or _ARRAY_DIMENSIONS").into())
}

/// Parse `ZarrDtype` from the zarrs DataType.
/// `DataType::to_string()` may emit "v3_name / v2_name"; we use the v3 name (first token).
pub fn parse_dtype(
    array: &ZarrArray,
    name: &str,
) -> Result<ZarrDtype, Box<dyn std::error::Error>> {
    let full = array.data_type().to_string();
    let type_str = full.split(" / ").next().unwrap_or(&full);
    ZarrDtype::from_str(type_str)
        .ok_or_else(|| format!("unsupported dtype '{full}' for array '{name}'").into())
}

/// Parse `ColumnEncoding` and `FillSentinel` from CF attrs.
///
/// Packed-int rule: integer on-disk dtype AND (scale_factor OR add_offset in attrs).
pub fn parse_encoding_and_sentinel(
    dtype: &ZarrDtype,
    attrs: &serde_json::Map<String, serde_json::Value>,
) -> (ColumnEncoding, Option<FillSentinel>) {
    let scale = attrs
        .get("scale_factor")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);
    let offset = attrs
        .get("add_offset")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let has_packing = attrs.contains_key("scale_factor") || attrs.contains_key("add_offset");
    let encoding = if dtype.is_integer() && has_packing {
        ColumnEncoding::PackedInt {
            scale_factor: scale,
            add_offset: offset,
        }
    } else {
        ColumnEncoding::Plain
    };

    let sentinel = parse_sentinel(dtype, attrs);
    (encoding, sentinel)
}

fn parse_sentinel(
    dtype: &ZarrDtype,
    attrs: &serde_json::Map<String, serde_json::Value>,
) -> Option<FillSentinel> {
    // _FillValue takes precedence over missing_value (CF conventions §2.5.1).
    if let Some(fv) = attrs.get("_FillValue") {
        return parse_fill_value(dtype, fv);
    }
    if let Some(mv) = attrs.get("missing_value") {
        return parse_fill_value(dtype, mv);
    }
    None
}

fn parse_fill_value(dtype: &ZarrDtype, v: &serde_json::Value) -> Option<FillSentinel> {
    match v {
        // xarray FillValueCoder encodes float _FillValue as base64 LE float64.
        serde_json::Value::String(s) => {
            let bytes = base64::engine::general_purpose::STANDARD.decode(s).ok()?;
            if bytes.len() == 8 {
                let arr: [u8; 8] = bytes.try_into().ok()?;
                Some(FillSentinel::Float(f64::from_le_bytes(arr)))
            } else {
                None
            }
        }
        serde_json::Value::Number(n) => {
            if dtype.is_unsigned() {
                n.as_u64().map(FillSentinel::UInt)
            } else if dtype.is_integer() {
                n.as_i64().map(FillSentinel::Int)
            } else {
                n.as_f64().map(FillSentinel::Float)
            }
        }
        _ => None,
    }
}

/// Fall back to the zarr.json `fill_value` field when no CF sentinel attr is present.
/// Returns None for the all-zero default fill_value so we don't mask legitimate zeros.
fn parse_zarr_fill_sentinel(array: &ZarrArray, dtype: &ZarrDtype) -> Option<FillSentinel> {
    let bytes = array.fill_value().as_ne_bytes();
    if bytes.iter().all(|&b| b == 0) {
        return None;
    }
    match dtype {
        ZarrDtype::Bool => None,
        ZarrDtype::Int8 => Some(FillSentinel::Int(bytes[0] as i8 as i64)),
        ZarrDtype::Int16 => {
            let arr: [u8; 2] = bytes.try_into().ok()?;
            Some(FillSentinel::Int(i16::from_ne_bytes(arr) as i64))
        }
        ZarrDtype::Int32 => {
            let arr: [u8; 4] = bytes.try_into().ok()?;
            Some(FillSentinel::Int(i32::from_ne_bytes(arr) as i64))
        }
        ZarrDtype::Int64 => {
            let arr: [u8; 8] = bytes.try_into().ok()?;
            Some(FillSentinel::Int(i64::from_ne_bytes(arr)))
        }
        ZarrDtype::UInt8 => Some(FillSentinel::UInt(bytes[0] as u64)),
        ZarrDtype::UInt16 => {
            let arr: [u8; 2] = bytes.try_into().ok()?;
            Some(FillSentinel::UInt(u16::from_ne_bytes(arr) as u64))
        }
        ZarrDtype::UInt32 => {
            let arr: [u8; 4] = bytes.try_into().ok()?;
            Some(FillSentinel::UInt(u32::from_ne_bytes(arr) as u64))
        }
        ZarrDtype::UInt64 => {
            let arr: [u8; 8] = bytes.try_into().ok()?;
            Some(FillSentinel::UInt(u64::from_ne_bytes(arr)))
        }
        ZarrDtype::Float32 => {
            let arr: [u8; 4] = bytes.try_into().ok()?;
            Some(FillSentinel::Float(f32::from_ne_bytes(arr) as f64))
        }
        ZarrDtype::Float64 => {
            let arr: [u8; 8] = bytes.try_into().ok()?;
            Some(FillSentinel::Float(f64::from_ne_bytes(arr)))
        }
    }
}

/// Collect the set of non-dimension coord names from all `coordinates` attrs
/// across all arrays. These must be excluded from dim-group classification.
pub fn collect_auxiliary_coords(
    store: &ZarrStore,
    array_names: &[String],
) -> HashSet<String> {
    let mut aux = HashSet::new();
    for name in array_names {
        if let Ok(arr) = open_array(store, name) {
            if let Some(serde_json::Value::String(coords_str)) = arr.attributes().get("coordinates")
            {
                for token in coords_str.split_whitespace() {
                    aux.insert(token.to_string());
                }
            }
        }
    }
    aux
}

/// Determine whether a variable is a CF bounds variable to suppress.
/// Criteria: another array has a `bounds` attr pointing to this name,
/// OR this name matches `<dim>_bnds` / `<dim>_bounds` with shape (N, 2).
pub fn collect_bounds_vars(
    store: &ZarrStore,
    array_names: &[String],
    aux_coords: &HashSet<String>,
) -> HashSet<String> {
    let mut bounds = HashSet::new();

    // Attr-based: bounds = "name" on a coord array.
    for name in array_names {
        if let Ok(arr) = open_array(store, name) {
            if let Some(serde_json::Value::String(b)) = arr.attributes().get("bounds") {
                bounds.insert(b.clone());
            }
        }
    }

    // Name-pattern fallback: *_bnds / *_bounds with shape (N, 2).
    for name in array_names {
        if aux_coords.contains(name) || bounds.contains(name) {
            continue;
        }
        let is_pattern = name.ends_with("_bnds") || name.ends_with("_bounds");
        if !is_pattern {
            continue;
        }
        if let Ok(arr) = open_array(store, name) {
            let shape = arr.shape();
            if shape.len() == 2 && shape[1] == 2 {
                bounds.insert(name.clone());
            }
        }
    }
    bounds
}

/// Infer dim groups from the array set.
///
/// A dim group is a set of arrays sharing an identical ordered dimension list.
/// Coordinates (1-D arrays whose only dim == their name) and bounds vars are
/// excluded from data variables.
///
/// Returns `(dim_groups, coord_names)` where coord_names is the complete set
/// of coordinate array names.
pub fn infer_dim_groups(
    store: &ZarrStore,
    array_names: &[String],
) -> Result<(Vec<DimGroup>, HashSet<String>), Box<dyn std::error::Error>> {
    // Step 1: scan coordinates attr first (must precede dim-group enumeration).
    let aux_coords = collect_auxiliary_coords(store, array_names);
    let bounds_vars = collect_bounds_vars(store, array_names, &aux_coords);

    // Step 2: classify each array as coord or data var.
    //   coord: 1-D, sole dim == array name (dim-coord) OR in aux_coords (non-dim coord)
    //   data var: everything else (excluding bounds and scalar arrays)
    let mut coord_names: HashSet<String> = HashSet::new();
    let mut data_vars: Vec<String> = Vec::new();
    let mut scalar_names: HashSet<String> = HashSet::new();

    for name in array_names {
        if bounds_vars.contains(name) {
            continue;
        }
        let arr = open_array(store, name)?;
        let shape = arr.shape();

        if shape.is_empty() {
            // 0-dim scalar coordinate — suppress from schema.
            scalar_names.insert(name.clone());
            continue;
        }

        if aux_coords.contains(name) {
            coord_names.insert(name.clone());
            continue;
        }

        // Dim-coord heuristic: 1-D array whose sole dim shares its name.
        if shape.len() == 1 {
            if let Ok(dims) = dimension_names(&arr, name) {
                if dims.len() == 1 && dims[0] == *name {
                    coord_names.insert(name.clone());
                    continue;
                }
            }
        }

        data_vars.push(name.clone());
    }

    // Step 3: group data vars by their dim signature.
    let mut groups: HashMap<Vec<String>, DimGroup> = HashMap::new();

    for var_name in &data_vars {
        let arr = open_array(store, var_name)?;
        let dims = dimension_names(&arr, var_name)?;
        let shape = arr.shape().to_vec();

        let ndim = shape.len();
        let first_chunk = vec![0u64; ndim];
        let chunk_shape: Vec<u64> = arr
            .chunk_shape(&first_chunk)?
            .iter()
            .map(|x| x.get())
            .collect();

        // Collect coord names that belong to this dim group (dims that have matching coord arrays).
        let group_coord_names: Vec<String> = dims
            .iter()
            .filter(|d| coord_names.contains(*d))
            .cloned()
            .collect();

        let entry = groups.entry(dims.clone()).or_insert_with(|| DimGroup {
            dims,
            shape,
            chunk_shape: chunk_shape.clone(),
            data_var_names: Vec::new(),
            coord_var_names: group_coord_names,
        });
        // Validate chunk shape consistency within the dim group.
        if entry.chunk_shape != chunk_shape {
            return Err(format!(
                "chunk shape mismatch in dim group {:?}: existing {:?} vs '{var_name}' {:?}",
                entry.dims, entry.chunk_shape, chunk_shape
            )
            .into());
        }
        entry.data_var_names.push(var_name.clone());
    }

    let mut dim_groups: Vec<DimGroup> = groups.into_values().collect();
    dim_groups.sort_by(|a, b| a.dims.cmp(&b.dims));

    Ok((dim_groups, coord_names))
}

/// Pre-load a coordinate array's raw bytes at bind time.
pub fn load_coord_array(
    store: &ZarrStore,
    coord_name: &str,
) -> Result<CoordArray, Box<dyn std::error::Error>> {
    let arr = open_array(store, coord_name)?;
    let dtype = parse_dtype(&arr, coord_name)?;
    let attrs = arr.attributes().clone();
    let (encoding, sentinel) = parse_encoding_and_sentinel(&dtype, &attrs);
    let sentinel = sentinel.or_else(|| parse_zarr_fill_sentinel(&arr, &dtype));
    let shape = arr.shape().to_vec();
    let n = shape[0] as usize;

    // Retrieve the full 1-D array as raw bytes via ArrayBytes::into_fixed().
    let _ = n;
    let subset = arr.subset_all();
    let array_bytes = arr.retrieve_array_subset::<zarrs::array::ArrayBytes<'static>>(&subset)?;
    let raw = array_bytes.into_fixed().map_err(|_| "coord array has variable-length dtype")?;
    let bytes: Vec<u8> = raw.into_owned();

    Ok(CoordArray {
        dtype,
        encoding,
        sentinel,
        bytes,
    })
}

/// Build the list of `WorkUnit`s for one dim group.
pub fn build_work_units(group: &DimGroup) -> Vec<WorkUnit> {
    // Number of chunks per dimension.
    let n_chunks_per_dim: Vec<u64> = group
        .dims
        .iter()
        .enumerate()
        .map(|(i, _)| {
            group.shape[i].div_ceil(group.chunk_shape[i])
        })
        .collect();

    // Total number of chunks.
    let total: u64 = n_chunks_per_dim.iter().product();

    // Generate all chunk index tuples in C (row-major) order.
    let ndim = n_chunks_per_dim.len();
    let mut strides = vec![1u64; ndim];
    for k in (0..ndim.saturating_sub(1)).rev() {
        strides[k] = strides[k + 1] * n_chunks_per_dim[k + 1];
    }

    (0..total)
        .map(|i| {
            let chunk_indices = (0..ndim)
                .map(|k| (i / strides[k]) % n_chunks_per_dim[k])
                .collect();
            WorkUnit { chunk_indices }
        })
        .collect()
}

/// Build `ColumnDef`s for one dim group: dims first, then data vars.
pub fn build_column_defs(
    store: &ZarrStore,
    group: &DimGroup,
    coord_arrays: &HashMap<String, CoordArray>,
) -> Result<Vec<ColumnDef>, Box<dyn std::error::Error>> {
    let mut cols = Vec::new();

    // Dimension columns (coords or synthesized integers).
    for dim in &group.dims {
        if let Some(ca) = coord_arrays.get(dim) {
            cols.push(ColumnDef {
                name: dim.clone(),
                on_disk_dtype: ca.dtype.clone(),
                encoding: ca.encoding.clone(),
                sentinel: ca.sentinel.clone(),
                is_coord: true,
            });
        } else {
            // Unindexed dim → synthesize 0..N integer range (Int64).
            // Mark is_coord=true so decode_work_unit skips it (no zarr array to load).
            cols.push(ColumnDef {
                name: dim.clone(),
                on_disk_dtype: ZarrDtype::Int64,
                encoding: ColumnEncoding::Plain,
                sentinel: None,
                is_coord: true,
            });
        }
    }

    // Data variable columns.
    for var_name in &group.data_var_names {
        let arr = open_array(store, var_name)?;
        let dtype = parse_dtype(&arr, var_name)?;
        let attrs = arr.attributes().clone();
        let (encoding, sentinel) = parse_encoding_and_sentinel(&dtype, &attrs);
        let sentinel = sentinel.or_else(|| parse_zarr_fill_sentinel(&arr, &dtype));
        cols.push(ColumnDef {
            name: var_name.clone(),
            on_disk_dtype: dtype,
            encoding,
            sentinel,
            is_coord: false,
        });
    }

    Ok(cols)
}
