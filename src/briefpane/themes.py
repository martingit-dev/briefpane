"""Matrix is the default; dark and light follow. ``t`` cycles them."""

from textual.theme import Theme

from .view import Palette

MATRIX = Theme(
    name="matrix",
    primary="#00ff41",
    secondary="#00b830",
    accent="#7dff9b",
    foreground="#33ff66",
    background="#000000",
    surface="#030a04",
    panel="#062b0e",
    success="#00ff41",
    warning="#c8ff00",
    error="#ff3b3b",
    dark=True,
)

DARK = Theme(
    name="briefpane-dark",
    primary="#e6e6e6",
    secondary="#9a9a9a",
    accent="#7fb4ff",
    foreground="#e6e6e6",
    background="#111111",
    surface="#181818",
    panel="#242424",
    success="#7ad97a",
    warning="#e0b252",
    error="#ff6b6b",
    dark=True,
)

LIGHT = Theme(
    name="briefpane-light",
    primary="#1a1a1a",
    secondary="#666666",
    accent="#1f5fbf",
    foreground="#1a1a1a",
    background="#fbfbf8",
    surface="#f2f2ee",
    panel="#e6e6e0",
    success="#2f7d32",
    warning="#8a6100",
    error="#c62828",
    dark=False,
)

THEMES = {"matrix": MATRIX, "dark": DARK, "light": LIGHT}
ORDER = list(THEMES)

PALETTES = {
    MATRIX.name: Palette(
        text="#2fd65a", dim="#1b7a36", strong="#b6ffb0", prompt="#e8ffe8", code="#d4ff3a"
    ),
    DARK.name: Palette(
        text="#cfcfcf", dim="#7a7a7a", strong="#ffffff", prompt="#7fb4ff", code="#e0b252"
    ),
    LIGHT.name: Palette(
        text="#2b2b2b", dim="#8a8a8a", strong="#000000", prompt="#1f5fbf", code="#8a4f00"
    ),
}
