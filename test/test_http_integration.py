"""
HTTP integration tests for duckdb-zarr.

Validates that read_zarr, read_zarr_metadata, and the replacement scan work
correctly when the store is accessed over HTTP via zarrs_http.

The fixture (float_baseline_http.zarr) must carry consolidated metadata in its
root zarr.json — HTTP stores cannot enumerate directories, so zarrs_http reads
the consolidated_metadata block to discover arrays.

Run with:
    pytest test/test_http_integration.py \
        --extension build/debug/duckdb_zarr.duckdb_extension
"""
import pathlib

import duckdb
import pytest

ROOT = pathlib.Path(__file__).parent.parent
FIXTURE = "test/fixtures/xarray_tutorial/float_baseline_http.zarr"


@pytest.fixture(scope="module")
def con(extension_path, http_server_url):
    """DuckDB connection with the zarr extension loaded, shared across the module."""
    c = duckdb.connect(config={"allow_unsigned_extensions": True})
    c.execute(f"LOAD '{extension_path}'")
    return c


@pytest.fixture(scope="module")
def store(http_server_url):
    """Full HTTP URL for the consolidated-metadata fixture."""
    fixture = ROOT / FIXTURE
    if not fixture.exists():
        pytest.fail(
            f"HTTP fixture not found: {fixture}\n"
            "Run: python scripts/generate_fixtures.py"
        )
    return f"{http_server_url}/{FIXTURE}"


# ── read_zarr ──────────────────────────────────────────────────────────────────

def test_row_count(con, store):
    got = con.execute(f"SELECT COUNT(*) FROM read_zarr('{store}')").fetchone()[0]
    assert got == 576


def test_distinct_lat(con, store):
    got = con.execute(
        f"SELECT COUNT(DISTINCT lat) FROM read_zarr('{store}')"
    ).fetchone()[0]
    assert got == 6


def test_distinct_lon(con, store):
    got = con.execute(
        f"SELECT COUNT(DISTINCT lon) FROM read_zarr('{store}')"
    ).fetchone()[0]
    assert got == 12


def test_lat_range(con, store):
    row = con.execute(
        f"SELECT MIN(lat), MAX(lat) FROM read_zarr('{store}')"
    ).fetchone()
    assert row == (-90.0, 90.0)


def test_no_null_temperature(con, store):
    got = con.execute(
        f"SELECT COUNT(*) FROM read_zarr('{store}') WHERE temperature IS NOT NULL"
    ).fetchone()[0]
    assert got == 576


def test_projection_pushdown(con, store):
    """SELECT only lat — projected columns only should still return correct values."""
    got = con.execute(
        f"SELECT COUNT(DISTINCT lat) FROM (SELECT lat FROM read_zarr('{store}'))"
    ).fetchone()[0]
    assert got == 6


# ── replacement scan ───────────────────────────────────────────────────────────

def test_replacement_scan_row_count(con, store):
    got = con.execute(f"SELECT COUNT(*) FROM '{store}'").fetchone()[0]
    assert got == 576


def test_replacement_scan_values(con, store):
    got = con.execute(
        f"SELECT MIN(lat), MAX(lat) FROM '{store}'"
    ).fetchone()
    assert got == (-90.0, 90.0)


# ── read_zarr_metadata ─────────────────────────────────────────────────────────

def test_metadata_array_count(con, store):
    got = con.execute(
        f"SELECT COUNT(*) FROM read_zarr_metadata('{store}')"
    ).fetchone()[0]
    assert got == 4


def test_metadata_roles(con, store):
    rows = con.execute(
        f"SELECT name, role FROM read_zarr_metadata('{store}') ORDER BY name"
    ).fetchall()
    assert rows == [
        ("lat", "coord"),
        ("lon", "coord"),
        ("temperature", "data"),
        ("time", "coord"),
    ]
