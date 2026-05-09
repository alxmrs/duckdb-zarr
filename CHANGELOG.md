# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

### Added

**Phase 2 — Zarr v2, Blosc, replacement scan, projection pushdown**
- Zarr v2 + Blosc/LZ4 codec support (#66)
- Replacement scan: bare `.zarr` paths and directories with `zarr.json`/`.zgroup` rewrite to `read_zarr(...)` automatically (#67, #9)
- Projection pushdown: non-requested data variables skip decompression; coord arrays always pre-loaded (#68, #6)

**Phase 1 — MVP table functions**
- `read_zarr` table function: bind, init, scan; single dim group; `_FillValue` → NULL (#5)
- `read_zarr_metadata` table function: per-array name, dims, dtype, shape, chunk_shape, attrs, role (#4)
- `read_zarr_groups` table function: lists dim groups with dims, shape, chunk_shape (#37)
- Schema inference: classify coord vs data via xarray dim metadata; map Zarr dtypes to DuckDB LogicalType (#3)
- v0.1 SQLLogicTest suite + synthetic Zarr v3 fixture generator (#7)
- v0.2 SQLLogicTest suite: v2 stores, codecs, replacement scan, all xarray tutorial fixtures (#13)

**Phase 0 — Design and spike**
- Design doc for native DuckDB Zarr integration (xarray-sql parity) (#1)
- Spike: verified duckdb-rs replacement-scan, ATTACH, dict-vector, config-var, and pushdown APIs (#24)
- Bootstrap: renamed crate to `duckdb_zarr`, dropped `rusty_quack` stub, added zarrs + ndarray deps (#2)
- Python+xarray test fixture generator covering 11 Zarr v3 test cases (#23)
- zarrs 0.23 Cargo feature audit (#27)

### Fixed

- `copy_scalar!` macro used `from_le_bytes` but zarrs returns native-endian bytes (#75)
- `read_zarr_metadata` paginated scan silently truncated stores with >2048 arrays (#76)
- `read_zarr_metadata` `chunk_shape` column reported chunk-grid dimensions, not per-chunk element shape (#74)
- Float sentinel comparison now uses exact equality per CF §2.5.1 (not `f64::EPSILON` band) (#86)
- Dead code `let _ = n` in `load_coord_array` replaced with `debug_assert_eq!` (#85)
- `decode_work_unit` now reuses the cached `FilesystemStore` from `ReadZarrBind` instead of reopening per chunk (#79)
- `missing_value` CF attribute now used as NULL sentinel when `_FillValue` is NaN/absent (#41)
- Implicit (missing) chunks in sparse Zarr stores now filled with `_FillValue`, not crash (#31)
- `scale_factor`/`add_offset` packed-integer decoding now requires integer on-disk dtype (#57)
- Base64-encoded `_FillValue` in zarr attrs decoded correctly (#56)
- Scalar (0-dim) coordinate variables excluded from row schema (#43)
- CF bounds variables (`time_bnds`, `lat_bnds`) suppressed from dim-group schema (#42)
- 2D non-dimension auxiliary coordinates (`xc`, `yc`) silently excluded from row schema (#64)
- Intra-group chunk shape mismatch now reported as bind error (#40)
- `dimension_names` read from `zarr.json` metadata field (not attrs) for Zarr v3 (#46)
- `list_array_names` handles nested sub-groups at store root without confusing errors (#83)
- Coordinate-only dimension with no backing array synthesizes integer range (#44)
- `read_zarr_metadata` unit struct init; no per-call state (#84)
- Blosc snappy_src build failure on macOS Tahoe resolved (#27)
- SQLLogicTest missing_value coverage: exact null counts for basin_mask and ersstv5 sst (#82)
- Data-variable `ZarrArray` objects pre-opened at bind time; `decode_work_unit` no longer calls `Array::open` per chunk (#96)
- SQLLogicTest projection pushdown coverage: column subset selection with value validation (#90)
- Blosc/LZ4 fixture (`blosc_compressed.zarr`) and SQLLogicTest: end-to-end codec pipeline verification (#92)
- Replacement scan SQLLogicTest: trailing slash, non-existent path error, and multi-group error path (#97)
- `ColumnDef.dim_idx: Option<usize>` replaces fragile `dim_col_k` counter in `fill_chunk_slice` (#81)

### Changed
- Entrypoint min API version updated from `"v1.5.2"` to `"v1.2.0"` (matches duckdb-rs 1.10502.0 default) (#93)
- 2D non-dim coordinate arrays are silently excluded (not a bind error) — matches xarray behavior (#78, #95)
- `duckdb_vector_size()` queried at runtime instead of hardcoded `STANDARD_VECTOR_SIZE=2048` (#65)
- CHANGELOG Phase 1 entries reclassified from Changed to Added (#62, #80)
