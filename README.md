# duckdb-zarr

[![Main Extension Distribution Pipeline](https://github.com/alxmrs/duckdb-zarr/actions/workflows/MainDistributionPipeline.yml/badge.svg)](https://github.com/alxmrs/duckdb-zarr/actions/workflows/MainDistributionPipeline.yml)

A Rust DuckDB extension that lets you query [Zarr](https://zarr.dev/) stores with SQL — in the same spirit as [xarray-sql](https://github.com/alxmrs/xarray-sql) and [zarr-datafusion](https://lib.rs/crates/zarr-datafusion), but as a first-class DuckDB extension with no external query engine.

```sql
-- Automatic path interception (v0.2+)
SELECT lat, lon, AVG(temperature)
FROM 'gs://my-bucket/era5.zarr'
GROUP BY lat, lon;

-- Explicit table function (v0.1+)
SELECT * FROM read_zarr('path/to/store.zarr');

-- Multi-group stores (v0.3+)
ATTACH 'era5.zarr' AS era5 (TYPE ZARR);
SELECT * FROM era5.surface WHERE time >= '2024-01-01';
```

See [docs/design.md](docs/design.md) for the full design.

## Status

Pre-implementation. Phase 0 (design + spike + fixtures) is complete. See the [phased plan](docs/design.md#phased-plan) for the roadmap.

## Building

```shell
git clone --recurse-submodules <repo>
make configure
make debug
```

Requires: Rust toolchain, Python 3, make, git.

## Testing

```shell
make test_debug   # or make test_release
```

Tests are in `test/sql/` (SQLLogicTest format). Zarr v3 test fixtures are in `test/fixtures/xarray_tutorial/`; regenerate with:

```shell
pip install xarray zarr scipy h5netcdf pooch numpy
python scripts/generate_fixtures.py
```

## Loading (once built)

```sh
duckdb -unsigned
```

```sql
LOAD './build/debug/extension/duckdb_zarr/duckdb_zarr.duckdb_extension';
```
