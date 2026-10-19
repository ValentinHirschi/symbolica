<h1 align="center">
  <br>
  <picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://symbolica.io/logo_dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="https://symbolica.io/logo.svg">
  <img src="https://symbolica.io/logo.svg" alt="logo" width="200">
</picture>
  <br>
</h1>

<p align="center">
<a href="https://symbolica.io"><img alt="Symbolica website" src="https://img.shields.io/static/v1?label=symbolica&message=website&color=orange&style=flat-square"></a>
  <a href="https://zulip.symbolica.io"><img alt="Zulip Chat" src="https://img.shields.io/static/v1?label=zulip&message=discussions&color=blue&style=flat-square"></a>
    <a href="https://github.com/symbolica-dev/symbolica"><img alt="Symbolica repository" src="https://img.shields.io/static/v1?label=github&message=development&color=green&style=flat-square&logo=github"></a>
    <a href="https://app.codecov.io/gh/symbolica-dev/symbolica"><img alt="Codecov" src="https://img.shields.io/codecov/c/github/symbolica-dev/symbolica?token=rhCESyNPk7&style=flat-square"></a>
</p>

# Symbolica ⊆ Modern Computer Algebra

Symbolica is a high-performance computer algebra library for Python and Rust. It
is built for large expressions, symbolic rewrites, exact polynomial arithmetic,
and optimized numerical evaluators.

Trusted by CERN and research groups at ETH Zurich, the University of Zurich, the
University of Bern, and Karlsruhe Institute of Technology.

Try the live [Jupyter Notebook demo](https://colab.research.google.com/drive/1VAtND2kddgBwNt1Tjsai8vnbVIbgg-7D?usp=sharing),
read the [documentation](https://symbolica.io/docs/), or see
[symbolica.io](https://symbolica.io) for licensing and support.

## Why Symbolica?

- Native Python and Rust APIs for the same symbolic core
- Optimized numerical evaluators, with JIT, C++, SIMD, ASM, and CUDA code generation
- Fast multivariate polynomial arithmetic for large symbolic workloads
- Exact solutions of polynomial and rational equation systems
- Pattern matching and rewrites for domain-specific algebra
- Mixed exact and numerical computation with error propagation
- Streaming tools for expressions too large to keep in memory
- Run symbolic computations directly in the browser with WebAssembly

# Installation

Visit the [Get Started](https://symbolica.io/docs/get_started.html) page for detailed installation instructions.

## Python

Symbolica can be installed from PyPI using `pip`:

```sh
pip install symbolica
```

## Rust

Add Symbolica to a Rust project:

```sh
cargo add symbolica
```

# Example

Go from a physical model to a calibrated result in a few steps.
Explore more in the [guide](https://symbolica.io/docs/).

### Pendulum calibration

Start with a pendulum whose restoring torque is controlled by the scale `κ`:

```python
from symbolica import *

θ, κ = S("θ", "κ")

V = κ*(1 - θ.cos())
τ = -V.derivative(θ)

τ
```

```math
-\kappa \sin\!\left(\theta\right)
```

Expand the torque to get a small-angle model:

```python
τ_small = τ.series(θ, 0, 3)
τ_small
```

```math
-\kappa\theta+\frac{1}{6}\kappa\theta^3+\mathcal{O}\!\left(\theta^4\right)
```

Suppose the scale `κ` and a sensor offset `τ_0` are unknown. Each pair
`(θ_i, τ_i)` is one sensor reading: at angle `θ_i`, the measured torque is
`τ_i`. Convert the truncated series back to an expression, evaluate it at two
measurement angles using `replace`, and solve the resulting linear system:

```python
τ0, τ1, τ2, θ1, θ2 = S("τ_0", "τ_1", "τ_2", "θ_1", "θ_2")

τ_model = τ_small.to_expression() + τ0

solution = Expression.solve([
    τ_model.replace(θ, θ1) - τ1,
    τ_model.replace(θ, θ2) - τ2,
], [κ, τ0])[0]

κ_fit, τ0_fit = solution[κ], solution[τ0]

κ_fit
```

```math
\frac{6\tau_1-6\tau_2}{-6\theta_1+6\theta_2+\theta_1^3-\theta_2^3}
```

Finally, plug in measured values:

```python
κ_fit.evaluate({
    θ1: 0.10,
    θ2: 0.20,
    τ1: -0.4697,
    τ2: -0.9545,
}).real
```

```text
4.905227655986509
```

## Development

Follow the development of Symbolica and the open-source spin-off projects [numerica](https://github.com/symbolica-dev/numerica), [graphica](https://github.com/symbolica-dev/graphica), and [symbolica-integrate](https://github.com/symbolica-dev/symbolica-integrate) on [Zulip](https://zulip.symbolica.io)!

### Notebook math display

Python `Expression` objects display as native MathML in notebooks. Large expressions
show a bounded preview and, with `pip install 'symbolica[notebook]'`, a live widget
that pages through terms, factors, or arguments and opens omitted subexpressions.
Math uses STIX Two Math at 21px. The widget uses a locally installed copy when
available, otherwise loading the full font from a pinned CDN URL.
Click or Tab into an expression to zoom with `+` / `=` and `-`; press `0` to reset.
Zoom ranges from 25–3200%, runs locally in the browser, and persists while navigating
the expression. Small expressions have no permanent toolbar; a brief percentage
indicator appears when zooming. Browser shortcuts such as Ctrl/Cmd+Plus are unaffected.
Nested detail adapts to the available width, preserving the enclosing structure;
clicking a marker opens a popup with its own page size and Previous/Next controls. Large function arguments appear
on separate lines. Abbreviated calls retain their actual names, as `F([1])` or
`F(x, [1])`. Omission markers
use `+ [1]` for sums and `· [2]` for products: click to browse the popup, or hover to preview the next
few omitted items. Show more here reveals another batch inside the original expression,
without changing its other parts. The popup’s Inspect button opens the subexpression in the main view;
Back to parent returns. Popup pagination is independent of the parent and other markers.
Popups also fit their detail to the available width and retain clickable nested
markers. Clicking one opens that part in the same box; its Back button restores
the previous popup and page.
Long product pages and expanded products use multiple MathML rows, with explicit
multiplication operators on continuation lines. Expansion shares the rendering budget
across factors; large sums inside them remain independently inspectable.
Previews are fetched lazily without changing the current page.
The complete expression stays in the kernel; only bounded layouts for the current page and requested hover
previews are sent to the browser. Resizing selects a layout locally, without kernel messages.

```python
from symbolica import E

expr = E('(x + y + 1)^100').expand()
expr                              # automatic MathML and pagination
expr.formatted(page_size=100)      # 1–500 items per page, with formatting options
expr.formatted(mathml=False)       # previous highlighted text display
```

Plain string formatting and explicit LaTeX output remain available through `format()`
and `to_latex()`. Text-layout options and custom function printers use the text view.
Static notebook exports contain the current preview; navigation needs a live kernel.

### Reusing the notebook viewer

`MathMLViewer`, `MathMLPage`, `MathMLMarker`, and `MathMLPageSource` are public Python
exports. Community types can supply a page renderer and reuse the same JavaScript,
font loading, zoom, hover, popup pagination, inspection, and notebook integration:

```python
from symbolica import MathMLViewer, MathMLPage, MathMLMarker

class MyPages:
    def page(self, path, start, size, layout=0, expansions=()):
        total = 1_000_000
        end = min(start + size, total)
        rows = "".join(f"<mtr><mtd><mn>{i}</mn></mtd></mtr>"
                       for i in range(start, end))
        markers = [MathMLMarker([], "Inspect entries [1]", end)] if end < total else []
        tail = MathMLMarker.marker(0) if markers else ""
        html = f"<math><mtable>{rows}</mtable>{tail}</math>"
        return MathMLPage(html, start, end, total, markers=markers)

viewer = MathMLViewer(MyPages(), page_size=25)
# Display viewer in Jupyter, or return it from a marimo cell.
```

The provider contract is:

- `path` is an absolute list of nonnegative integers; `[]` denotes the root.
  It may identify child indices or provider-owned handles. Expressions and cursor
  tables remain in the provider, never in widget state.
- A page returns MathML and the half-open item range `[start, end)`, its `total`,
  and optional `markers`. A `MathMLMarker(path, label, start=0)` records a target and
  the first omitted item. Insert `MathMLMarker.marker(i)` at its position in the
  MathML, where `i` is its index in `page.markers`.
- `layout_count=1` is the default: one render per uncached page. Providers with
  cheaper responsive layouts can supply 1–8 alternatives, from most detailed to
  most compact, selected locally by the browser. Optionally mark corresponding
  nested MathML subtrees with `data-symbolica-path="stable-key"` (unique,
  nonempty keys identifying the same occurrence across layouts). The browser
  can then reveal more detailed subtrees wherever they fit, using only the
  supplied pages. Markers retain their source layout for navigation. Local
  fitting is limited to 64 replacement attempts and 10,000 DOM elements.
- `supports_expansion=False` is the default. Enable it only when the renderer
  honors `(relative_path, visible_item_count)` entries in `expansions`, preserving
  those branches in compact layouts. This enables Show more here. Tree paths are
  required for expansion; opaque handles can still use pagination and inspection.
- Providers must bound traversal and rendering. The viewer also rejects HTML
  above 256 KiB and page metadata above a combined 512 KiB per layout. It retains
  at most three pages, fetches popups on demand, and serializes only documented
  fields. `viewer.close()` releases the source and cached pages.

A native PyO3 provider may return a dictionary with `html`, `start`, `end`, `total`,
`kind`, `unit`, and optional `markers` (pairs of absolute path and label), `marker_ids`
(marker indices), and `marker_starts` (first omitted offsets). The latter arrays
align with `markers`; omitted ids default to consecutive indices and offsets to zero.
Rust community modules using the `python_export` feature can construct all three
objects directly. `MathMLMarker::new` and `MathMLPage::new` create ordinary Rust
values; their `into_python(py)` methods construct the corresponding public Python
objects. A `MathMLViewer` wraps a native `MathMLPageSource` without requiring a
custom PyO3 class or hand-built dictionaries:

```rust,ignore
use pyo3::prelude::*;
use symbolica::api::python::{
    MathMLMarker, MathMLPage, MathMLPageRequest, MathMLPageSource, MathMLViewer,
};

struct TensorPages { /* Arc<TensorExpression>, compiler, cursor cache, ... */ }

impl MathMLPageSource for TensorPages {
    fn page(&mut self, py: Python<'_>, request: MathMLPageRequest<'_>)
        -> PyResult<MathMLPage>
    {
        // Render a bounded page, using request.path/start/size. The GIL token
        // is available for adapters calling a Python-based Typst compiler.
        // Return MathMLPage::new(html, start, end, total), with optional
        // page.markers.push(MathMLMarker::new(target_path, label, offset)).
        todo!("call the tensor renderer")
    }
}

fn viewer(py: Python<'_>, source: TensorPages) -> PyResult<Py<PyAny>> {
    MathMLViewer::new(source).page_size(25).into_python(py)
}
```

The source is mutable and `Send + 'static`; native expression ownership can stay
in an `Arc`. Rendering and optional notebook imports remain lazy. Layouts and
inline expansion are opt-in through `.layout_count(n)` and
`.supports_expansion(true)`. The viewer owns the source until `close()` or
collection. [The Rust integration test](tests/mathml_viewer_rust.rs) exercises
construction, focus handles, paging, caching and source release.

Existing Python/PyO3 sources can still use
`symbolica::api::python::mathml_viewer(py, source, page_size, layout_count, supports_expansion)`.
No JavaScript, marimo, or Jupyter code is needed in the provider.

[The custom provider example](examples/mathml_custom_viewer.py) exposes a virtual
100-million-component tensor and evaluates only requested components. For
TensorExpression, an adapter can retain its existing bounded Typst compiler,
map absolute paths to its native focus handles, and map item offsets to its
native continuation cursors. It then returns the compiled MathML and omission
metadata through this contract. Start with one layout and expansion disabled;
additional layout/expansion support can be added in its renderer independently.
The adapter must also replace Typst's rendered omission labels with
`MathMLMarker::marker(i)` (or its Python equivalent), preserving the matching
marker metadata. An opaque native cursor is not an item offset: cache cursors
by focus and offset, and retain enough information to serve hover offsets and
changed page sizes. This keeps cursor handling and Typst compilation inside
spenso rather than coupling the shared frontend to either implementation.
