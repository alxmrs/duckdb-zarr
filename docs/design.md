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
- WebAssembly. The current scaffold ships a `wasm_lib.rs` target; it is not a supported build because `zarrs` and `ndarray` use threading and I/O patterns that don't trivially compile to `wasm32`. 

## Implementability risks (open spike)

The pinned `duckdb` crate (`=1.10502.0`) exposes the VTab (table-function) and scalar-function APIs we lean on in v0.1, but it is not yet established that it exposes (a) replacement-scan registration, (b) storage-extension `ATTACH` hooks, (c) custom dictionary-vector construction, or (d) extension-config-variable registration. If any are missing from the Rust binding, the affected feature falls back to a coarser API, ships behind raw FFI into the DuckDB C API, or moves to "Later."

A short spike maps each design entry point to the concrete `duckdb-rs` symbol it requires. The spike gates the work that depends on those APIs (v0.2 onward); the v0.1 MVP can move forward in parallel — the table-function bind/init/scan path and the schema/reader code only need APIs `duckdb-rs` definitely supports.

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

Coordinate columns within a single chunk are highly repetitive. *If* `duckdb-rs` exposes dictionary-vector construction (open question — see Implementability risks), we emit them as DuckDB dictionary vectors and capture the same memory-saving idea zarr-datafusion realizes through Arrow `DictionaryArray`. If it does not, coord columns fall back to flat vectors filled by indexing into the cached coord array; correctness is identical, the cost is more bytes per scan.

### One table per dimension group

A Zarr store often holds variables with *different* dimension sets. ERA5 is the canonical example: surface variables (`t2m`, `sp`, `tcwv`) have dims `(time, lat, lon)`, while pressure-level variables (`t`, `u`, `v`, `q`) have dims `(time, level, lat, lon)`. A single wide table can't hold both without padding NULLs across millions of rows.

We split the store into **one table per distinct dimension set**. The ERA5 example surfaces as two tables:

- a surface table over `(time, lat, lon)` with one column per surface variable
- an atmosphere table over `(time, level, lat, lon)` with one column per pressure-level variable

Tables get a default name derived from the sorted dim names (`t_lat_lon`, `level_t_lat_lon`); users can override via the `ATTACH` syntax below. `read_zarr_metadata` enumerates them so users can discover groupings before issuing the scan.

## SQL surface

Four entry points, in priority order.

### 1. Replacement scan (the headline UX)

```sql
SELECT lat, lon, AVG(temperature)
FROM 'gs://my-bucket/era5.zarr'
GROUP BY lat, lon;
```

DuckDB's [replacement scan API](https://duckdb.org/docs/api/c/replacement_scans) claims paths via a two-step probe:

1. **Normalized suffix check** — strip a trailing slash, lowercase the suffix, accept `.zarr`. Handles macOS case-insensitive filesystems and the `s3://bucket/store.zarr/` form that cloud consoles love to produce. URL-decoding is DuckDB's job; we accept whatever string we're handed.
2. **Metadata stat** — single read of `zarr.json` (v3) or `.zgroup` (v2) at the path. Only if this succeeds do we claim the path; otherwise we let DuckDB's normal replacement chain take over.

Suffix-less Zarr groups are reachable via `read_zarr(path)` directly; auto-claim of suffix-less paths is deferred because it would force a stat on every string literal DuckDB hands us.

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

### 4. `read_zarr_metadata` and `read_zarr_groups`

Two introspection table functions, each with a homogeneous schema (split because mixing array rows and group rows in one table forces NULLs across most columns of the group rows):

```sql
SELECT * FROM read_zarr_metadata('store.zarr');
-- name | kind (coord|data) | dims | dtype | shape | chunks | compressor | attrs

SELECT * FROM read_zarr_groups('store.zarr');
-- group_name | dims | n_variables | variables | n_rows
```

Both are read-no-chunks. `read_zarr_metadata` enumerates arrays for inspection and tooling; `read_zarr_groups` shows what `ATTACH` would mount and is what the multi-group error message points users at.

## Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                        DuckDB query engine                       │
│   replacement scan ─▶ TableFunction(bind/init/scan)              │
└────────────────────────┬─────────────────────────────────────────┘
                         │
              ┌──────────▼──────────┐
              │  duckdb-zarr crate  │  this repository
              │  ─────────────────  │
              │  extension entry    │
              │  read_zarr          │
              │  read_zarr_metadata │
              │  read_zarr_groups   │
              │  ATTACH (TYPE ZARR) │
              │  replacement scan   │
              └──────────┬──────────┘
                         │  no DuckDB types cross this seam
              ┌──────────▼──────────┐
              │   zarr_reader mod   │  pure Rust; zarrs + ndarray
              │  open / metadata /  │
              │  decode / storage   │
              └─────────────────────┘
```

We follow DuckDB extension conventions: a flat top-level layout with one Rust module per registered SQL entry point and a thin extension entrypoint that wires them in (against `duckdb` pinned to `=1.10502.0` in Cargo.toml).

The `zarr_reader` module is the natural seam — it depends on [`zarrs`](https://lib.rs/crates/zarrs) and `ndarray` and exposes a small interface (open store, read schema, decode chunk into `ndarray::ArrayD`, storage adapter trait) that has no DuckDB types in its public API. Keeping that seam clean costs us nothing today and means the reader could be lifted into a shared crate later if it becomes useful to other Rust projects (e.g. zarr-datafusion). We're not designing for that reuse up front — DataFusion is async-first and DuckDB is sync-with-morsels, and trying to factor across that boundary on day one is more design overhead than it's worth.

### Bind phase

1. Open the store with `zarrs` and read group metadata.
2. Classify arrays as coordinates vs data variables. **Honor xarray's metadata first** — `_ARRAY_DIMENSIONS` (Zarr v2) and `dimension_names` (Zarr v3) — so we get the same coord/data split any xarray-produced store already encodes. Fall back to the "1D = coord, nD = data" heuristic only when that metadata is missing.
3. Read the `coordinates` attribute on each data variable. xarray uses this to mark *non-dimension* coordinates — most commonly a 2D `lat(y, x) / lon(y, x)` mesh on satellite swath data, where the coordinate variable is itself nD. v1 cannot represent these as scalar columns; if encountered, error at bind with a clear message ("non-dimension coordinate `lat` has shape (1024, 1024); 2D coords are deferred"). 
4. Group data variables by their dimension set. Each distinct dim set becomes a candidate output table. **Within a dim group, all selected variables must share the same chunk shape** — if `temperature` is chunked `[24, 10, 10]` and `humidity` is chunked `[1, 20, 20]`, the chunk plan in init can't enumerate one cartesian product that aligns both. We error at bind with a message that names the offending pair and tells the user to query them in separate `read_zarr` calls. (See decision 6.)
5. Materialize coordinate arrays eagerly (they are 1D and small — ERA5 lat/lon/time fits in a few MB) so we have them for both schema metadata and later filter pruning. Coordinates are cached at the **store** level, keyed by array name. An `ATTACH` that mounts multiple views shares one cache, so `time` is loaded exactly once even if it appears in three groups.
6. The `read_zarr` call resolves to exactly one dim group (via `dims :=`, the variables list, or — if unambiguous — the only group present); `ATTACH` materializes all of them as views, each with its own `BindData` but a shared coord cache.
7. Build a DuckDB schema for the chosen group:
   - one column per coordinate in that group, typed from its Zarr dtype
   - one column per selected data variable, typed from its Zarr dtype
8. Stash a `BindData` containing: store handle, chosen dim group, common chunk shape, projected columns, captured DuckDB `FileSystem` handle (see §Storage backends), and a reference to the store-level coord cache.

### Init phase (chunk plan)

Compute the cartesian product of *chunk indices* across the dim group's common chunk shape (validated in bind). Each chunk-index tuple becomes a parallel scan unit. If the group's chunk shape is `[t=24, lat=10, lon=10]`, init enumerates `⌈T/24⌉ × ⌈L/10⌉ × ⌈M/10⌉` units.

Filter pushdown (see below) prunes this list before it ever runs.

### Scan phase (per chunk)

For each work unit:

1. Decode each selected data variable's chunk from `zarrs` into an `ndarray::ArrayD`. Sparse stores have **implicit chunks** — chunk keys with no backing storage object simply don't exist; `zarrs` returns a "chunk not found" error variant for these. We catch it and synthesize a fill-value chunk in place (every cell becomes `_FillValue` and therefore SQL `NULL`). Treating missing chunks as errors would break sparse satellite-swath data and is wrong by Zarr spec.
2. Compute the row-major iteration order over the chunk's logical extent.
3. Fill DuckDB's output `DataChunk` by:
   - emitting coordinate columns as dictionary vectors sliced from the cached coord arrays (or flat vectors if dictionary construction isn't exposed by `duckdb-rs` — see Implementability risks)
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
| CF-encoded int time  | `BIGINT`                     | exposed raw; CF decoding deferred (see Later)  |
| `S<n>` (fixed bytes) | `BLOB`                       |                                                |
| `U<n>` (UTF-32)      | `VARCHAR`                    | decoded                                        |
| structured / object  | unsupported v1               | error at bind                                  |

`_FillValue` from `.zattrs`/`zarr.json` is honored: matching cells become SQL `NULL`. CF time conventions (encoded `int64` + `units`/`calendar` attributes) are exposed raw; decoding is on the deferred list (see Phased plan / Later and decision 3 below). Stores that use native NumPy datetime dtypes pass through unchanged.

**Endianness:** Zarr dtypes carry an explicit byte-order marker (`<f4` little-endian, `>f4` big-endian, `=f4` native). `zarrs` decodes both orientations into native-byte-order `ndarray` buffers, so the copy into DuckDB's `DataChunk` is a `memcpy` with no swap. Round-trip correctness is verified against a big-endian fixture — most modern hardware is little-endian, so the bug would otherwise only surface on legacy machines.

## Storage backends

DuckDB already speaks S3, GCS, Azure, and HTTP through its `httpfs` extension and the secrets manager. Rather than implement our own object-store layer, the storage adapter is a thin shim that:

- detects the URI scheme,
- for `file://` and bare paths, uses `zarrs`'s local store directly,
- for remote schemes, delegates per-key reads to a DuckDB-backed `ReadableStorageTraits` impl that calls into DuckDB's `FileSystem` API.

`zarrs` is built around an abstract `ReadableStorageTraits` trait, so swapping in a DuckDB-backed store is a few hundred lines, not a fork.

**Capturing the FileSystem handle.** DuckDB's `FileSystem` is owned by the `ClientContext` of the active connection, not by any global. We capture a handle to it during the bind callback (the `ClientContext` is one of the bind arguments DuckDB hands us), wrap it in an `Arc`, and stash it inside `BindData` next to the coord cache. The init and scan callbacks pull the handle out of `BindData` and clone the `Arc` into each thread's local state. The Rust storage adapter then forwards each `get(key)` and `get_partial(key, range)` call to the captured `FileSystem` over FFI — credentials, retries, and filesystem-level caching all stay inside DuckDB and don't get re-implemented here. The exact `duckdb-rs` symbols for grabbing the `FileSystem` from the `ClientContext` are part of the spike mentioned above.

## Predicate & projection pushdown

DuckDB's `TableFunction` exposes `pushdown_filters` and `pushdown_projection`. We exploit both:

- **Projection pushdown** — only the data variables referenced by the query (or computed-on after pushdown of `SELECT` columns) are decoded. This is the same easy win zarr-datafusion already takes.
- **Coordinate-range filter pushdown** — for any conjunctive filter on a coordinate column (`time >= '2024-01-01' AND lat BETWEEN 30 AND 60`), we use the cached 1D coord arrays to translate the filter into an index range per dimension, then compute which chunk-index tuples intersect that range. Non-intersecting chunks are dropped from the work-unit list before init returns.
- **Statistics** — we expose per-coordinate min/max as table statistics so DuckDB's optimizer can also reason about ordering and joins.

Filters on data variables cannot be pushed (Zarr is dense, no chunk-level statistics by default); they fall back to DuckDB's normal filter execution after the scan.

**Correctness rigor — required test cases.** Filter pushdown is easy to get wrong on the boundary, and the failure mode is silently-wrong results. The implementation must pass:

- **Decreasing coordinate arrays** — ERA5 latitude runs 90.0 → -90.0; the index translation can't assume monotonic-increasing.
- **Non-uniform spacing** — gaussian grids and pressure levels are not evenly spaced; chunk index can't be `coord_value / chunk_size`. The translation goes through the cached coord array (binary-searchable since coords are sorted-by-spec), not arithmetic.
- **Exact chunk-boundary predicates** — `time = '2024-01-15T00:00:00'` where the value sits exactly on a chunk seam must include the owning chunk once, not zero or two.
- **Empty-result predicates** — `lat > 100` returns zero rows without panic and without scheduling any chunk reads.
- **Inclusive vs exclusive bounds** — `BETWEEN`, `<`, `<=`, `>`, `>=` each must intersect the chunk grid correctly.

## Concurrency & memory

- One decoded chunk per active scan thread. With ERA5-style 24×10×10 chunks at `f4` that's a few hundred KB resident per thread.
- Decode is CPU-heavy; we let DuckDB schedule across cores.
- Coordinate arrays are loaded once in bind and shared (Arc) across threads.
- An optional small LRU of decoded chunks helps when many queries hit the same data. Configuration is exposed as a DuckDB extension config variable (`SET zarr_chunk_cache_mb = 256`) rather than a custom `PRAGMA` — `SET` is the documented mechanism for loadable extensions to register tunables and is the API actually exposed by `duckdb-rs`.

## Phased plan

0. **Spike (v0.0)** — verify what `duckdb-rs` exposes (or doesn't) for replacement-scan registration, storage-extension `ATTACH` hooks, dictionary-vector construction, and extension-config-variable registration. Outcome: a short note appended to this design doc and any bootstrap adjustments. The spike gates the work that depends on those APIs (v0.2 onward); v0.1 MVP can begin in parallel since it only needs the table-function and scalar APIs `duckdb-rs` definitely supports.
1. **MVP (v0.1)** — local-filesystem only, Zarr v3, `read_zarr` + `read_zarr_metadata` + `read_zarr_groups`, single dim group only, no pushdown beyond projection. Goal: end-to-end demo with the synthetic dataset zarr-datafusion ships with.
2. **v0.2** — Zarr v2 + Blosc/LZ4 codecs (free with `zarrs`), replacement scan, dictionary coord columns *(if exposed by `duckdb-rs`)*, type mapping for native datetime/string dtypes.
3. **v0.3** — Multi-group stores via `ATTACH ... (TYPE ZARR)`; coordinate-range filter pushdown; parallel scan; statistics. This is where we should beat naive `xarray + pandas` (with dask, on the same thread budget) on a real ERA5 query. Benchmark must capture chunks-decoded vs total chunks, not just wall-clock.
4. **v0.4** — Remote stores via DuckDB filesystem FFI, secrets integration, community-extension submission.
5. **Later** — CF time UDFs (deferred until a permissively-licensed implementation path exists; nice-to-have, not blocking), chunk-level statistics (when present), aggregate pushdown, write support, 2D non-dimension coordinates, async `zarrs` if remote latency demands it.

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

CF-encoded time (e.g. `int64` + `units = "hours since 1970-01-01"` + `calendar = "noleap"`) needs explicit conversion. We can decode at scan time (transparent but opinionated), expose a UDF (explicit but a tiny extra hop), or defer the whole thing.

> **Decision:** Defer. CF-encoded time columns are exposed raw as `BIGINT`; users handle the conversion app-side (xarray, pandas, or a follow-up SQL macro) until a permissively-licensed implementation path is identified.
>
> **Rationale:** Be honest about what's central. The product win — SQL on a Zarr store, with chunk pruning and parallel scan — does not depend on CF time. The originally-proposed `cftime-rs` is AGPL-3.0 (incompatible with community-extension binary distribution) and unmaintained since October 2023, so the easy path is closed. Implementing CF math in-tree is doable but is several hundred lines of calendar code that would gate every other v0.2 deliverable on its testing burden. Keeping CF support on the deferred list lets the v0.2 milestone land cleanly and lets us pick this up properly when the right dependency exists, without rushing the call.

### 4. Replacement scan ambiguity

`.zarr` is a directory, not a file. DuckDB's replacement scan fires on any string literal in `FROM`, so we need to claim only the strings that actually point at a Zarr store without stat'ing every random path.

> **Decision:** Two-step probe — (a) **normalized** suffix check (case-insensitive `.zarr`, trailing slash stripped to handle `s3://bucket/store.zarr/` cloud-console paths), then (b) a single stat for `zarr.json` (v3) or `.zgroup` (v2) at the path. Only if both pass do we claim the path; otherwise fall through to DuckDB's normal scan chain. Suffix-less Zarr groups are reachable via `read_zarr(path)` directly; auto-claim of suffix-less paths is deferred to avoid stat'ing every string literal.
>
> **Rationale:** A single stat call against a known filename is essentially free, and the suffix gate keeps us from probing every Parquet path the user types. The `FROM 'foo.zarr'` UX is the headline pitch of this extension; spending one stat per query to make it seamless is the right trade. The normalization is enumerated explicitly because the original wording missed the trailing-slash and case-insensitive cases.

### 5. `zarrs` async story

`zarrs` has an async API behind a feature flag; the DuckDB scan callback is sync.

> **Decision:** Sync. One chunk decoded per scan call; intra-query parallelism comes from DuckDB's morsel scheduler dispatching across threads.
>
> **Rationale:** DuckDB's table-function callbacks are sync; bridging async would force a `tokio` runtime per scan or `block_on`, both of which add complexity for unclear gain. zarr-datafusion ships sync today, and it's still well ahead of any non-DataFusion alternative. We can revisit if remote-store I/O latency starts dominating wall-clock time on real workloads.

### 6. Mismatched chunk shapes within a dim group

Within a dim group, two data variables might be chunked differently — e.g. `temperature[24,10,10]` and `humidity[1,20,20]`, both over `(time, lat, lon)`. The init phase enumerates a single cartesian product of chunk indices and so cannot align both grids without finer-grained iteration. Three options: (a) require uniform chunk shape per dim group, error at bind on mismatch; (b) plan against the coarsest chunk grid and re-read finer-chunked variables multiple times per work unit; (c) iterate at the row level rather than the chunk level.

> **Decision:** (a) — require uniform chunk shape across all selected variables in a dim group. The bind phase checks this and fails with a message naming the offending pair plus the workaround (`read_zarr('store.zarr', variables := ['humidity'])`).
>
> **Rationale:** xarray writes uniform chunks across data variables sharing dims by default, so this restriction will rarely be observed in practice. Option (b) doubles implementation complexity for a perf benefit that mostly hits stores nobody actually has. Option (c) defeats the parallel-scan model entirely. Forcing uniform chunks now keeps v1 small and gives us an unambiguous escape hatch via `variables :=` when a real user does hit the case.

## Why this is worth building

Every team running a Pangeo-style pipeline already has DuckDB installed for the tabular side of their workload. Today they shuttle data through Parquet exports or notebook glue to bridge the two worlds. A native extension collapses that bridge: ad-hoc SQL on a Zarr store with no copy, no external service, and the same DuckDB session that already holds their joins, dashboards, and BI tooling.

It's also the smallest piece of the xarray-sql / zarr-datafusion family, because DuckDB does the optimizer, scheduler, and SQL frontend for us. Our job is just the scan.
