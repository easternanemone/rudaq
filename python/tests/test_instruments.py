from __future__ import annotations

import pytest
from hypothesis import given, strategies as st

from rust_daq import MaiTai, Newport1830C


def test_maitai_initialization_logs(capsys) -> None:
    MaiTai("COM42")
    captured = capsys.readouterr()

    assert "MaiTai" in captured.out
    assert "COM42" in captured.out


@given(wavelength=st.floats(min_value=690.0, max_value=1100.0, allow_nan=False, allow_infinity=False))
def test_maitai_set_wavelength_accepts_valid_values(capsys, mai_tai, wavelength) -> None:
    mai_tai.set_wavelength(wavelength)
    captured = capsys.readouterr()

    assert "MaiTai" in captured.out
    assert f"{wavelength}" in captured.out


def test_maitai_rejects_invalid_port_type() -> None:
    with pytest.raises(TypeError):
        MaiTai(123)  # type: ignore[arg-type]


def test_newport_initialization_logs(capsys) -> None:
    Newport1830C("USB::TEST")
    captured = capsys.readouterr()

    assert "Newport1830C" in captured.out
    assert "USB::TEST" in captured.out


def test_newport_read_power_returns_mock_value(newport_meter) -> None:
    power = newport_meter.read_power()

    assert isinstance(power, float)
    assert power == pytest.approx(1.23e-3)


def test_newport_rejects_invalid_resource_type() -> None:
    with pytest.raises(TypeError):
        Newport1830C(None)  # type: ignore[arg-type]
