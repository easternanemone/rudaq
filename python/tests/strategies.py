from __future__ import annotations

from hypothesis import strategies as st

JSON_PRIMITIVE = st.one_of(
    st.none(),
    st.booleans(),
    st.integers(min_value=-10**6, max_value=10**6),
    st.floats(width=64, allow_nan=False, allow_infinity=False),
    st.text(max_size=32),
)

JSON_METADATA = st.recursive(
    JSON_PRIMITIVE,
    lambda children: st.one_of(
        st.lists(children, max_size=4),
        st.dictionaries(st.text(max_size=16), children, max_size=4),
    ),
    max_leaves=10,
)

__all__ = ["JSON_METADATA", "JSON_PRIMITIVE"]
