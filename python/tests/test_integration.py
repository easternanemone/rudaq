from __future__ import annotations

from datetime import datetime, timezone
from typing import Dict

import pytest
from hypothesis import given, strategies as st

from rust_daq import DataPoint


STRATEGY_CHANNEL = st.text(min_size=1, max_size=24).filter(lambda value: ":" in value or value.isidentifier())


def _build_metadata(wavelength: float, power: float) -> Dict[str, float]:
    return {
        "instrument": {
            "laser": {"wavelength_nm": wavelength},
            "power_meter": {"reading_w": power},
        }
    }


def test_end_to_end_data_acquisition_flow(mai_tai, newport_meter) -> None:
    wavelength_nm = 795.0
    mai_tai.set_wavelength(wavelength_nm)
    power_reading = newport_meter.read_power()

    datapoint = DataPoint(
        datetime.now(timezone.utc),
        "mai_tai:power",
        power_reading,
        "W",
        _build_metadata(wavelength_nm, power_reading),
    )

    assert datapoint.metadata["instrument"]["laser"]["wavelength_nm"] == pytest.approx(wavelength_nm)
    assert datapoint.metadata["instrument"]["power_meter"]["reading_w"] == pytest.approx(power_reading)
    assert datapoint.value == pytest.approx(power_reading)
    assert datapoint.unit == "W"


@given(
    wavelength=st.floats(min_value=690.0, max_value=1100.0, allow_nan=False, allow_infinity=False),
    channel=STRATEGY_CHANNEL,
)
def test_pipeline_handles_randomised_inputs(mai_tai, newport_meter, wavelength, channel) -> None:
    mai_tai.set_wavelength(wavelength)
    reading = newport_meter.read_power()

    datapoint = DataPoint(
        datetime.now(timezone.utc),
        channel,
        reading,
        "W",
        _build_metadata(wavelength, reading),
    )

    assert datapoint.channel == channel
    assert datapoint.metadata["instrument"]["laser"]["wavelength_nm"] == pytest.approx(wavelength)


def test_pipeline_rejects_invalid_metadata(mai_tai) -> None:
    metadata = {"instrument": {"laser": mai_tai}}

    with pytest.raises(TypeError):
        DataPoint(datetime.now(timezone.utc), "mai_tai:power", 0.0, "W", metadata)


def test_invalid_wavelength_type_raises(mai_tai) -> None:
    with pytest.raises(TypeError):
        mai_tai.set_wavelength("800")  # type: ignore[arg-type]
