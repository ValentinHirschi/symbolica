"""Static MathML embeds remain usable in constrained notebook layouts."""

from html.parser import HTMLParser
import unittest

from symbolica import E, PrintMode


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
