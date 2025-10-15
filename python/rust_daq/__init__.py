"""
Python bindings for the rust-daq high-performance DAQ system.

This package provides a high-level, Pythonic interface to the core Rust
functionality, allowing for easy scripting and integration into Python-based
scientific workflows.
"""
# Import the symbols from the compiled Rust extension module (_rust_daq.so)
# and expose them at the top level of the rust_daq package.
from ._rust_daq import DataPoint, MaiTai, Newport1830C

__all__ = [
    "DataPoint",
    "MaiTai",
    "Newport1830C",
]
