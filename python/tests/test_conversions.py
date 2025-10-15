from __future__ import annotations

import json
from datetime import datetime, timezone

import pytest
from hypothesis import given, strategies as st

from rust_daq import DataPoint

from .strategies import JSON_METADATA


UTC_DATETIMES = st.datetimes(
    min_value=datetime(2020, 1, 1, tzinfo=timezone.utc),
    max_value=datetime(2035, 12, 31, tzinfo=timezone.utc),
    timezones=st.just(timezone.utc),
)


@given(timestamp=UTC_DATETIMES)
def test_datetime_roundtrip(timestamp: datetime) -> None:
    datapoint = DataPoint(timestamp, "detector:signal", 0.5, "V", None)

    assert datapoint.timestamp == timestamp
    assert datapoint.timestamp.tzinfo is not None
    assert datapoint.timestamp.tzinfo.utcoffset(datapoint.timestamp).total_seconds() == 0


@given(metadata=JSON_METADATA)
def test_metadata_json_roundtrip(metadata) -> None:
    datapoint = DataPoint(datetime.now(timezone.utc), "detector:signal", 0.5, "V", metadata)

    if metadata is None:
        assert datapoint.metadata is None
    else:
        assert datapoint.metadata == metadata
        assert json.loads(json.dumps(datapoint.metadata)) == json.loads(json.dumps(metadata))


def test_invalid_metadata_type_raises() -> None:
    bad_metadata = {"unsupported": {"value": {1, 2, 3}}}

    with pytest.raises(TypeError):
        DataPoint(datetime.now(timezone.utc), "detector:signal", 0.5, "V", bad_metadata)


@given(value=st.floats(min_value=-1e6, max_value=1e6, allow_infinity=False, allow_nan=False))
def test_float_precision_preserved(value: float) -> None:
    datapoint = DataPoint(datetime.now(timezone.utc), "detector:signal", value, "V", None)

    assert datapoint.value == pytest.approx(value)
