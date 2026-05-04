# Design: native Zarr in DuckDB

A design for `duckdb-zarr` — a Rust DuckDB extension that lets users query Zarr stores with SQL, in the same spirit as [xarray-sql](https://github.com/alxmrs/xarray-sql) (Python/DataFusion) and [zarr-datafusion](https://lib.rs/crates/zarr-datafusion) (Rust/DataFusion), but as a first-class DuckDB extension with no external query engine in the loop.

## Goals

- `SELECT * FROM 'path/to/store.zarr'` works for any Zarr v2 or v3 store that follows xarray-style conventions.
- Read remote stores (S3, GCS, HTTP) via DuckDB's existing `httpfs`/`secrets` machinery.
- Push projection and coordinate-range filters down so that only the chunks we need are touched.
- Use DuckDB's native parallel scan (one Zarr chunk ≈ one morsel of work).
- Ship as a community extension installable with `INSTALL zarr FROM community`.

## Non-goals (for the first cut)

- Writes. Read-only.
- Nested group hierarchies. We assume a flat Zarr group at the store root, with the xarray convention of sibling 1D coordinate arrays + nD data arrays. Multiple SQL tables can come out of that flat group (one per dim set, see below), but we don't recurse into subgroups.
- Replacing xarray. Users who need lazy array operations should keep using xarray; we just want a SQL handle on the same data.
- Custom codecs beyond what `zarrs` already supports.

## The pivot, in DuckDB terms

A Zarr store with coordinates `lat(L)`, `lon(M)`, `time(T)` and data variables `temperature[T,L,M]`, `humidity[T,L,M]` is exposed as a single DuckDB table:

```
| time      | lat   | lon   | temperature | humidity |
|-----------|-------|-------|-------------|----------|
| 2024-01-01| 0.0   | 0.0   | 273.15      | 0.81     |
| 2024-01-01| 0.0   | 0.5   | 273.42      | 0.79     |
| ...       |       |       |             |          |
```

Logical row count is `T × L × M`. The nD → 2D mapping is the same `ravel()`/metadata reshape that xarray-sql relies on: we never materialize the cartesian product in memory; we generate it row-major on the fly inside each chunk-sized scan.

Coordinate columns within a single chunk are highly repetitive. We emit them as DuckDB dictionary vectors when the column type permits, mirroring the ~75% memory savings zarr-datafusion sees with Arrow `DictionaryArray`.

### One table per dimension group

A Zarr store often holds variables with *different* dimension sets. ERA5 is the canonical example: surface variables (`t2m`, `sp`, `tcwv`) have dims `(time, lat, lon)`, while pressure-level variables (`t`, `u`, `v`, `q`) have dims `(time, level, lat, lon)`. A single wide table can't hold both without padding NULLs across millions of rows.

We split the store into **one table per distinct dimension set**. The ERA5 example surfaces as two tables:

- a surface table over `(time, lat, lon)` with one column per surface variable
- an atmosphere table over `(time, level, lat, lon)` with one column per pressure-level variable

Tables get a default name derived from the sorted dim names (`t_lat_lon`, `level_t_lat_lon`); users can override via the `ATTACH` syntax below. `read_zarr_metadata` enumerates them so users can discover groupings before issuing the scan.

## SQL surface

Five entry points, in priority order.

### 1. Replacement scan (the headline UX)

```sql
SELECT lat, lon, AVG(temperature)
FROM 'gs://my-bucket/era5.zarr'
GROUP BY lat, lon;
```

DuckDB's [replacement scan API](https://duckdb.org/docs/api/c/replacement_scans) claims paths ending in `.zarr` once they pass a cheap probe for `zarr.json` (v3) or `.zgroup` (v2). The scan rewrites to `read_zarr(path)` under the hood.

If the store contains exactly one dimension group, the replacement scan returns that table directly. If it contains multiple, the scan errors with a message listing the available groups and tells the user to `ATTACH` or call `read_zarr` with an explicit `dims :=`.

### 2. `read_zarr` table function

```sql
SELECT * FROM read_zarr('path/to/store.zarr');

-- Pick a subset of variables
SELECT * FROM read_zarr('store.zarr', variables := ['temperature']);

-- Pick a dimension group when the store has more than one
SELECT * FROM read_zarr('era5.zarr', dims := ['time', 'lat', 'lon']);

-- Override coordinate-to-variable mapping when conventions don't apply
SELECT * FROM read_zarr(
  'store.zarr',
  coords := ['t', 'y', 'x'],
  variables := ['u', 'v']
);
```

Named arguments stay close to xarray's vocabulary (`variables`, `coords`, `chunks`, `dims`). Variables that share the requested dim set become one output column each, joined on coordinate index — exactly the xarray-sql `pivot()` shape. Variables outside that dim set are silently excluded; the user picks them up by querying a different `dims` group.

### 3. `ATTACH` for multi-group stores

```sql
ATTACH 'era5.zarr' AS era5 (TYPE ZARR);

SELECT * FROM era5.surface          -- (time, lat, lon)
WHERE time >= '2024-01-01';

SELECT level, AVG(t)
FROM era5.atmosphere                -- (time, level, lat, lon)
GROUP BY level;
```

`ATTACH ... (TYPE ZARR)` mounts the store as a DuckDB schema with one view per dimension group. Group names default to a slugified join of the dim names; users can rename with `ALTER VIEW`. This is the recommended UX for ERA5-class stores where you'll be issuing many queries and want stable table names.

### 4. `read_zarr_metadata` table function

```sql
SELECT * FROM read_zarr_metadata('store.zarr');
-- name | kind (coord|data|group) | dims | dtype | shape | chunks | compressor | attrs
```

A cheap, read-no-chunks introspection function for tooling and notebooks. Returns one row per Zarr array plus one row per inferred dimension group (so users can see what `ATTACH` would create before they run it).

### 5. CF time/calendar UDFs

CF-encoded time coordinates (e.g. `int64` with `units = "hours since 1970-01-01"`) stay raw in the table. We ship scalar UDFs to convert them on demand:

```sql
SELECT cf_to_timestamp(time, 'hours since 1970-01-01', 'gregorian') AS ts,
       lat, lon, temperature
FROM read_zarr('era5.zarr')
WHERE cf_to_timestamp(time, 'hours since 1970-01-01') BETWEEN '2024-01-01' AND '2024-02-01';
```

Inverse (`timestamp_to_cf`) and a convenience overload that pulls `units`/`calendar` from the array attributes (`cf_to_timestamp(time, '<store>.zarr', 'time')`) round out the surface. Implementation wraps [`cftime-rs`](https://github.com/antscloud/cftime-rs) so non-Gregorian calendars (`noleap`, `360_day`, etc.) work correctly. This mirrors xarray-sql's pattern: explicit conversion at query time, no hidden auto-decoding at scan time.

## Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                        DuckDB query engine                       │
│   replacement scan ─▶ TableFunction(bind/init/scan)              │
└────────────────────────┬─────────────────────────────────────────┘
                         │
              ┌──────────▼──────────┐
              │   ZarrTableFunction │  Rust, in this crate
              │  - bind: schema     │
              │  - init: chunk plan │
              │  - scan: fill chunk │
              └──────────┬──────────┘
                         │
        ┌────────────────┼─────────────────┐
        ▼                ▼                 ▼
   ┌──────────┐    ┌──────────────┐   ┌──────────────────┐
   │  zarrs   │    │ChunkPlanner  │   │  Storage adapter │
   │ (decode) │    │(pruning,     │   │  local / s3 /    │
   │          │    │ scheduling)  │   │  gs / http       │
   └──────────┘    └──────────────┘   └──────────────────┘
```

Three Rust modules, mirroring the zarr-datafusion split but rewired for the DuckDB C API via the `duckdb` crate (already pinned to `=1.10502.0` in Cargo.toml):

- `reader/` — wraps [`zarrs`](https://lib.rs/crates/zarrs) for v2/v3 metadata + chunk decode and converts `ndarray::ArrayD` views into DuckDB `DataChunk` columns.
- `planner/` — turns a query against a Zarr store into a list of (chunk, row-range) work units after projection and filter pushdown.
- `table_function/` — implements DuckDB's bind / init / local-init / scan callbacks, plus the replacement scan registration.

### Bind phase

1. Open the store with `zarrs` and read group metadata.
2. Classify arrays as coordinates vs data variables. **Honor xarray's metadata first** — `_ARRAY_DIMENSIONS` (Zarr v2) and `dimension_names` (Zarr v3) — so we get the same coord/data split that any xarray-produced store already encodes. Fall back to the "1D = coord, nD = data" heuristic only when that metadata is missing.
3. Group data variables by their dimension set. Each distinct dim set becomes a candidate output table. The `read_zarr` call resolves to exactly one of these (via `dims :=`, the variables list, or — if unambiguous — the only group present); `ATTACH` materializes all of them as views.
4. Materialize coordinate arrays eagerly (they are 1D and small — ERA5 lat/lon/time fits in a few MB) so we have them for both schema metadata and later filter pruning. Cache them in the bind data, keyed by name and shared across dim groups that reference them.
5. Build a DuckDB schema for the chosen group:
   - one column per coordinate in that group, typed from its Zarr dtype
   - one column per selected data variable, typed from its Zarr dtype
6. Stash a `BindData` containing: store handle, chosen dim group, coord arrays, variable list, projected columns.

### Init phase (chunk plan)

Compute the cartesian product of *chunk indices* across the dimensions of the (first) data variable. Each chunk-index tuple becomes a parallel scan unit. With the example above, if `temperature` has chunk shape `[t=24, lat=10, lon=10]`, init enumerates `⌈T/24⌉ × ⌈L/10⌉ × ⌈M/10⌉` units.

Filter pushdown (see below) prunes this list before it ever runs.

### Scan phase (per chunk)

For each work unit:

1. Decode each selected data variable's chunk from `zarrs` into an `ndarray::ArrayD`.
2. Compute the row-major iteration order over the chunk's logical extent.
3. Fill DuckDB's output `DataChunk` by:
   - emitting coordinate columns as dictionary vectors sliced from the cached coord arrays
   - emitting data columns by reading the ndarray buffer in row-major order
4. Yield up to `STANDARD_VECTOR_SIZE` (2048) rows at a time; resume on the next call.

The local scan state holds the current chunk's decoded buffers and a cursor; the global init state holds the immutable work-unit list. This matches DuckDB's morsel-driven model and gives us free intra-query parallelism.

## Type mapping

| Zarr dtype           | DuckDB type                  | Notes                                          |
| -------------------- | ---------------------------- | ---------------------------------------------- |
| `i1/i2/i4/i8`        | `TINYINT`..`BIGINT`          |                                                |
| `u1/u2/u4/u8`        | `UTINYINT`..`UBIGINT`        |                                                |
| `f4/f8`              | `FLOAT`/`DOUBLE`             | NaN preserved                                  |
| `bool`               | `BOOLEAN`                    |                                                |
| `M8[ns]` / `M8[us]`  | `TIMESTAMP_NS` / `TIMESTAMP` | native NumPy datetimes; mapped directly        |
| CF-encoded int time  | `BIGINT`                     | exposed raw; convert with `cf_to_timestamp`    |
| `S<n>` (fixed bytes) | `BLOB`                       |                                                |
| `U<n>` (UTF-32)      | `VARCHAR`                    | decoded                                        |
| structured / object  | unsupported v1               | error at bind                                  |

`_FillValue` from `.zattrs`/`zarr.json` is honored: matching cells become SQL `NULL`. We deliberately do **not** auto-decode CF time conventions at scan time — see decision 3 below. Stores that use native NumPy datetime dtypes pass through unchanged.

## Storage backends

DuckDB already speaks S3, GCS, Azure, and HTTP through its `httpfs` extension and the secrets manager. Rather than implement our own object-store layer, the storage adapter is a thin shim that:

- detects the URI scheme,
- for `file://` and bare paths, uses `zarrs`'s local store directly,
- for remote schemes, delegates per-key reads to DuckDB's filesystem via FFI, so users get unified credential handling.

`zarrs` is built around an abstract `ReadableStorageTraits` trait, so swapping in a DuckDB-backed store is a few hundred lines, not a fork.

## Predicate & projection pushdown

DuckDB's `TableFunction` exposes `pushdown_filters` and `pushdown_projection`. We exploit both:

- **Projection pushdown** — only the data variables referenced by the query (or computed-on after pushdown of `SELECT` columns) are decoded. This is the same easy win zarr-datafusion already takes.
- **Coordinate-range filter pushdown** — for any conjunctive filter on a coordinate column (`time >= '2024-01-01' AND lat BETWEEN 30 AND 60`), we use the cached 1D coord arrays to translate the filter into an index range per dimension, then compute which chunk-index tuples intersect that range. Non-intersecting chunks are dropped from the work-unit list before init returns.
- **Statistics** — we expose per-coordinate min/max as table statistics so DuckDB's optimizer can also reason about ordering and joins.

Filters on data variables cannot be pushed (Zarr is dense, no chunk-level statistics by default); they fall back to DuckDB's normal filter execution after the scan.

## Concurrency & memory

- One decoded chunk per active scan thread. With ERA5-style 24×10×10 chunks at `f4` that's a few hundred KB resident per thread.
- Decode is CPU-heavy; we let DuckDB schedule across cores.
- Coordinate arrays are loaded once in bind and shared (Arc) across threads.
- An optional small LRU of decoded chunks (configurable via `PRAGMA zarr_chunk_cache_mb`) helps when many queries hit the same data.

## Phased plan

1. **MVP (v0.1)** — local-filesystem only, Zarr v3, `read_zarr` + `read_zarr_metadata`, single dim group only, no pushdown beyond projection. Goal: end-to-end demo with the synthetic dataset zarr-datafusion ships with.
2. **v0.2** — Zarr v2 + Blosc/LZ4 codecs (free with `zarrs`), replacement scan, dictionary coord columns, type mapping for native datetime/string dtypes, CF UDFs (`cf_to_timestamp` / `timestamp_to_cf`) wrapping `cftime-rs`.
3. **v0.3** — Multi-group stores via `ATTACH ... (TYPE ZARR)`; coordinate-range filter pushdown; parallel scan; statistics. This is where we should beat naive `xarray + pandas` on a real ERA5 query.
4. **v0.4** — Remote stores via DuckDB filesystem FFI, secrets integration, community-extension submission.
5. **Later** — chunk-level statistics (when present), aggregate pushdown, write support, async `zarrs` if remote latency demands it.

## Open questions and decisions

Each subsection keeps the original tradeoff visible so future readers can see what was on the table, then records the call we made and why.

### 1. Convention vs. config

zarr-datafusion infers the coord/data split from dimensionality alone; xarray reads `_ARRAY_DIMENSIONS` (v2) or `dimension_names` (v3). We can either commit to one or layer them.

> **Decision:** Honor xarray's metadata first; fall back to the "1D = coord, nD = data" heuristic only when the metadata is missing.
>
> **Rationale:** xarray is the de facto producer of stores in this ecosystem, and its metadata already encodes the right answer — using it gets us correct dim names and ordering for free. The dimensionality heuristic exists for ad-hoc Zarr stores written by tools that don't follow the xarray convention.

### 2. Multi-variable layout

When a store contains many data variables, do we expose them as one wide table, one table per variable, or something in between?

> **Decision:** One wide table **per dimension group** — i.e. per distinct set of dimensions across the data variables. ERA5 → two tables: surface (`time, lat, lon`) and atmosphere (`time, level, lat, lon`).
>
> **Rationale:** A single wide table only works when all variables share dims. Real scientific stores routinely mix surface (3D) and atmospheric (4D) fields, and forcing them together would mean pad-NULL-or-bust. One-table-per-variable goes the opposite direction and wastes the natural locality. Grouping by dim set keeps schemas tight, row counts honest, and queries readable. `ATTACH` with one view per group is what makes this UX tolerable for stores with several groups.

### 3. CF-time decoding

CF-encoded time (e.g. `int64` + `units = "hours since 1970-01-01"` + `calendar = "noleap"`) needs explicit conversion. We can decode at scan time (transparent but opinionated) or expose a UDF (explicit but a tiny extra hop).

> **Decision:** Expose a UDF, don't auto-decode. Wrap [`cftime-rs`](https://github.com/antscloud/cftime-rs) and ship `cf_to_timestamp(value, units, calendar)` plus an inverse and an attribute-driven overload.
>
> **Rationale:** Matches xarray-sql's pattern, which the user already maintains. Auto-decoding hides where the conversion happens, locks us into one timestamp type, and gets non-Gregorian calendars wrong unless we guess correctly. A UDF gives users an explicit handle and lets the optimizer reason about the conversion as a normal scalar function. `cftime-rs` is the canonical Rust port of the Python `cftime` library, so behavior parity with xarray comes essentially for free.

### 4. Replacement scan ambiguity

`.zarr` is a directory, not a file. DuckDB's replacement scan fires on any string literal in `FROM`, so we need to claim only the strings that actually point at a Zarr store without stat'ing every random path.

> **Decision:** Two-step probe — (a) cheap suffix check for `.zarr`, then (b) a single stat for `zarr.json` (v3) or `.zgroup` (v2) at the path. Only if both pass do we claim the path; otherwise fall through to DuckDB's normal scan chain.
>
> **Rationale:** A single stat call against a known filename is essentially free, and the suffix gate keeps us from probing every Parquet path the user types. The `FROM 'foo.zarr'` UX is the headline pitch of this extension; spending one stat per query to make it seamless is the right trade.

### 5. `zarrs` async story

`zarrs` has an async API behind a feature flag; the DuckDB scan callback is sync.

> **Decision:** Sync. One chunk decoded per scan call; intra-query parallelism comes from DuckDB's morsel scheduler dispatching across threads.
>
> **Rationale:** DuckDB's table-function callbacks are sync; bridging async would force a `tokio` runtime per scan or `block_on`, both of which add complexity for unclear gain. zarr-datafusion ships sync today, and it's still well ahead of any non-DataFusion alternative. We can revisit if remote-store I/O latency starts dominating wall-clock time on real workloads.

## Why this is worth building

Every team running a Pangeo-style pipeline already has DuckDB installed for the tabular side of their workload. Today they shuttle data through Parquet exports or notebook glue to bridge the two worlds. A native extension collapses that bridge: ad-hoc SQL on a Zarr store with no copy, no external service, and the same DuckDB session that already holds their joins, dashboards, and BI tooling.

It's also the smallest piece of the xarray-sql / zarr-datafusion family, because DuckDB does the optimizer, scheduler, and SQL frontend for us. Our job is just the scan.
