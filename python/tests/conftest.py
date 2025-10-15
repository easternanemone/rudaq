from __future__ import annotations

from datetime import datetime, timezone
from typing import Any, Callable, Dict, Optional

import pytest
from rust_daq import DataPoint, MaiTai, Newport1830C

from .strategies import JSON_METADATA


@pytest.fixture
def utc_now() -> datetime:
    """Return a timezone-aware UTC timestamp suitable for PyO3 conversions."""
    return datetime.now(timezone.utc)


@pytest.fixture
def datapoint_factory() -> Callable[[str, float, str, Optional[Dict[str, Any]], Optional[datetime]], DataPoint]:
    """Construct a DataPoint with sensible defaults, overriding as needed."""

    def factory(
        channel: str = "signal:0",
        value: float = 0.0,
        unit: str = "V",
        metadata: Optional[Dict[str, Any]] = None,
        timestamp: Optional[datetime] = None,
    ) -> DataPoint:
        return DataPoint(
            timestamp=timestamp or datetime.now(timezone.utc),
            channel=channel,
            value=value,
            unit=unit,
            metadata=metadata,
        )

    return factory


@pytest.fixture
def mai_tai() -> MaiTai:
    """Provide a mock MaiTai laser connection for integration tests."""

    return MaiTai("COM_TEST")


@pytest.fixture
def newport_meter() -> Newport1830C:
    """Provide a mock Newport 1830C power meter connection."""

    return Newport1830C("USB::0x0::0x01::INSTR")


__all__ = [
    "JSON_METADATA",
    "DataPoint",
    "MaiTai",
    "Newport1830C",
]
