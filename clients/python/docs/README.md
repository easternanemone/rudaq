# rust-daq Python Client Documentation

This directory contains comprehensive Sphinx documentation for the rust-daq Python client.

## Building the Documentation

### Prerequisites

Install documentation dependencies:

```bash
cd clients/python
pip install -e ".[docs]"
```

Or install Sphinx manually:

```bash
pip install sphinx sphinx-rtd-theme
```

### Build HTML Documentation

```bash
cd docs
make html
```

The built documentation will be in `build/html/`. Open `build/html/index.html` in your browser.

### Clean Build

To clean previous builds:

```bash
make clean
make html
```

## Documentation Structure

```
docs/
├── source/
│   ├── index.rst                # Main landing page
│   ├── installation.rst         # Installation guide
│   ├── getting_started.rst      # Quick start guide
│   ├── comparison.rst           # Comparison with other frameworks
│   ├── tutorials/               # Step-by-step tutorials
│   │   ├── basic_usage.rst
│   │   ├── motor_control.rst
│   │   ├── data_acquisition.rst
│   │   └── jupyter_notebooks.rst
│   ├── guides/                  # How-to guides
│   │   ├── async_vs_sync.rst
│   │   ├── error_handling.rst
│   │   └── best_practices.rst
│   └── api/                     # API reference
│       ├── core.rst             # AsyncClient
│       ├── devices.rst          # Motor, Detector, Status
│       └── exceptions.rst       # Exception hierarchy
└── build/
    └── html/                    # Built HTML files (generated)
```

## Viewing Documentation Locally

After building:

```bash
# On macOS
open build/html/index.html

# On Linux
xdg-open build/html/index.html

# Or use Python's http.server
cd build/html
python -m http.server 8000
# Then visit http://localhost:8000
```

## Adding New Documentation

1. Create `.rst` file in `source/` or appropriate subdirectory
2. Add to `toctree` in parent index file
3. Rebuild documentation

Example:

```rst
My New Guide
============

Introduction text here.

Section 1
---------

Content...
```

## Updating API Documentation

API documentation is auto-generated from docstrings. To update:

1. Edit docstrings in Python source files (`src/rust_daq/*.py`)
2. Use Google-style docstrings format
3. Rebuild documentation

Example docstring:

```python
def my_function(arg1: str, arg2: int) -> bool:
    """
    Short description.

    Longer description with more details.

    Args:
        arg1: Description of arg1
        arg2: Description of arg2

    Returns:
        Description of return value

    Raises:
        ValueError: When something is wrong

    Example:
        >>> my_function("test", 42)
        True
    """
    pass
```

## Configuration

Documentation configuration is in `source/conf.py`. Key settings:

- `extensions`: Sphinx extensions (autodoc, napoleon, etc.)
- `html_theme`: Documentation theme (sphinx_rtd_theme)
- `autodoc_*`: Automatic documentation options
- `napoleon_*`: Google/NumPy docstring parsing options

## Troubleshooting

**Missing theme error:**
```bash
pip install sphinx-rtd-theme
```

**Import errors during build:**
```bash
# Make sure package is installed
cd ../..  # Back to clients/python
pip install -e .
```

**Warnings about duplicate descriptions:**
These are expected and can be ignored. They occur when explicitly documenting members that are also auto-documented.

## Publishing Documentation

For Read the Docs or GitHub Pages:

1. Ensure `docs/requirements.txt` exists with Sphinx dependencies
2. Configure `.readthedocs.yaml` or GitHub Pages settings
3. Documentation builds automatically on push

## Contributing

When adding new features:

1. Add comprehensive docstrings to all public APIs
2. Create tutorial or guide if appropriate
3. Update API reference if needed
4. Build and verify documentation locally
5. Include in pull request

## Links

- [Sphinx Documentation](https://www.sphinx-doc.org/)
- [reStructuredText Primer](https://www.sphinx-doc.org/en/master/usage/restructuredtext/basics.html)
- [Read the Docs Theme](https://sphinx-rtd-theme.readthedocs.io/)
