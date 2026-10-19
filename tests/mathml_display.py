"""Run against the built extension; widget tests additionally use symbolica[notebook]."""
import inspect
import json
import re
import sys
import unittest
import xml.etree.ElementTree as ET
from unittest.mock import patch

from symbolica import E, S, PrintMode

MATH = '{http://www.w3.org/1998/Math/MathML}'


def formatted_pager(expr, **kwargs):
    """Capture the internal pager created through the public formatting API."""
    if '_symbolica_paging' not in sys.modules:
        E('pager_init_x+pager_init_y').formatted(page_size=1)
    module = sys.modules['_symbolica_paging']
    constructor = module.MathMLViewer
    created = []

    def capture(*args, **kwargs):
        pager = constructor(*args, **kwargs)
        created.append(pager)
        return pager

    with patch.object(module, 'MathMLViewer', side_effect=capture):
        expr.formatted(**kwargs)._repr_mimebundle_(include=['text/html'])
    assert len(created) == 1, 'Expected formatted() to create a pager'
    created[0]._cache.clear()
    return created[0]


class MathMLDisplayTests(unittest.TestCase):
    def assertMath(self, html):
        roots = [ET.fromstring(m) for m in re.findall(r'<math\b.*?</math>', html, re.S)]
        self.assertTrue(roots)
        self.assertLess(len(html.encode()), 256 * 1024)
        self.assertLess(sum(sum(1 for _ in r.iter()) for r in roots), 10000)
        return roots

    def pager(self, expr, **kwargs):
        pager = formatted_pager(expr, **kwargs)
        self.addCleanup(pager.close)
        return pager

    def test_default_math_and_legacy_output(self):
        expr = E('x^2 + 2/3 + sqrt(y) + 1/z')
        self.assertFalse(hasattr(expr, 'paged'))
        html = expr._repr_html_()
        root, = self.assertMath(html)
        for tag in ('msup', 'mfrac', 'msqrt'):
            self.assertIsNotNone(root.find('.//' + MATH + tag))
        self.assertNotIn('$$', html)
        self.assertMath(expr.formatted()._repr_html_())
        old = expr.formatted(mathml=False)
        self.assertNotIn('<math', old._repr_html_())
        self.assertIn('x²', old._repr_html_())
        self.assertIn('$$', old._repr_latex_())
        self.assertNotIn('<math', expr.formatted(mode=PrintMode.Sympy)._repr_html_())
        self.assertEqual(str(expr.formatted()), str(old))
        self.assertNotIn('<math', expr.format())

    def test_static_representations_do_not_construct_the_notebook_viewer(self):
        module = sys.modules['_symbolica_paging']
        constructor = module.MathMLViewer
        with patch.object(module, 'MathMLViewer', wraps=constructor) as create:
            output = E('x+y').formatted()
            self.assertEqual(create.call_count, 0)
            self.assertMath(output._repr_html_())
            self.assertIn('x', str(output))
            self.assertIsNone(output._repr_latex_())
            self.assertEqual(create.call_count, 0)
            bundle = output._repr_mimebundle_(include=['text/html', 'text/plain'])
            self.assertEqual(create.call_count, 1)
            self.assertEqual(bundle['text/plain'], str(output))
            self.assertMath(bundle['text/html'])

    def test_formatting_signatures_expose_their_keyword_options(self):
        expr = E('x')
        for name in ('format', 'formatted'):
            with self.subTest(method=name):
                method = getattr(expr, name)
                self.assertTrue(method.__text_signature__.isascii())
                signature = inspect.signature(method)
                for parameter in ('max_terms', 'mode', 'show_namespaces', 'hide_namespace',
                                  'precision', 'custom_print_mode', 'multiplication_operator'):
                    self.assertIn(parameter, signature.parameters)
                self.assertNotIn('self', signature.parameters)
                self.assertEqual(signature.parameters['max_terms'].default, 100)
                self.assertEqual(signature.parameters['multiplication_operator'].default, '·')
                self.assertEqual(signature.parameters['function_brackets'].default, ('(', ')'))
        signature = inspect.signature(expr.formatted)
        self.assertEqual(signature.parameters['page_size'].default, 25)
        self.assertIs(signature.parameters['mathml'].default, True)
        self.assertEqual(signature.parameters['mathml'].kind, inspect.Parameter.KEYWORD_ONLY)
        signature.bind(show_namespaces=True, mathml=False, page_size=100)

    def test_notebook_completion_lists_formatting_options(self):
        try:
            import jedi
        except ImportError:
            self.skipTest('Jedi is an optional notebook dependency')
        expr = E('x')
        interpreter = jedi.Interpreter('expr.formatted(', [{'expr': expr}])
        signature, = interpreter.get_signatures()
        names = {parameter.name for parameter in signature.params}
        self.assertTrue({'show_namespaces', 'mathml', 'page_size', 'precision'} <= names)
        interpreter = jedi.Interpreter('expr.formatted(show_', [{'expr': expr}])
        self.assertIn('show_namespaces=', {completion.name for completion in interpreter.complete()})

    def test_namespaces_escaping_and_negative_terms(self):
        expr = E('mathml_test::x^2-2*mathml_test::y-1/3')
        html = expr.formatted(show_namespaces=True)._repr_html_()
        self.assertIn('mathml_test::', html)
        self.assertNotIn('mathml_test::', expr._repr_html_())
        self.assertNotIn('<mo>+</mo><mrow><mo>−</mo>', html)
        self.assertMath(html)
        value = S('mathml_test::x<tag>')
        html = value._repr_html_()
        self.assertIn('&lt;tag&gt;', html)
        self.assertMath(html)

    def test_complex_coefficients_are_grouped_and_have_no_ansi(self):
        for value in ('(1+2i)*x', '(-1+2i)*x', '(1/2+3i/5)*x', '1i'):
            html = E(value)._repr_html_()
            self.assertNotIn('\x1b', html)
            self.assertMath(html)
            if 'x' in value:
                self.assertIn('<mo>(</mo>', html)

    def test_derivatives_pair_orders_with_arguments(self):
        for source in ('der(0,1,f,x,y)', 'der(2,f,x)', 'g(der(1,2,f,x,y))'):
            root, = self.assertMath(E(source)._repr_html_())
            head = root.find('.//' + MATH + 'msup')
            self.assertEqual(head[0].text, 'f')
            self.assertNotIn('der', ''.join(root.itertext()))
        big = '+'.join(f'x{i}' for i in range(151))
        pager = self.pager(E(f'der(0,1,f,x,{big})'), page_size=1)
        first = pager._page()
        self.assertEqual((first['start'], first['end'], first['total']), (0, 1, 2))
        second = pager._action({'action': 'next'})
        self.assertEqual(second['layouts'][0]['markers'][0][0], [4])
        child = pager._action({'action': 'open', 'value': [4], 'layout': 1})
        self.assertEqual(child['total'], 151)
        self.assertEqual(pager._action({'action': 'back'})['html'], second['html'])
        # Invalid derivative encodings still display their literal arguments.
        self.assertIn('<mi>der</mi>', E('der(1,f)')._repr_html_())

    def test_negative_powers_use_reciprocals(self):
        for exponent in ('-2', '-3/2', '-9223372036854775809'):
            root, = self.assertMath(E(f'(a+b)^({exponent})')._repr_html_())
            fraction = root.find(MATH + 'mfrac')
            self.assertIsNotNone(fraction)
            self.assertEqual(fraction[0].text, '1')
            self.assertEqual(fraction[1].tag, MATH + 'msup')
            self.assertNotIn('−', ''.join(fraction[1][1].itertext()))
            self.assertIn('(', ''.join(fraction[1][0].itertext()))
        root, = self.assertMath(E('(a+b)^(-1/2)')._repr_html_())
        self.assertIsNotNone(root.find(MATH + 'mfrac/' + MATH + 'msqrt'))
        root, = self.assertMath(E('(a+b)^(-2+i)')._repr_html_())
        self.assertEqual(root[0].tag, MATH + 'msup')

    def test_small_expressions_have_zoom_widget_and_static_fallback(self):
        expr = E('(a+b)^-2')
        pager = self.pager(expr)
        self.assertEqual(pager._repr_html_(), expr._repr_html_())
        self.assertIn('application/vnd.jupyter.widget-view+json', expr.formatted()._repr_mimebundle_())
        with patch.dict(sys.modules, {'anywidget': None}):
            fallback = self.pager(expr)._repr_mimebundle_()
            self.assertEqual(fallback['text/html'], expr._repr_html_())
            self.assertNotIn('application/vnd.jupyter.widget-view+json', fallback)

    def test_product_fractions_are_combined_and_multiplication_is_spaced(self):
        for source in ('x/(a*b)', '2*x*y/(3*(a+b)^2*(1+z))', '-x/(a*b)'):
            root, = self.assertMath(E(source)._repr_html_())
            fractions = root.findall('.//' + MATH + 'mfrac')
            self.assertEqual(len(fractions), 1)
            self.assertIn('x', ''.join(fractions[0][0].itertext()))
            self.assertIn('a', ''.join(fractions[0][1].itertext()))
            self.assertNotIn('·', ''.join(root.itertext()))
        html = E('x-y/(a*b)')._repr_html_()
        self.assertNotIn('<mo>+</mo><mrow><mo>−</mo>', html)
        self.assertNotIn('<mo>+</mo><mrow><mrow><mo>−</mo>', html)
        self.assertIn('<mo>*</mo>', E('x*y').formatted(multiplication_operator='*')._repr_html_())
        self.assertIn('<mo>·</mo>', E('2*3^x')._repr_html_())
        big = '+'.join(f'x{i}' for i in range(151))
        pager = self.pager(E(f'a/(({big})*(1+z)^2)'))
        parent = pager._page()
        self.assertEqual(pager._action({'action': 'open', 'value': parent['layouts'][0]['markers'][0][0], 'layout': 1})['total'], 151)
        self.assertEqual(pager._action({'action': 'back'})['html'], parent['html'])

    def test_pages_are_contiguous_reversible_and_lazy(self):
        expr = E('+'.join(f'{i+1}*paging::x^{i}' for i in range(301)))
        original = repr(expr)
        pager = self.pager(expr)
        pages = []
        while True:
            page = pager._page()
            self.assertMath(page['html'])
            pages.append((page['start'], page['end'], page['html']))
            self.assertLessEqual(len(pager._cache), 3)
            if page['next'] is None:
                break
            pager._action({'action': 'next'})
        self.assertEqual(pages[0][0], 0)
        self.assertEqual(pages[-1][1], 301)
        self.assertTrue(all(a[1] == b[0] for a, b in zip(pages, pages[1:])))
        for start, end, html in reversed(pages[:-1]):
            page = pager._action({'action': 'previous'})
            self.assertEqual((page['start'], page['end'], page['html']), (start, end, html))
        self.assertEqual(repr(expr), original)
        self.assertIn('Expression preview', expr.formatted()._repr_html_())

    def test_page_size_combines_with_formatting_options(self):
        expr = E('+'.join(f'page_options::x{i}' for i in range(101)))
        pager = self.pager(expr, page_size=100, show_namespaces=True)
        page = pager._page()
        self.assertEqual((page['start'], page['end']), (0, 100))
        self.assertIn('page_options::', page['html'])
        page = pager._action({'action': 'next'})
        self.assertEqual((page['start'], page['end']), (100, 101))
        self.assertIn('page_options::', page['html'])

    def test_nested_sums_products_and_arguments(self):
        big = '+'.join(f'x{i}' for i in range(150))
        expr = E(f'f(({big})*(a+b),z)')
        pager = self.pager(expr)
        parent = pager._page()
        self.assertMath(parent['html'])
        self.assertTrue(parent['layouts'][0]['markers'])
        child = pager._action({'action': 'open', 'value': parent['layouts'][0]['markers'][0][0], 'layout': 1})
        self.assertEqual(child['total'], 150)
        self.assertEqual(pager._action({'action': 'next'})['start'], 25)
        pager._action({'action': 'size', 'value': 100})
        self.assertEqual(pager._action({'action': 'back'})['html'], parent['html'])
        self.assertEqual(pager._page()['page_size'], parent['page_size'])
        for expression in ('*'.join(f'x{i}' for i in range(80)),
                           'g(' + ','.join(f'x{i}' for i in range(80)) + ')'):
            page = self.pager(E(expression), page_size=10)._page()
            self.assertEqual((page['start'], page['end'], page['total']), (0, 10, 80))
            self.assertMath(page['html'])

    def test_responsive_layouts_and_lazy_tail_preview(self):
        expr = E('f(' + '+'.join(f'x{i}' for i in range(151)) + ',z)')
        pager = self.pager(expr, page_size=10)
        parent = pager._page()
        # Start with all nested terms; the browser chooses a shorter alternative
        # only when the full expression exceeds its available width.
        self.assertEqual(parent['markers'], [])
        root, = self.assertMath(parent['html'])
        self.assertEqual(len(root.findall('.//' + MATH + 'mi')), 153)
        short = pager._selected_page({'layout': 4})
        self.assertEqual(short['marker_starts'], [4])
        root, = self.assertMath(short['html'])
        self.assertEqual(len(root.findall('.//' + MATH + 'mi')), 6)
        marker = root.find('.//' + MATH + 'mtext')
        self.assertEqual(marker.text, '[1]')
        self.assertIn('<mo form="infix">+</mo><mtext data-symbolica-marker=', short['html'])
        self.assertEqual(len(root.findall('.//' + MATH + 'mtable/' + MATH + 'mtr')), 2)
        widget = pager._get_widget()
        path = short['markers'][0][0]
        pager._on_message(widget, {'action': 'preview', 'value': path,
                                  'layout': 4, 'request': 'hover-1'}, [])
        self.assertEqual((widget.preview['start'], widget.preview['end'], widget.preview['total']), (4, 9, 151))
        self.assertEqual((widget.preview['kind'], widget.preview['unit']), ('Sum', 'terms'))
        self.assertEqual(widget.preview['request'], 'hover-1')
        self.assertMath(widget.preview['html'])
        self.assertEqual(pager._page(), parent)
        self.assertEqual(len(pager._cache), 1)
        for message in ({'action': 'preview', 'value': [999]},
                        {'action': 'preview', 'value': path, 'layout': 999}):
            pager._on_message(widget, message, [])
            self.assertIn('error', widget.preview)
        pager._action({'action': 'size', 'value': 100})
        opened = pager._action({'action': 'open', 'value': path, 'layout': 4})
        self.assertEqual((opened['start'], opened['end']), (0, 100))
        full = pager._action({'action': 'size', 'value': 250})
        self.assertEqual(full['markers'], [])
        self.assertNotIn('data-symbolica-marker', full['html'])
        mixed = self.pager(E('A(' + '+'.join(f'x{i}' for i in range(30)) + ',' +
                             '*'.join(f'y{i}' for i in range(30)) + ')'), page_size=3)
        short = mixed._selected_page({'layout': 4})
        self.assertEqual(short['marker_ids'], [0, 1])
        self.assertIn('<mo form="infix">+</mo><mtext data-symbolica-marker="0">[1]', short['html'])
        self.assertIn('<mo form="infix">·</mo><mtext data-symbolica-marker="1">[2]', short['html'])
        widget = mixed._get_widget()
        for i, kind in enumerate(('Sum', 'Product')):
            mixed._on_message(widget, {'action': 'preview', 'value': short['markers'][i][0],
                                      'layout': 4, 'request': i}, [])
            self.assertEqual(widget.preview['kind'], kind)
            self.assertEqual((widget.preview['start'], widget.preview['end']), (4, 7))

    def test_popup_pagination_is_local_and_root_markers_work(self):
        pager = self.pager(E('+'.join(f'x{i}' for i in range(151))), page_size=10)
        parent = pager._page()
        self.assertIn([], [path for path, _ in parent['markers']])
        widget = pager._get_widget()
        for start, size, end in ((0, 25, 25), (25, 25, 50), (0, 100, 100)):
            pager._on_message(widget, {'action': 'preview', 'value': [], 'popup': True,
                                      'start': start, 'size': size, 'request': str(start)}, [])
            self.assertEqual((widget.preview['start'], widget.preview['end']), (start, end))
            self.assertEqual(pager._page(), parent)
            self.assertEqual(pager._size, 10)
            self.assertEqual(pager._parents, [])
        for invalid in ({'size': 501}, {'start': -1}, {'size': True}, {'start': '0'}):
            pager._on_message(widget, dict({'action': 'preview', 'value': [], 'popup': True}, **invalid), [])
            self.assertIn('error', widget.preview)
        opened = pager._action({'action': 'open', 'value': [], 'size': 100})
        self.assertEqual(opened['end'], 100)
        self.assertTrue(opened['back'])
        self.assertEqual(pager._action({'action': 'next'})['start'], 100)
        self.assertEqual(pager._action({'action': 'back'}), parent)

    def test_nested_popup_markers_are_validated_against_their_source_page(self):
        sums = ['+'.join(f'{j+1}*q{i}^{j}' for j in range(24)) for i in range(1, 33)]
        pager = self.pager(E('Factorized(' + '*'.join(f'({s})' for s in sums) + ')'))
        original = pager._page()
        widget = pager._get_widget()
        pager._on_message(widget, {'action': 'preview', 'value': [0], 'popup': True,
                                  'size': 5, 'request': 'product'}, [])
        popup = widget.preview
        compact = next(p for p in popup['layouts'] if p['layout'] == 4)
        path = next(path for path, label in compact['markers'] if 'sum' in label)
        with self.assertRaisesRegex(ValueError, 'Unknown subexpression'):
            pager._action({'action': 'open', 'value': path})
        pager._on_message(widget, {'action': 'preview', 'value': path, 'popup': True,
                                  'size': 5, 'layout': 4, 'context': popup['context'],
                                  'request': 'sum'}, [])
        self.assertEqual(widget.preview['kind'], 'Sum')
        self.assertEqual(widget.preview['total'], 24)
        self.assertEqual(pager._page(), original)
        opened = pager._action({'action': 'open', 'value': path, 'layout': 4,
                                'context': popup['context']})
        self.assertEqual(opened['kind'], 'Sum')
        self.assertEqual(pager._action({'action': 'back'}), original)
        pager._on_message(widget, {'action': 'preview', 'value': path,
                                  'context': 'invented-context'}, [])
        self.assertIn('error', widget.preview)
        for capability in pager._preview_pages.values():
            self.assertNotIn('html', capability)
            self.assertTrue(all('html' not in p for p in capability['layouts']))

    def test_abbreviated_functions_keep_their_names(self):
        pager = self.pager(E('F(x,y,z)'), page_size=1)
        page = pager._page()
        root, = self.assertMath(page['html'])
        self.assertEqual(''.join(root.itertext()).replace('\u2061', ''), 'F(x,[1])')
        self.assertEqual(page['markers'][0][0], [])
        self.assertEqual(pager._action({'action': 'open', 'value': []})['total'], 3)

        pager = self.pager(E('g(h(mathml_test::F(x+y+z,a+b+c)),z)'), show_namespaces=True)
        short = pager._selected_page({'layout': 7})
        root, = self.assertMath(short['html'])
        text = ''.join(root.itertext()).replace('\u2061', '')
        self.assertIn('mathml_test::F([1])', text)
        self.assertNotIn('function', text)
        path = short['markers'][0][0]
        opened = pager._action({'action': 'open', 'layout': 7, 'value': path})
        self.assertEqual(opened['total'], 2)
        self.assertIn('<mi>mathml_test::F</mi>', opened['html'])

    def test_compact_parts_are_not_replaced_by_wider_markers(self):
        # The smallest layout normally retains just one nested item and two
        # levels. Short tails and subtrees should survive both cutoffs.
        for source in ('outer(g(x,z))', 'outer(g(x,y,z))', 'outer(g(h(x)))',
                       'outer(g(long_function_name(x)))',
                       'outer(g(x^2))', 'outer(g(x+y))', 'outer(g(x*y))'):
            with self.subTest(source=source):
                pager = self.pager(E(source))
                short = pager._render_page(7)
                self.assertEqual(short['markers'], [])
                self.assertMath(short['html'])
                self.assertIn('<mi>x</mi>', short['html'])
        # Keep a cheap sibling visible even when the preceding argument needs
        # abbreviation; don't expand that large argument to inspect its size.
        large = '+'.join(f'x{i}' for i in range(1000))
        short = self.pager(E(f'outer(g({large},z))'))._selected_page({'layout': 7})
        self.assertEqual([hole[0] for hole in short['markers']], [[0, 0]])
        self.assertIn('<mi>z</mi>', short['html'])
        self.assertLess(len(short['html']), 2000)

    def test_collapsed_arithmetic_has_no_leading_operator(self):
        for arithmetic in ('x+y+z', 'x*y*z'):
            pager = self.pager(E(f'g(h({arithmetic}),z)'))
            short = pager._selected_page({'layout': 7})
            root, = self.assertMath(short['html'])
            text = ''.join(root.itertext()).replace('\u2061', '')
            self.assertIn('h([1])', text)
            self.assertNotIn('<mo form="infix">', short['html'])

    def test_show_more_expands_only_the_requested_part(self):
        left = '+'.join(f'x{i}' for i in range(151))
        right = '+'.join(f'y{i}' for i in range(121))
        pager = self.pager(E(f'F({left},{right})'))
        before = pager._selected_page({'layout': 4})
        self.assertEqual(before['marker_starts'], [4, 4])
        path = before['markers'][0][0]
        pager._action({'action': 'expand', 'value': path, 'layout': 4, 'size': 25})
        after = pager._selected_page({'layout': 4})
        self.assertEqual(after['marker_starts'], [29, 4])
        self.assertEqual(pager._path, [])
        self.assertEqual(pager._parents, [])
        self.assertEqual(pager._size, 25)
        # Even the smallest responsive layout keeps the explicitly expanded sum.
        smallest = pager._page()['layouts'][-1]
        self.assertEqual(smallest['marker_starts'][0], 29)
        self.assertEqual(pager._action({'action': 'open', 'value': path, 'layout': 4})['end'], 25)
        pager._action({'action': 'back'})
        self.assertEqual(pager._selected_page({'layout': 4})['html'], after['html'])
        root = self.pager(E(left))
        self.assertEqual(root._action({'action': 'expand', 'value': [], 'size': 25})['end'], 50)

    def test_product_pages_and_inline_expansion_can_wrap(self):
        product = '*'.join(f'(1+x^{i})' for i in range(1, 41))
        pager = self.pager(E(f'F({product},z)'))
        before = pager._selected_page({'layout': 4})
        path = before['markers'][0][0]
        self.assertEqual(before['marker_starts'], [4])
        pager._action({'action': 'expand', 'value': path, 'layout': 4, 'size': 25})
        after = pager._render_page(4)
        self.assertEqual(after['marker_starts'], [29])
        root, = self.assertMath(after['html'])
        product_row = root.find('.//' + MATH + 'mtable[@data-symbolica-product]')
        self.assertIsNotNone(product_row)
        factors = product_row.findall('.//' + MATH + 'mrow[@data-symbolica-factor]')
        self.assertEqual(len(factors), 29)
        self.assertTrue(all(factor[0].tag == MATH + 'mo' and factor[0].text == '·'
                            for factor in factors[1:]))
        # Popup pages wrap large sum factors too, and still stop on a complete
        # factor when their bounded rendering budget is exhausted.
        sums = ['+'.join(f'{j+1}*q{i}^{j}' for j in range(24)) for i in range(1, 33)]
        source = self.pager(E('*'.join(f'({s})' for s in sums)))._source
        page = source.page([], 0, 50, 0)
        root, = self.assertMath(page['html'])
        self.assertIsNotNone(root.find('.//' + MATH + 'mtable[@data-symbolica-product]'))
        self.assertIsNotNone(root.find('.//' + MATH + 'mtable'))
        self.assertGreater(page['end'], 1)
        self.assertLess(page['end'], 32)
        self.assertTrue(all(path == [] for path, _ in page['markers']))
        nested = self.pager(E('Factorized(' + '*'.join(f'({s})' for s in sums) + ')'))
        self.assertEqual([path for path, _ in nested._page()['markers']], [[0]])
        expanded = nested._action({'action': 'expand', 'value': [0], 'size': 50})
        root, = self.assertMath(expanded['html'])
        product_row = root.find('.//' + MATH + 'mtable[@data-symbolica-product]')
        self.assertIsNotNone(product_row)
        self.assertEqual(len(product_row.findall('.//' + MATH + 'mrow[@data-symbolica-factor]')), 32)
        self.assertNotIn([0], [path for path, _ in expanded['markers']])

    def test_large_expressions_send_bounded_pages_on_demand(self):
        class CountingSource:
            def __init__(self, source):
                self.source, self.calls = source, []
            def page(self, *args):
                self.calls.append(args[:4])
                return self.source.page(*args)

        payloads = []
        for count in (1000, 20000):
            expr = E('F(' + '+'.join(f'stream_x{i}' for i in range(count)) + ',z)')
            pager = self.pager(expr)
            source = CountingSource(pager._source)
            pager._source = source
            page = pager._page()
            payloads.append(len(json.dumps(page)))
            self.assertLess(payloads[-1], 256 * 1024)
            self.assertLess(len(str(expr.formatted())), 1024)
            self.assertEqual(len(source.calls), 8)  # bounded alternatives of the current page
            self.assertTrue(all(path == [] and start == 0 for path, start, *_ in source.calls))
            widget = pager._get_widget()
            path = page['markers'][0][0]
            before = len(source.calls)
            pager._on_message(widget, {'action': 'preview', 'value': path, 'popup': True,
                                      'start': 500, 'size': 25, 'request': 'next'}, [])
            self.assertEqual(len(source.calls), before + pager._layout_count)
            self.assertEqual((widget.preview['start'], widget.preview['end']), (500, 525))
            self.assertEqual(pager._page(), page)
            self.assertLessEqual(len(pager._cache), 3)
        self.assertLess(payloads[1], 2 * payloads[0])  # 20× more terms, bounded transfer

    def test_budget_pages_end_between_complete_terms(self):
        pager = self.pager(E('(x+y+z+1)^10').expand())
        start = 0
        pages = []
        while start < 286:
            page = pager._source.page([], start, 250, 0)
            self.assertMath(page['html'])
            self.assertGreater(page['end'], start)
            self.assertEqual(page['start'], start)
            # Only the root continuation may be omitted: no partial powers or
            # factors inside the last displayed polynomial term.
            self.assertTrue(all(path == [] for path, _ in page['markers']))
            self.assertEqual(page['marker_starts'], [page['end']] if page['end'] < 286 else [])
            pages.append(page)
            start = page['end']
        self.assertGreater(len(pages), 1)
        self.assertLess(pages[0]['end'], 250)
        # A single term larger than a whole page still advances, retaining one
        # inspectable subtree instead of a trail of incomplete leaf markers.
        huge = '*'.join(f'f(x{i})' for i in range(600))
        page = self.pager(E(f'F({huge},z)'))._source.page([], 0, 250, 0)
        self.assertEqual(page['end'], 1)
        self.assertEqual([path for path, _ in page['markers']], [[0], []])

    def test_depth_and_leaf_budgets(self):
        expr = E('f(' * 80 + 'x' + ')' * 80)
        pager = self.pager(expr)
        depth = 0
        while (page := pager._page())['markers']:
            self.assertMath(page['html'])
            pager._action({'action': 'open', 'value': page['markers'][0][0]})
            depth += 1
            self.assertLess(depth, 10)
        self.assertIn('<mi>x</mi>', page['html'])
        oversized = S('mathml_test::' + 'a' * 20000)
        self.assertMath(oversized._repr_html_())
        self.assertIn('⋯', oversized._repr_html_())
        self.assertNotIn('a' * 20000, oversized._repr_html_())

    def test_only_outer_sums_wrap(self):
        terms = '+'.join(f'w{i}' for i in range(10))
        self.assertIn('flex-wrap:wrap', E(terms)._repr_html_())
        for value in (f'z*({terms})', f'({terms})^2', f'f({terms})', f'1/({terms})'):
            self.assertNotIn('flex-wrap:wrap', E(value)._repr_html_())

    def test_widget_messages_filters_and_close(self):
        expr = E('+'.join(f'x{i}' for i in range(101)))
        pager = self.pager(expr)
        try:
            widget = pager._get_widget()
        except ImportError:
            self.skipTest('anywidget not installed')
        for action, start in [('next', 25), ('previous', 0)]:
            pager._on_message(widget, {'action': action}, [])
            self.assertEqual(widget.page['start'], start)
        pager._on_message(widget, {'action': 'size', 'value': 100}, [])
        self.assertEqual(widget.page['end'], 100)
        self.assertMath(widget.page['html'])
        bundle = expr._repr_mimebundle_()
        self.assertIn('application/vnd.jupyter.widget-view+json', bundle)
        self.assertEqual(set(expr._repr_mimebundle_(include=['text/html'])), {'text/html'})
        self.assertNotIn('text/html', pager._repr_mimebundle_(exclude=['text/html']))
        self.assertIn('application/vnd.jupyter.widget-view+json', expr.formatted()._repr_mimebundle_())
        pager.close()
        with self.assertRaises(RuntimeError):
            pager._page()
        with patch.dict(sys.modules, {'anywidget': None}):
            fallback = self.pager(expr)._repr_mimebundle_()
            self.assertIn('text/html', fallback)
            self.assertNotIn('application/vnd.jupyter.widget-view+json', fallback)
        for size in (0, 501):
            with self.assertRaises(ValueError):
                expr.formatted(page_size=size)

    def test_ipython_prefers_mathml_and_solution_composition(self):
        from IPython.core.formatters import DisplayFormatter
        expr = E('x^2+y')
        data, _ = DisplayFormatter().format(expr)
        self.assertEqual(data['text/html'], expr._repr_html_())
        self.assertMath(data['text/html'])
        from symbolica import Expression
        x = S('x')
        result = Expression.solve(x*x-2, [x])
        self.assertMath(result._repr_html_())
        for branch in result:
            self.assertIn(branch[x]._repr_html_(), result._repr_html_())


if __name__ == '__main__':
    unittest.main()
