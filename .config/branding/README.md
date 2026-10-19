# Branding

The brand colour tokens. **Draft:** the design story is not finished, so every value is provisional.

| File | Is |
|---|---|
| `colors.css` | The tokens: five pigment ramps, Mars black, and a light and a dark value for each role. Source of truth |
| `check.py` | Checks the contrast of the text pairs (exit 1 if one fails) and reports which status colours colour blindness cannot separate. `uv run .config/branding/check.py` |

The palette, its roles, the measured results and the open design questions are in the book, under `docs/src/uiux/`.
The book reads this directory through the `docs/branding` symlink.
