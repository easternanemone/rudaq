#!/usr/bin/env python3
"""
Polarization Optical Element Characterization via gRPC
=======================================================

Identifies which rotator is HWP, QWP, or Linear Polarizer by analyzing
power response vs rotation angle.

Theory:
- Linear Polarizer: cos^2(theta) -> 2 peaks per 360 degrees
- Half-Wave Plate: cos^2(2*theta) -> 4 peaks per 360 degrees
- Quarter-Wave Plate: Complex pattern -> 2 peaks, phase shifted

Usage:
    pip install grpcio grpcio-tools
    python scripts/polarization_characterization.py --addr localhost:50051
"""

import argparse
import sys
import time
from typing import List, Tuple
import numpy as np

# gRPC imports
import grpc

# Import generated protobuf classes
# First try relative import, then fall back to path manipulation
try:
    from proto import daq_pb2, daq_pb2_grpc
except ImportError:
    import os
    sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
    # Generate them on the fly if needed
    import subprocess
    proto_dir = os.path.join(os.path.dirname(__file__), '..', 'proto')
    output_dir = os.path.dirname(__file__)
    subprocess.run([
        'python', '-m', 'grpc_tools.protoc',
        f'-I{proto_dir}',
        f'--python_out={output_dir}',
        f'--grpc_python_out={output_dir}',
        f'{proto_dir}/daq.proto'
    ], check=True)
    from scripts import daq_pb2, daq_pb2_grpc


class HardwareClient:
    """Simple gRPC client for hardware control"""

    def __init__(self, addr: str):
        self.channel = grpc.insecure_channel(addr)
        self.stub = daq_pb2_grpc.HardwareServiceStub(self.channel)

    def list_devices(self) -> List[dict]:
        """List all registered devices"""
        response = self.stub.ListDevices(daq_pb2.ListDevicesRequest())
        return [
            {
                'id': d.id,
                'name': d.name,
                'is_movable': d.is_movable,
                'is_readable': d.is_readable,
            }
            for d in response.devices
        ]

    def move_abs(self, device_id: str, position: float) -> bool:
        """Move a device to absolute position"""
        req = daq_pb2.MoveRequest(
            device_id=device_id,
            value=position,
            wait_for_completion=True
        )
        response = self.stub.Move(req)
        return response.success

    def read_value(self, device_id: str) -> float:
        """Read a value from a device"""
        req = daq_pb2.ReadValueRequest(device_id=device_id)
        response = self.stub.ReadValue(req)
        if response.success:
            return response.value
        else:
            raise RuntimeError(f"Read failed: {response.error_message}")

    def wait_settled(self, device_id: str, tolerance: float = 0.1, timeout_ms: int = 5000) -> float:
        """Wait for device to settle and return final position"""
        req = daq_pb2.WaitSettledRequest(
            device_id=device_id,
            tolerance=tolerance,
            timeout_ms=timeout_ms
        )
        response = self.stub.WaitSettled(req)
        if response.success:
            return response.position
        else:
            return response.position  # Return position even if not settled


def scan_rotator(client: HardwareClient, rotator_id: str, power_meter_id: str,
                 angles: List[float], settle_time: float = 0.5,
                 num_samples: int = 3) -> List[Tuple[float, float]]:
    """
    Scan a rotator through angles and measure power at each position.

    Returns list of (angle, power) tuples.
    """
    results = []

    for angle in angles:
        # Move to position
        success = client.move_abs(rotator_id, angle)
        if not success:
            print(f"  WARNING: Move to {angle} deg failed")
            continue

        # Wait for settling
        time.sleep(settle_time)

        # Take multiple readings and average
        readings = []
        for _ in range(num_samples):
            try:
                value = client.read_value(power_meter_id)
                readings.append(value)
            except RuntimeError as e:
                print(f"  WARNING: Read failed at {angle} deg: {e}")
            time.sleep(0.05)

        if readings:
            avg_power = sum(readings) / len(readings)
            results.append((angle, avg_power))
            print(f"  {rotator_id}: {angle:6.1f} deg -> {avg_power:.6e} W")

    return results


def analyze_scan(results: List[Tuple[float, float]], name: str) -> dict:
    """
    Analyze scan results to identify optical element type.

    Returns analysis dict with:
    - min_power, max_power
    - contrast (visibility)
    - peak_count (approximate)
    - period_deg (approximate angular period)
    """
    if len(results) < 3:
        return {'name': name, 'error': 'Insufficient data'}

    angles = np.array([r[0] for r in results])
    powers = np.array([r[1] for r in results])

    min_power = np.min(powers)
    max_power = np.max(powers)
    contrast = (max_power - min_power) / (max_power + min_power) if (max_power + min_power) > 0 else 0

    # Count zero crossings of derivative to estimate peaks
    # Smooth with simple moving average
    smoothed = np.convolve(powers, np.ones(3)/3, mode='same')
    derivative = np.diff(smoothed)
    sign_changes = np.where(np.diff(np.sign(derivative)))[0]
    peak_count = len(sign_changes) // 2  # Each peak has 2 sign changes

    # Estimate period
    period_deg = 360.0 / max(peak_count, 1)

    return {
        'name': name,
        'min_power': min_power,
        'max_power': max_power,
        'contrast': contrast,
        'peak_count': peak_count,
        'period_deg': period_deg,
        'data': results,
    }


def identify_element(analysis: dict) -> str:
    """
    Identify optical element based on analysis.

    - 4 peaks (90 deg period) -> Half-Wave Plate
    - 2 peaks (180 deg period) with high contrast -> Linear Polarizer
    - 2 peaks (180 deg period) with lower contrast -> Quarter-Wave Plate
    """
    peaks = analysis.get('peak_count', 0)
    contrast = analysis.get('contrast', 0)

    if peaks >= 3:  # 3-4 peaks suggests HWP
        return "HALF-WAVE PLATE (4 peaks / 360 deg)"
    elif contrast > 0.5:  # High contrast with 2 peaks
        return "LINEAR POLARIZER (high contrast)"
    else:
        return "QUARTER-WAVE PLATE (low contrast)"


def main():
    parser = argparse.ArgumentParser(description='Polarization Element Characterization')
    parser.add_argument('--addr', default='localhost:50051', help='gRPC server address')
    parser.add_argument('--step', type=float, default=15.0, help='Angle step size (degrees)')
    parser.add_argument('--settle', type=float, default=0.5, help='Settle time (seconds)')
    parser.add_argument('--samples', type=int, default=3, help='Number of readings to average')
    args = parser.parse_args()

    print("=" * 60)
    print("  POLARIZATION ELEMENT CHARACTERIZATION")
    print(f"  Server: {args.addr}")
    print("=" * 60)
    print()

    # Connect to server
    print("[1/5] Connecting to gRPC server...")
    try:
        client = HardwareClient(args.addr)
        devices = client.list_devices()
    except grpc.RpcError as e:
        print(f"  ERROR: Failed to connect: {e}")
        sys.exit(1)

    print(f"  Found {len(devices)} devices:")
    movables = []
    power_meter_id = None
    for d in devices:
        status = []
        if d['is_movable']:
            status.append('movable')
            movables.append(d['id'])
        if d['is_readable']:
            status.append('readable')
            if 'power' in d['id'].lower() or 'meter' in d['id'].lower():
                power_meter_id = d['id']
        print(f"    {d['id']}: {d['name']} [{', '.join(status)}]")

    # Validate hardware
    rotator_ids = [m for m in movables if 'rotator' in m.lower()]
    if not power_meter_id:
        # Try to find any readable device
        for d in devices:
            if d['is_readable']:
                power_meter_id = d['id']
                break

    if not power_meter_id:
        print("  ERROR: No readable device (power meter) found")
        sys.exit(1)

    if len(rotator_ids) < 3:
        print(f"  WARNING: Only {len(rotator_ids)} rotators found (expected 3)")

    print(f"\n  Power meter: {power_meter_id}")
    print(f"  Rotators: {rotator_ids}")
    print()

    # Generate angles
    num_steps = int(360.0 / args.step) + 1
    angles = [i * args.step for i in range(num_steps)]
    print(f"[2/5] Scan parameters: {len(angles)} points, {args.step} deg step, {args.settle}s settle")
    print()

    # Home all rotators first
    print("[3/5] Homing all rotators...")
    for rid in rotator_ids:
        print(f"  Homing {rid} to 0 deg...")
        client.move_abs(rid, 0.0)
        time.sleep(1.5)
    print()

    # Scan each rotator
    print("[4/5] Running characterization scans...")
    all_results = {}

    for i, rotator_id in enumerate(rotator_ids):
        print(f"\n  --- Scanning {rotator_id} ({i+1}/{len(rotator_ids)}) ---")

        # Set other rotators to 0
        for other_id in rotator_ids:
            if other_id != rotator_id:
                client.move_abs(other_id, 0.0)
        time.sleep(1.0)

        # Scan
        results = scan_rotator(client, rotator_id, power_meter_id, angles,
                              settle_time=args.settle, num_samples=args.samples)
        all_results[rotator_id] = results

    # Return all rotators to 0
    print("\n  Returning rotators to home...")
    for rid in rotator_ids:
        client.move_abs(rid, 0.0)
    time.sleep(1.0)
    print()

    # Analyze results
    print("[5/5] Analyzing results...")
    print()

    analyses = {}
    for rotator_id, results in all_results.items():
        analysis = analyze_scan(results, rotator_id)
        analyses[rotator_id] = analysis

        print(f"  {rotator_id}:")
        print(f"    Min power: {analysis['min_power']:.6e} W")
        print(f"    Max power: {analysis['max_power']:.6e} W")
        print(f"    Contrast:  {analysis['contrast']*100:.1f}%")
        print(f"    Peaks:     ~{analysis['peak_count']} per 360 deg")
        print(f"    Period:    ~{analysis['period_deg']:.0f} deg")
        identification = identify_element(analysis)
        print(f"    >>> IDENTIFIED AS: {identification}")
        print()

    print("=" * 60)
    print("  CHARACTERIZATION COMPLETE")
    print()
    print("  Summary:")
    for rotator_id, analysis in analyses.items():
        ident = identify_element(analysis)
        print(f"    {rotator_id} -> {ident}")
    print()
    print("  Note: Verify by examining the peak structure in detail.")
    print("  HWP shows 4 peaks (45 deg apart), Polarizer shows 2 peaks (90 deg apart).")
    print("=" * 60)

    return 0


if __name__ == '__main__':
    sys.exit(main())
