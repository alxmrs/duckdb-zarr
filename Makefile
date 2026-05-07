.PHONY: clean clean_all

PROJ_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

EXTENSION_NAME=duckdb_zarr

# Set to 1 to enable Unstable API (binaries will only work on TARGET_DUCKDB_VERSION, forwards compatibility will be broken)
# Note: currently extension-template-rs requires this, as duckdb-rs relies on unstable C API functionality
USE_UNSTABLE_C_API=1

# Target DuckDB version
TARGET_DUCKDB_VERSION=v1.5.2

all: configure debug

# Include makefiles from DuckDB
include extension-ci-tools/makefiles/c_api_extensions/base.Makefile
include extension-ci-tools/makefiles/c_api_extensions/rust.Makefile

configure: venv platform extension_version

debug: build_extension_library_debug build_extension_with_metadata_debug
release: build_extension_library_release build_extension_with_metadata_release

test: test_debug
test_debug: test_extension_debug
test_release: test_extension_release

# HTTP integration tests — require a built extension; skip in offline CI.
test_http_debug: check_configure
	$(PYTHON_VENV_BIN) -m pytest test/test_http_integration.py \
		--extension build/debug/$(EXTENSION_NAME).duckdb_extension -v

test_http_release: check_configure
	$(PYTHON_VENV_BIN) -m pytest test/test_http_integration.py \
		--extension build/release/$(EXTENSION_NAME).duckdb_extension -v

clean: clean_build clean_rust
clean_all: clean_configure clean
