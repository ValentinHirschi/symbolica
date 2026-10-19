"""A custom tensor-like provider using only Symbolica's public viewer API.

The callback is evaluated only for requested components. No full tensor array
or complete MathML document is built, even for millions of components. A real
TensorExpression adapter can replace the callback with its bounded renderer.
"""
from html import escape
from symbolica import MathMLMarker, MathMLPage, MathMLViewer


class TensorComponentSource:
    def __init__(self, dimension, component):
        self.dimension = dimension
        self.component = component

    def page(self, path, start, size, layout=0, expansions=()):
        if path == []:
            marker = MathMLMarker([0], "Inspect components [1]")
            html = f'<math><mi>T</mi><mo>(</mo>{marker.marker(0)}<mo>)</mo></math>'
            return MathMLPage(html, 0, 1, 1, kind="Tensor", markers=[marker])
        if path != [0]:
            raise ValueError("Unknown component path")
        total = self.dimension ** 2
        end = min(start + size, total)
        rows = []
        for index in range(start, end):
            i, j = divmod(index, self.dimension)
            value = escape(str(self.component(i, j)))
            rows.append(f'<mtr><mtd><msub><mi>T</mi><mrow><mn>{i}</mn><mo>,</mo>'
                        f'<mn>{j}</mn></mrow></msub></mtd><mtd><mo>=</mo>'
                        f'<mn>{value}</mn></mtd></mtr>')
        markers = [MathMLMarker([0], "Inspect components [1]", end)] if end < total else []
        tail = markers[0].marker(0) if markers else ''
        html = f'<math><mtable>{"".join(rows)}</mtable>{tail}</math>'
        return MathMLPage(html, start, end, total, "Tensor", "components", markers)


def example_viewer():
    return MathMLViewer(TensorComponentSource(10_000, lambda i, j: i + j))


if __name__ == '__main__':
    # In Jupyter, display this object. In marimo, return it from a cell.
    viewer = example_viewer()
    print(viewer._repr_html_())
