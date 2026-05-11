"""
Shared pytest fixtures for duckdb-zarr integration tests.

The --extension flag is required for any test that loads the extension:

    pytest test/ --extension build/debug/duckdb_zarr.duckdb_extension
"""
import pathlib
import socket
import subprocess
import sys
import time

import pytest

ROOT = pathlib.Path(__file__).parent.parent


def pytest_addoption(parser):
    parser.addoption(
        "--extension",
        required=False,
        default=None,
        help="Path to duckdb_zarr.duckdb_extension binary",
    )


@pytest.fixture(scope="session")
def extension_path(pytestconfig) -> pathlib.Path:
    raw = pytestconfig.getoption("--extension")
    if raw is None:
        pytest.skip("--extension not provided")
    p = pathlib.Path(raw).resolve()
    if not p.exists():
        pytest.fail(f"Extension binary not found: {p}")
    return p


@pytest.fixture(scope="session")
def http_server_url() -> str:
    """Start python3 -m http.server rooted at the repo root; yield the base URL."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]

    proc = subprocess.Popen(
        [
            sys.executable, "-m", "http.server", str(port),
            "--directory", str(ROOT),
            "--bind", "127.0.0.1",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

    deadline = time.monotonic() + 5.0
    while time.monotonic() < deadline:
        try:
            socket.create_connection(("127.0.0.1", port), timeout=0.2).close()
            break
        except OSError:
            time.sleep(0.05)
    else:
        proc.kill()
        proc.wait()
        pytest.fail("HTTP server did not become ready within 5 seconds")

    yield f"http://127.0.0.1:{port}"

    proc.terminate()
    proc.wait()
