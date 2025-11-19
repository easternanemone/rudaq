#!/bin/bash
set -e

echo "=========================================="
echo "rust-daq Python Client Setup"
echo "=========================================="
echo ""

# Check Python version
echo "1. Checking Python installation..."
if ! command -v python3 &> /dev/null; then
    echo "❌ Error: python3 is not installed"
    exit 1
fi

PYTHON_VERSION=$(python3 --version | cut -d' ' -f2)
echo "   ✅ Found Python $PYTHON_VERSION"

# Create virtual environment
echo ""
echo "2. Creating virtual environment..."
if [ -d "venv" ]; then
    echo "   ⚠️  Virtual environment already exists, skipping..."
else
    python3 -m venv venv
    echo "   ✅ Virtual environment created"
fi

# Activate virtual environment
echo ""
echo "3. Activating virtual environment..."
source venv/bin/activate
echo "   ✅ Virtual environment activated"

# Install dependencies
echo ""
echo "4. Installing dependencies..."
pip install --upgrade pip > /dev/null 2>&1
pip install -r requirements.txt
echo "   ✅ Dependencies installed"

# Generate proto code
echo ""
echo "5. Generating Python code from proto file..."
chmod +x generate_proto.sh
./generate_proto.sh

# Fix generated imports
echo ""
echo "6. Fixing generated imports..."
echo "# Generated protobuf code" > generated/__init__.py
sed -i '' 's/import daq_pb2/from . import daq_pb2/g' generated/daq_pb2_grpc.py
echo "   ✅ Imports fixed"

# Verify installation
echo ""
echo "7. Verifying installation..."
python3 -c "from daq_client import DaqClient; print('   ✅ Client import successful')"

echo ""
echo "=========================================="
echo "Setup Complete!"
echo "=========================================="
echo ""
echo "Next steps:"
echo "  1. Start the rust-daq daemon:"
echo "     cd /path/to/rust-daq"
echo "     cargo run -- daemon --port 50051"
echo ""
echo "  2. Activate the virtual environment:"
echo "     source venv/bin/activate"
echo ""
echo "  3. Run examples:"
echo "     python daq_client.py"
echo "     python examples/basic_usage.py"
echo "     python examples/stream_status.py"
echo ""
