#!/bin/bash
set -e

# Create generated directory if it doesn't exist
mkdir -p generated

# Generate Python code from proto file
python3 -m grpc_tools.protoc \
    -I../../src/network/proto \
    --python_out=generated \
    --grpc_python_out=generated \
    ../../src/network/proto/daq.proto

echo "✅ Proto files generated successfully in generated/"
