#!/usr/bin/env python3
"""
Generate Zarr v3 test fixtures from xarray tutorial datasets.

Each fixture exercises one or more code paths in the Rust reader.
See docs/design.md §Bind phase and §Type mapping for the requirements.

Usage:
    pip install xarray zarr scipy h5netcdf pooch numpy
    python scripts/generate_fixtures.py

Output: test/fixtures/xarray_tutorial/<name>.zarr
"""
import pathlib
import shutil

import numpy as np
import xarray as xr
import zarr

ROOT = pathlib.Path(__file__).parent.parent
FIXTURES = ROOT / "test" / "fixtures" / "xarray_tutorial"


def write_zarr(ds: xr.Dataset, name: str, encoding: dict | None = None) -> None:
    dest = FIXTURES / f"{name}.zarr"
    if dest.exists():
        shutil.rmtree(dest)
    ds.to_zarr(dest, zarr_format=3, consolidated=False, encoding=encoding or {})
    print(f"  wrote {dest}")


def ensure_attr(ds: xr.Dataset, var: str, key: str, value) -> xr.Dataset:
    """Add key=value to da.attrs; used to restore attrs that xarray moved to encoding."""
    da = ds[var].copy()
    da.attrs[key] = value
    if var in ds.coords:
        return ds.assign_coords({var: da})
    return ds.assign({var: da})


def main() -> None:
    FIXTURES.mkdir(parents=True, exist_ok=True)

    # ── air_temperature ──────────────────────────────────────────────────────
    # Baseline: single float64 variable, 3D (time, lat, lon), CF-encoded time.
    # Tests: basic read_zarr, coord classification, CF time passthrough.
    print("air_temperature...")
    ds = xr.tutorial.open_dataset("air_temperature")
    write_zarr(ds, "air_temperature")

    # ── air_temperature_gradient ─────────────────────────────────────────────
    # Tests: scale_factor/add_offset packed decoding (Tair as int16), plus
    # mismatched-chunk-shape bind error (Tair chunks != dTdx chunks).
    # Tair is re-encoded as int16 with scale_factor=0.01 and a distinct chunk
    # shape so bind sees three variables over the same dims but two chunk grids.
    print("air_temperature_gradient...")
    ds = xr.tutorial.open_dataset("air_temperature_gradient")
    # Tair chunked [1, 25, 53] (one time-step); dTdx/dTdy left at default.
    # The mismatch is between Tair's chunk shape and the gradient fields' shape.
    encoding = {
        "Tair": {
            "dtype": "int16",
            "scale_factor": np.float64(0.01),
            "_FillValue": np.int16(-32767),
            "chunks": [1, 25, 53],
        },
        "dTdx": {"dtype": "float32", "chunks": [10, 25, 53]},
        "dTdy": {"dtype": "float32", "chunks": [10, 25, 53]},
    }
    write_zarr(ds, "air_temperature_gradient", encoding=encoding)

    # ── ersstv5 ──────────────────────────────────────────────────────────────
    # Tests: CF bounds variable suppression (time has bounds='time_bnds';
    # time_bnds has shape (624, 2) and an extra nbnds dim).
    # Also: missing_value sentinel masking on sst.
    print("ersstv5...")
    ds = xr.tutorial.open_dataset("ersstv5", mask_and_scale=False)
    # Restore missing_value and _FillValue as explicit attrs so the Rust reader
    # can find them; xarray may have moved them to encoding during load.
    if "missing_value" not in ds["sst"].attrs and "missing_value" in ds["sst"].encoding:
        ds = ensure_attr(ds, "sst", "missing_value", ds["sst"].encoding["missing_value"])
    if "_FillValue" not in ds["sst"].attrs and "_FillValue" in ds["sst"].encoding:
        ds = ensure_attr(ds, "sst", "_FillValue", ds["sst"].encoding["_FillValue"])
    # Add explicit bounds attr on time (the raw NetCDF omits it, but time_bnds is
    # present with shape (624, 2)). Adding it here exercises the primary CF bounds
    # detection path; the fallback (name-pattern matching) is implicitly also tested.
    ds = ensure_attr(ds, "time", "bounds", "time_bnds")
    write_zarr(ds, "ersstv5")

    # ── basin_mask ───────────────────────────────────────────────────────────
    # Tests: missing_value=-100 (int8) with no _FillValue — the "missing_value
    # only" branch in NULL masking precedence.
    print("basin_mask...")
    ds = xr.tutorial.open_dataset("basin_mask", mask_and_scale=False)
    if "missing_value" not in ds["basin"].attrs and "missing_value" in ds["basin"].encoding:
        ds = ensure_attr(ds, "basin", "missing_value", ds["basin"].encoding["missing_value"])
    write_zarr(ds, "basin_mask")

    # ── rasm ─────────────────────────────────────────────────────────────────
    # Tests: noleap calendar CF-encoded time (raw on-disk), 2D non-dim coords
    # (xc, yc) which should trigger the v1 bind error for non-dimension coords.
    print("rasm...")
    ds = xr.tutorial.open_dataset("rasm")
    write_zarr(ds, "rasm")

    # ── unindexed_dim (synthetic) ────────────────────────────────────────────
    # Tests: dimension with no backing coordinate array (the "tiny" dataset case
    # from the design doc). dim_0 has no coord; the reader synthesizes 0..5.
    print("unindexed_dim (synthetic)...")
    rng = np.random.default_rng(42)
    data = rng.standard_normal((5, 4, 6)).astype("float32")
    # lat and lon are explicit coords; dim_0 is unindexed (no coord array).
    lat = np.linspace(-90.0, 90.0, 4)
    lon = np.linspace(0.0, 360.0, 6, endpoint=False)
    da = xr.DataArray(data, dims=["dim_0", "lat", "lon"],
                      coords={"lat": lat, "lon": lon})
    ds = xr.Dataset({"values": da})
    write_zarr(ds, "unindexed_dim")

    # ── scalar_coord (synthetic) ─────────────────────────────────────────────
    # Tests: scalar (0-dim) coordinate variables (e.g. ROMS hc, Vtransform).
    # These should be excluded from the row schema and surfaced in metadata.
    print("scalar_coord (synthetic)...")
    rng = np.random.default_rng(7)
    data = rng.standard_normal((3, 4)).astype("float32")
    lat = np.array([10.0, 20.0, 30.0, 40.0])
    time = np.array([0, 1, 2])
    da = xr.DataArray(data, dims=["time", "lat"],
                      coords={"time": time, "lat": lat})
    ds = xr.Dataset(
        {"temperature": da},
        coords={
            "hc": xr.DataArray(np.float64(250.0), attrs={"long_name": "critical depth"}),
            "Vtransform": xr.DataArray(np.int32(2), attrs={"long_name": "vertical transform"}),
        },
    )
    write_zarr(ds, "scalar_coord")

    print("All fixtures written.")


if __name__ == "__main__":
    main()
