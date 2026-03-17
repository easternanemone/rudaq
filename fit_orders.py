import numpy as np

y_detected = np.array([209, 241, 398, 436, 623, 700, 825, 1062, 1159, 1397, 1497])

# Try to find a sequence of integer order numbers m_i that makes y(m_i) a smooth function
for start_m in range(30, 80):
    # let's just try to assign delta_m for each step
    # assume y(m) is roughly quadratic in m
    # we want to minimize the variance of second differences, or similar
    pass

# A better way: just do a combinatorial search of delta_m's.
# We know the gaps are [32, 157, 38, 187, 77, 125, 237, 97, 238, 100]
# We can estimate delta_m by assuming the gap per order is smoothly increasing.
gaps = np.diff(y_detected)
# print(gaps)

# Let's try to fit y(m) = a * m^2 + b * m + c
# We want integer m_i such that y_detected[i] \approx y(m_i)
# Let's just find the m_i that give a smooth d(y)/dm.
