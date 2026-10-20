"""Static MathML embeds remain usable in constrained notebook layouts."""

from html.parser import HTMLParser
from fractions import Fraction
import builtins
import json
import unittest
from unittest.mock import patch
from xml.etree import ElementTree

from symbolica import E, S, FormattedOutput, MathMLViewer, PrintMode


def mathml_value(html, variables):
    """Evaluate the arithmetic MathML subset used by these sign regressions."""
    def tokens(node):
        tag = node.tag.rsplit("}", 1)[-1]
        if tag == "mn":
            yield Fraction(node.text)
        elif tag == "mi":
            yield Fraction(variables[node.text])
        elif tag == "mspace":
            pass
        elif tag == "mo":
            operator = node.text
            if operator in ("(", ")", "+", "−"):
                yield operator
            elif operator not in ("*", "·", "×", "\u2062"):
                raise AssertionError(f"Unsupported arithmetic operator: {operator}")
        elif tag == "mfrac":
            yield value(node[0]) / value(node[1])
        elif tag == "msup":
            yield value(node[0]) ** value(node[1])
        elif tag in ("math", "mrow"):
            for child in node:
                yield from tokens(child)
        else:
            raise AssertionError(f"Unsupported arithmetic element: {tag}")

    def value(node):
        items = list(tokens(node))
        index = 0

        def factor():
            nonlocal index
            sign = 1
            while items[index] in ("+", "−"):
                if items[index] == "−":
                    sign = -sign
                index += 1
            item = items[index]
            index += 1
            if item == "(":
                item = expression()
                assert items[index] == ")"
                index += 1
            assert isinstance(item, Fraction)
            return sign * item

        def product():
            nonlocal index
            result = factor()
            while index < len(items) and items[index] not in ("+", "−", ")"):
                result *= factor()
            return result

        def expression():
            nonlocal index
            result = product()
            while index < len(items) and items[index] in ("+", "−"):
                operator = items[index]
                index += 1
                term = product()
                result += term if operator == "+" else -term
            return result

        result = expression()
        assert index == len(items)
        return result

    return value(ElementTree.fromstring(f"<math>{html}</math>"))


class MathDisplay(HTMLParser):
    def __init__(self, html):
        super().__init__()
        self.stack = []
        self.math = []
        self.feed(html)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "math":
            self.math.append((attrs, list(self.stack)))
        self.stack.append((tag, attrs))

    def handle_endtag(self, tag):
        for index in range(len(self.stack) - 1, -1, -1):
            if self.stack[index][0] == tag:
                del self.stack[index:]
                break


class MathMLDisplayTests(unittest.TestCase):
    def test_rendered_signs_preserve_the_expression_value(self):
        polynomial = E("x^4+4*y^4")
        factored = polynomial.factor()
        self.assertEqual(factored.expand(), polynomial)
        expressions = [factored, *map(E, [
            "(-x+y)*(x+y)",
            "-(-x+y)*(x+y)",
            "(-x+y)*(-x+2*y)",
            "(-x-y)*(x-y)",
            "(-x+y)/(x+y)",
            "(-x+y)*(x+y)/(x+2*y)",
            "(-x+y)*(x+y)+3*x",
            "-3*x/7",
            "x-3*y/7",
            "(-x+y)^2",
            "(-x+y)^-2",
            "(-2*x+y)^-3/7",
        ])]
        for expression in expressions:
            viewer = expression.mathml_viewer()
            try:
                page = viewer.page()
                self.assertFalse(page["markers"])
                outputs = [expression.format(mode=PrintMode.MathML), page["html"]]
                for x, y in [(2, 3), (-3, 2), (5, -1)]:
                    expected = expression.replace(S("x"), E(str(x))).replace(S("y"), E(str(y)))
                    for html in outputs:
                        with self.subTest(expression=str(expression), x=x, y=y, html=html):
                            displayed = mathml_value(html, {"x": x, "y": y})
                            self.assertEqual(E(str(displayed)), expected)
            finally:
                viewer.close()

    def test_standalone_viewer_never_imports_notebook_packages(self):
        expression = E("(1+x)^100").expand()
        original_import = builtins.__import__

        def checked_import(name, *args, **kwargs):
            if name.split(".")[0] in {"anywidget", "ipywidgets", "traitlets", "marimo", "IPython"}:
                raise AssertionError(f"Unexpected notebook dependency: {name}")
            return original_import(name, *args, **kwargs)

        with patch("builtins.__import__", checked_import):
            viewer = expression.mathml_viewer()
            self.assertIsInstance(viewer, MathMLViewer)
            page = json.loads(json.dumps(viewer.page()))
            self.assertEqual((page["start"], page["end"], page["total"]), (0, 25, 101))
            self.assertLess(len(page["html"].encode()), 256 * 1024)
            self.assertTrue(page["markers"])
            self.assertIsInstance(MathMLViewer.javascript(), str)
            formatted = expression.formatted(page_size=7)
            other = formatted.mathml_viewer()
            self.assertIs(other, formatted.mathml_viewer())
            self.assertEqual(other.page()["end"], 7)
            self.assertIsNone(FormattedOutput("plain").mathml_viewer())
            viewer.close()
            other.close()
            with self.assertRaises(RuntimeError):
                viewer.page()

    def test_worker_messages_page_forward_and_back_without_changing_expression(self):
        expression = E("(1+x)^100").expand()
        before = expression.format()
        viewer = expression.mathml_viewer(page_size=10)
        field, first = viewer.handle_message({"action": "attach", "view": "worker"})
        self.assertEqual(field, "page")
        ranges = [(first["start"], first["end"])]
        while first["next"] is not None:
            field, first = json.loads(json.dumps(viewer.handle_message({"action": "next"})))
            self.assertEqual(field, "page")
            ranges.append((first["start"], first["end"]))
        self.assertEqual(ranges[-1][1], 101)
        self.assertTrue(all(a[1] == b[0] for a, b in zip(ranges, ranges[1:])))
        for start, end in reversed(ranges[:-1]):
            field, page = viewer.handle_message({"action": "previous"})
            self.assertEqual((page["start"], page["end"]), (start, end))
        self.assertEqual(expression.format(), before)
        self.assertEqual(viewer.handle_message({"action": "dispose", "view": "worker"})[0], "")
        viewer.close()

    def test_standalone_page_size_is_bounded(self):
        for size in (0, 501):
            with self.assertRaises(ValueError):
                E("x").mathml_viewer(page_size=size)

    def test_static_output_scrolls_html_ancestors_instead_of_math(self):
        for expression in [E("sin(1/x)"), E("sqrt(1+x^2)"), E("x+y+z")]:
            for html in [expression._repr_html_(), expression.formatted()._repr_html_()]:
                with self.subTest(expression=str(expression), html=html):
                    display = MathDisplay(html)
                    self.assertEqual(len(display.math), 1)
                    attrs, ancestors = display.math[0]
                    self.assertNotIn("overflow", attrs.get("style", ""))
                    self.assertNotIn("max-width", attrs.get("style", ""))
                    self.assertTrue(any(
                        tag in ("span", "div") and "overflow-x:auto" in attrs.get("style", "")
                        for tag, attrs in ancestors
                    ))

    def test_mathml_fragments_stay_embeddable_in_math(self):
        fragment = E("sin(1/x)").format(mode=PrintMode.MathML)
        self.assertIn("<mfrac>", fragment)
        self.assertNotIn("<span", fragment)
        self.assertNotIn("<div", fragment)
        self.assertNotIn("<math", fragment)


if __name__ == "__main__":
    unittest.main()
