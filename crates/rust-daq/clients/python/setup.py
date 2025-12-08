"""
Setup script for rust-daq-client.

This script handles:
- Auto-compilation of protobuf definitions during installation
- Generating Python gRPC stubs from proto/daq.proto
- Placing generated code in src/rust_daq/generated/
"""

import os
import sys
from pathlib import Path
from setuptools import setup
from setuptools.command.build_py import build_py
from grpc_tools import protoc


class BuildWithProto(build_py):
    """Custom build command that generates protobuf code before building."""

    def run(self):
        """Generate protobuf code, then run standard build."""
        # Paths relative to this file
        base_dir = Path(__file__).parent.absolute()
        proto_dir = base_dir.parent.parent / "proto"
        proto_file = proto_dir / "daq.proto"
        generated_dir = base_dir / "src" / "rust_daq" / "generated"

        # Create generated directory if it doesn't exist
        generated_dir.mkdir(parents=True, exist_ok=True)

        # Create __init__.py in generated directory
        init_file = generated_dir / "__init__.py"
        init_file.write_text(
            '"""Auto-generated protobuf code for rust-daq gRPC API."""\n'
        )

        # Check proto file exists
        if not proto_file.exists():
            print(f"ERROR: Proto file not found at {proto_file}", file=sys.stderr)
            sys.exit(1)

        print(f"Generating protobuf code from {proto_file}")
        print(f"Output directory: {generated_dir}")

        # Run protoc to generate Python code
        # protoc arguments:
        #   --proto_path: where to find proto files (import path)
        #   --python_out: where to output *_pb2.py files
        #   --grpc_python_out: where to output *_pb2_grpc.py files
        result = protoc.main(
            [
                "grpc_tools.protoc",
                f"--proto_path={proto_dir}",
                f"--python_out={generated_dir}",
                f"--grpc_python_out={generated_dir}",
                str(proto_file),
            ]
        )

        if result != 0:
            print(f"ERROR: protoc failed with exit code {result}", file=sys.stderr)
            sys.exit(1)

        print("Protobuf code generation complete")

        # Fix import statements in generated _grpc.py file
        # Generated code uses "import daq_pb2" but we need "from . import daq_pb2"
        grpc_file = generated_dir / "daq_pb2_grpc.py"
        if grpc_file.exists():
            content = grpc_file.read_text()
            content = content.replace("import daq_pb2 as", "from . import daq_pb2 as")
            grpc_file.write_text(content)
            print("Fixed imports in daq_pb2_grpc.py")

        # Continue with standard build
        super().run()


if __name__ == "__main__":
    setup(
        cmdclass={
            "build_py": BuildWithProto,
        }
    )
