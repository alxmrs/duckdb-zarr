# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

### Added

### Fixed

### Changed
- Python+xarray test fixture generator (synthetic v2/v3, CF-time, ERA5-mini) (#23)
- Verify zarrs Cargo feature flags: ndarray+blosc are defaults, not opt-ins (#27)
- Bootstrap: rename crate to duckdb_zarr, drop rusty_quack stub, add zarrs + ndarray deps (#2)
- Process round-2 adversarial review (#41-45 + comments) against tutorial fixtures (#47)
- Write design doc for native DuckDB Zarr integration (xarray-sql parity) (#1)
- Spike: verify duckdb Rust crate replacement scan + ATTACH storage extension APIs (#24)
- Dictionary-vector emission for coordinate columns (#10)
