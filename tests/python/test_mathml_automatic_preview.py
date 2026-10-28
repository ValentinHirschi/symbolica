"""Automatic product previews retain small complete tails without changing paging."""

import unittest

from symbolica import E, MathMLViewer


class ProductSource:
    def __init__(self, total=34, candidate="complete", kind="Product"):
        self.total = total
        self.candidate = candidate
        self.kind = kind
        self.calls = []

    def page(self, path, start, size, layout, expansions):
        self.calls.append((tuple(path), start, size, layout, tuple(expansions)))
        probing = size == self.total and size > 25
        if probing and self.candidate == "error":
            raise ValueError("temporary candidate failure")
        end = min(start + size, self.total)
        if probing and self.candidate == "partial":
            end -= 1
        # Indexed factors are small in the rendered output even when their
        # provider-side expressions contain much larger tensor descriptors.
        parts = [f"<msup><mi>g</mi><mrow><mi>a</mi><mn>{i}</mn></mrow></msup>"
                 for i in range(start, end)]
        markers, starts = [], []
        if end < self.total:
            markers.append((list(path), "Inspect product"))
            starts.append(end)
            parts.append('<mtext data-symbolica-marker="0">[1]</mtext>')
        if probing:
            if self.candidate == "bytes":
                parts.append(f"<mtext>{'x' * 16_384}</mtext>")
            elif self.candidate == "elements":
                parts.extend("<mrow/>" for _ in range(1000))
            elif self.candidate == "hidden":
                parts.append('<mtext data-symbolica-marker="99">[100]</mtext>')
        return dict(html=f"<math><mrow>{''.join(parts)}</mrow></math>",
                    start=start, end=end, total=self.total, kind=self.kind,
                    markers=markers, marker_starts=starts)


class AutomaticProductPreviewTests(unittest.TestCase):
    def viewer(self, source, automatic=True):
        viewer = MathMLViewer(source, supports_expansion=True,
                              _automatic_product_preview=automatic)
        self.addCleanup(viewer.close)
        return viewer

    def send(self, viewer, action, **kwargs):
        field, page = viewer.handle_message(dict(action=action, **kwargs))
        self.assertEqual(field, "page")
        self.assertNotIn("error", page)
        return page

    def test_small_complete_product_uses_one_cached_candidate(self):
        source = ProductSource()
        viewer = self.viewer(source)
        page = viewer.page()
        self.assertEqual((page["end"], page["total"], page["page_size"]), (34, 34, 34))
        self.assertFalse(page["markers"])
        self.assertFalse(page["can_collapse"])
        self.assertEqual([call[2] for call in source.calls], [25, 34])
        calls = list(source.calls)
        self.assertEqual(viewer.page()["html"], page["html"])
        self.assertEqual(viewer._render_page(0)["html"], page["html"])
        self.assertEqual(source.calls, calls)

    def test_explicit_size_and_later_navigation_remain_exact(self):
        explicit_source = ProductSource()
        explicit = self.viewer(explicit_source, automatic=False)
        self.assertEqual(explicit.page()["end"], 25)
        self.assertEqual([call[2] for call in explicit_source.calls], [25])

        source = ProductSource()
        automatic = self.viewer(source)
        self.assertEqual(automatic.page()["end"], 34)
        selected = self.send(automatic, "size", value=25)
        self.assertEqual((selected["end"], selected["page_size"]), (25, 25))
        self.assertEqual(automatic.page()["end"], 25)
        later = self.send(automatic, "next")
        self.assertEqual((later["start"], later["end"]), (25, 34))
        self.assertEqual(self.send(automatic, "previous")["end"], 25)
        self.assertEqual(self.send(automatic, "collapse")["end"], 25)
        self.assertEqual(sum(call[2] == 34 for call in source.calls), 1)

    def test_incomplete_or_large_candidates_keep_the_original_page(self):
        for candidate in ["partial", "bytes", "elements", "hidden"]:
            with self.subTest(candidate=candidate):
                source = ProductSource(candidate=candidate)
                viewer = self.viewer(source)
                page = viewer.page()
                self.assertEqual((page["end"], page["page_size"]), (25, 25))
                self.assertTrue(page["markers"])
                self.assertEqual([call[2] for call in source.calls], [25, 34])
                calls = list(source.calls)
                self.assertEqual(viewer.page()["html"], page["html"])
                self.assertEqual(source.calls, calls)

    def test_sums_and_products_beyond_one_extra_page_are_not_probed(self):
        for source in [ProductSource(kind="Sum"), ProductSource(total=51)]:
            viewer = self.viewer(source)
            self.assertEqual(viewer.page()["end"], 25)
            self.assertEqual([call[2] for call in source.calls], [25])

    def test_optional_candidate_errors_are_not_cached(self):
        source = ProductSource(candidate="error")
        viewer = self.viewer(source)
        self.assertEqual(viewer.page()["end"], 25)
        source.candidate = "complete"
        self.assertEqual(viewer.page()["end"], 34)
        self.assertEqual([call[2] for call in source.calls], [25, 34, 34])

    def test_native_expression_default_and_explicit_sizes(self):
        expression = E("*".join(f"x{i}" for i in range(34)))
        automatic = expression.mathml_viewer()
        explicit = expression.mathml_viewer(25)
        self.addCleanup(automatic.close)
        self.addCleanup(explicit.close)
        self.assertEqual(automatic.page()["end"], 34)
        self.assertFalse(automatic.page()["markers"])
        self.assertEqual(explicit.page()["end"], 25)

        # Increasing the item allowance cannot make a genuinely large product
        # eligible: its complete output still contains budget omissions.
        large_sum = "+".join(f"y{i}" for i in range(100))
        complex_product = E("*".join(f"f{i}({large_sum})" for i in range(34)))
        automatic = complex_product.mathml_viewer()
        explicit = complex_product.mathml_viewer(25)
        self.addCleanup(automatic.close)
        self.addCleanup(explicit.close)
        self.assertEqual(automatic.page()["html"], explicit.page()["html"])
        self.assertTrue(automatic.page()["markers"])


if __name__ == "__main__":
    unittest.main()
