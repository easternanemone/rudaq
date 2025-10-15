from __future__ import annotations

import json
from datetime import datetime, timezone

import pytest
from hypothesis import given

from rust_daq import DataPoint

from .strategies import JSON_METADATA


def test_datapoint_creation_with_metadata(utc_now, datapoint_factory) -> None:
    metadata = {"exposure_ms": 12.5, "laser": {"wavelength_nm": 780}}
    datapoint = datapoint_factory(metadata=metadata, timestamp=utc_now)

    assert datapoint.timestamp == utc_now
    assert datapoint.channel == "signal:0"
    assert datapoint.value == pytest.approx(0.0)
    assert datapoint.unit == "V"
    assert datapoint.metadata == metadata


def test_datapoint_without_metadata(utc_now) -> None:
    datapoint = DataPoint(utc_now, "signal:detector", 5.2, "mW", None)

    assert datapoint.metadata is None


@given(metadata=JSON_METADATA)
def test_datapoint_metadata_roundtrip(metadata) -> None:
    timestamp = datetime.now(timezone.utc)
    datapoint = DataPoint(timestamp, "signal:random", 1.23, "V", metadata)

    if metadata is None:
        assert datapoint.metadata is None
    else:
        assert datapoint.metadata == metadata
        # Metadata should stay JSON-serialisable across the boundary.
        assert json.loads(json.dumps(datapoint.metadata)) == json.loads(json.dumps(metadata))


def test_datapoint_repr_contains_key_fields(datapoint_factory) -> None:
    datapoint = datapoint_factory(channel="laser:power", value=3.14, unit="mW")
    representation = repr(datapoint)

    assert "laser:power" in representation
    assert "mW" in representation


def test_datapoint_rejects_naive_datetime() -> None:
    naive_timestamp = datetime.now()

    with pytest.raises(TypeError):
        DataPoint(naive_timestamp, "signal:0", 1.0, "V", None)


def test_datapoint_independent_of_original_metadata() -> None:
    timestamp = datetime.now(timezone.utc)
    metadata = {"detector": {"name": "PMT", "gain": 2}}
    datapoint = DataPoint(timestamp, "signal:0", 1.0, "V", metadata)

    metadata["detector"]["gain"] = 10

    assert datapoint.metadata["detector"]["gain"] == 2
