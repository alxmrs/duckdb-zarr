# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

### Added
- Design doc for native DuckDB Zarr integration (xarray-sql parity) (#1)
- Spike: verified duckdb-rs replacement-scan, ATTACH, dict-vector, config-var, and pushdown APIs (#24)
- Bootstrap: renamed crate to duckdb_zarr, dropped rusty_quack stub, added zarrs + ndarray deps (#2)
- zarrs 0.23 Cargo feature verification; blosc excluded on macOS Tahoe due to snappy_src build failure (#27)
- Python+xarray test fixture generator covering 11 Zarr v3 test cases (#23)

### Fixed

### Changed
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
