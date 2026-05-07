# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

### Added
- Zarr v2 + Blosc codec support (#66)
- Replacement scan: .zarr path interception (#67)
- Projection pushdown: skip non-requested columns (#68)
- Design doc for native DuckDB Zarr integration (xarray-sql parity) (#1)
- Spike: verified duckdb-rs replacement-scan, ATTACH, dict-vector, config-var, and pushdown APIs (#24)
- Bootstrap: renamed crate to duckdb_zarr, dropped rusty_quack stub, added zarrs + ndarray deps (#2)
- zarrs 0.23 Cargo feature verification; blosc excluded on macOS Tahoe due to snappy_src build failure (#27)
- Python+xarray test fixture generator covering 11 Zarr v3 test cases (#23)

### Fixed

### Changed
- Zarr v2 support + Blosc/LZ4 codec features (#8)
- Replacement scan: claim *.zarr paths via suffix + metadata probe (#9)
- Projection pushdown: only decode requested data variables (#6)
- v0.2 SQLLogicTest coverage: v2 stores, codecs, replacement scan (#13)
- Replacement scan: handle trailing slash + uppercase .ZARR + URL-encoded paths (#28)
- implement projection pushdown in read_zarr VTab (#69)
- Decision 6 'rarely observed in practice' is wrong: real tutorial data has intra-group chunk mismatches (#40)
- dimension_names in Zarr v3 is in zarr.json, NOT in array attrs — schema inference must read zarr metadata, not attrs (#46)
- Coordinate-only dimension (no backing array): tiny dataset has dim_0 with no coord variable (#44)
- Scalar (0-dim) coordinate variables not addressed in design: ROMS has physical constants as Zarr scalars (#43)
- CF bounds variables (time_bnds, lat_bnds) create spurious dim groups and confuse schema (#42)
- missing_value CF attribute not handled: cells won't be NULLed when only missing_value is present (#41)
- CORRECTNESS: scale_factor/add_offset packed integers not decoded — silently wrong data values (#39)
- Adversarial review: xarray fixture generation + design gap analysis (#38)
- read_zarr_groups table function (#37)
- Endianness: big-endian Zarr arrays (>f4, >i4) must round-trip correctly through DuckDB DataChunk fill (#36)
- Design gap: coordinate arrays in bind are loaded for ALL dims, but coord cache key is unclear when coord appears in multiple dim groups (#35)
- Implicit (missing) chunks in sparse Zarr stores: fill with _FillValue, not crash (#31)
- read_zarr_metadata schema: group rows have no dtype/shape/chunks — schema inconsistency (#30)
- Multi-variable mismatched chunk shapes: design gap in init/scan phase (#29)
- v0.1 SQLLogicTest suite + synthetic Zarr v3 fixture generator (#7)
- read_zarr table function: bind, init, scan (single dim group, _FillValue → NULL) (#5)
- read_zarr_metadata table function (#4)
- Schema inference: classify coords vs data via xarray dim metadata, map Zarr dtypes to DuckDB LogicalType (#3)
- Scan hardcodes STANDARD_VECTOR_SIZE=2048 instead of querying DataChunk capacity (#65)
- Design doesn't specify how to classify rasm's xc/yc when coordinates attr is present but those arrays also appear as dim-group candidates (#64)
- No fixture tests multi-dim-group store — core 'one table per dim group' feature is untestable (#63)
- CHANGELOG: all phase-0 entries are under Changed instead of Added (#62)
- air_temperature_gradient: Tair chunk size [1,25,53] generates 2920 files — needlessly large fixture for a bind-error test (#61)
- air_temperature fixture is int16 packed, not float64 — comment is wrong, no plain-float baseline exists (#60)
- Missing big-endian fixture — design explicitly requires it, never generated (#59)
- Missing sparse/implicit chunk fixture — design's fill-value-for-missing-chunks code path untestable (#58)
- scale_factor packed-decoding must require integer dtype, not just presence of attr (#57)
- Base64-encoded _FillValue in zarr attrs: design gap and fixture reality (#56)
