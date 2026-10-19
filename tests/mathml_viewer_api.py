"""Public provider API: no Symbolica expression or private source is required."""
import importlib.util
from pathlib import Path
import sys
import unittest
from typing import get_type_hints
from unittest.mock import patch

from symbolica import MathMLMarker, MathMLPage, MathMLPageSource, MathMLViewer

spec = importlib.util.spec_from_file_location('mathml_custom_viewer', Path(__file__).parents[1] / 'examples/mathml_custom_viewer.py')
example = importlib.util.module_from_spec(spec)
spec.loader.exec_module(example)


class ViewerAPITests(unittest.TestCase):
    def test_marker_and_page_construction(self):
        marker = MathMLMarker([7], 'Inspect tensor [1]', 12)
        self.assertEqual(type(marker).__name__, 'MathMLMarker')
        page = MathMLPage('<math/>', 0, 1, 1, markers=[marker])
        self.assertEqual(page.markers, [marker])

    def test_public_types_and_lazy_provider(self):
        calls = []
        source = example.TensorComponentSource(10_000, lambda i, j: calls.append((i, j)) or i + j)
        viewer = MathMLViewer(source, page_size=10)
        self.addCleanup(viewer.close)
        self.assertEqual(calls, [])
        parent = viewer._page()
        self.assertEqual(parent['layouts'], [])
        self.assertFalse(parent['supports_expansion'])
        self.assertEqual(calls, [])  # Only the visible tensor head is requested.
        widget = viewer._get_widget()
        viewer._on_message(widget, {'action': 'preview', 'value': [0], 'popup': True,
                                   'start': 0, 'size': 10, 'request': 'a'}, [])
        self.assertEqual(calls, [(0, j) for j in range(10)])
        self.assertEqual(widget.preview['total'], 100_000_000)
        self.assertFalse(widget.preview['can_expand'])
        self.assertEqual(viewer._page(), parent)
        viewer._action({'action': 'open', 'value': [0]})
        self.assertEqual(viewer._action({'action': 'next'})['start'], 10)
        self.assertEqual(viewer._action({'action': 'previous'})['start'], 0)
        self.assertEqual(viewer._action({'action': 'back'}), parent)
        with self.assertRaisesRegex(ValueError, 'does not support'):
            viewer._action({'action': 'expand', 'value': [0]})
        self.assertIn('html', get_type_hints(MathMLPage))
        self.assertIn('path', get_type_hints(MathMLMarker))

    def test_static_fallback_and_close(self):
        viewer = example.example_viewer()
        with patch.dict(sys.modules, {'anywidget': None}):
            bundle = viewer._repr_mimebundle_()
        self.assertIn('<mi>T</mi>', bundle['text/html'])
        self.assertNotIn('application/vnd.jupyter.widget-view+json', bundle)
        viewer.close()
        with self.assertRaisesRegex(RuntimeError, 'closed'):
            viewer._repr_html_()

    def test_provider_errors_are_reported_in_popup(self):
        def failing_component(i, j):
            raise TimeoutError("component compiler timed out")
        viewer = MathMLViewer(example.TensorComponentSource(100, failing_component))
        self.addCleanup(viewer.close)
        widget = viewer._get_widget()
        parent = viewer._page()
        viewer._on_message(widget, {'action': 'preview', 'value': [0], 'request': 'failed'}, [])
        self.assertEqual(widget.preview['request'], 'failed')
        self.assertIn('component compiler timed out', widget.preview['error'])
        self.assertEqual(viewer._page(), parent)
        viewer._on_message(widget, {'action': 'open', 'value': [0]}, [])
        self.assertIn('component compiler timed out', widget.page['error'])
        self.assertEqual(viewer._path, [])
        self.assertEqual(viewer._parents, [])
        self.assertEqual(viewer._page(), parent)

    def test_transfer_contract_and_limits(self):
        class Source:
            def page(self, *args):
                return self.value
        source = Source()
        source.value = dict(html='<math><mi>x</mi></math>', start=0, end=1, total=1,
                            private_native_tree=object())
        viewer = MathMLViewer(source)
        self.addCleanup(viewer.close)
        self.assertNotIn('private_native_tree', viewer._page())
        for value in (MathMLPage('x' * (256 * 1024 + 1), 0, 1, 1),
                      MathMLPage('<math/>', 0, 0, 1),
                      MathMLPage('<math/>', 0, 26, 26),
                      MathMLPage('<math/>', 0, 1, 1, markers=[MathMLMarker([-1], 'bad')])):
            source.value = value
            with self.assertRaises(ValueError):
                MathMLViewer(source)._page()
        for kwargs in ({'page_size': 0}, {'page_size': True}, {'layout_count': 9}):
            with self.assertRaises(ValueError):
                MathMLViewer(source, **kwargs)
        with self.assertRaises(TypeError):
            MathMLViewer(object())
        self.assertEqual(MathMLMarker.marker(0), '<mtext data-symbolica-marker="0">[1]</mtext>')

    def test_nested_popup_ancestors_survive_many_page_requests(self):
        class Source:
            def page(self, path, start, size, layout=0, expansions=()):
                if len(path) < 3:
                    marker = MathMLMarker([*path, 0], 'Inspect child [1]')
                    return MathMLPage(f'<math>{marker.marker(0)}</math>', 0, 1, 1,
                                      markers=[marker])
                return MathMLPage(f'<math><mn>{start}</mn></math>', start,
                                  min(start + size, 1000), 1000)
        viewer = MathMLViewer(Source())
        self.addCleanup(viewer.close)
        widget = viewer._get_widget()
        viewer._on_message(widget, {'action': 'preview', 'value': [0], 'popup': True}, [])
        outer = widget.preview['context']
        viewer._on_message(widget, {'action': 'preview', 'value': [0, 0],
                                   'context': outer, 'popup': True}, [])
        inner = widget.preview['context']
        for start in range(40):
            viewer._on_message(widget, {'action': 'preview', 'value': [0, 0, 0],
                                       'context': inner, 'popup': True, 'start': start}, [])
            self.assertNotIn('error', widget.preview)
        self.assertIn(outer, viewer._preview_pages)
        self.assertIn(inner, viewer._preview_pages)
        self.assertLessEqual(len(viewer._preview_pages), 32)


if __name__ == '__main__':
    unittest.main()
