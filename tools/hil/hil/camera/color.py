import numpy as np


def srgb_to_linear(rgb01):
    a = np.asarray(rgb01, dtype=np.float64)
    return np.where(a <= 0.04045, a / 12.92, ((a + 0.055) / 1.055) ** 2.4)


def luminance_linear(rgb01):
    lin = srgb_to_linear(rgb01)
    return 0.2126 * lin[..., 0] + 0.7152 * lin[..., 1] + 0.0722 * lin[..., 2]


def xy_chromaticity(rgb01):
    lin = srgb_to_linear(np.asarray(rgb01, dtype=np.float64))
    r, g, b = lin[..., 0], lin[..., 1], lin[..., 2]
    x = 0.4124 * r + 0.3576 * g + 0.1805 * b
    y = 0.2126 * r + 0.7152 * g + 0.0722 * b
    z = 0.0193 * r + 0.1192 * g + 0.9505 * b
    s = x + y + z
    with np.errstate(invalid="ignore", divide="ignore"):
        return x / s, y / s


def mccamy_cct(x, y):
    n = (x - 0.3320) / (0.1858 - y)
    return 449.0 * n ** 3 + 3525.0 * n ** 2 + 6823.3 * n + 5520.33
