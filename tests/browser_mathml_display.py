"""Exercise the widget UI against a native pager using Playwright.

Run with the built extension on PYTHONPATH. Set CHROMIUM_EXECUTABLE to use an
existing browser, and optionally MATHML_SCREENSHOT to save the rendered viewer.
"""
import os
from pathlib import Path

from playwright.sync_api import sync_playwright
from symbolica import E
from mathml_display import formatted_pager


def main():
    large_sum = '+'.join(f'x{i}' for i in range(151))
    expression = E(f'f(a*({large_sum}),z)')
    pager = formatted_pager(expression)
    widget = pager._get_widget()
    source = Path(__file__).parents[1] / 'src/api/python/display/paging.js'
    errors = []
    try:
        with sync_playwright() as p:
            browser = p.chromium.launch(
                executable_path=os.environ.get('CHROMIUM_EXECUTABLE'),
                headless=True, args=['--no-sandbox'],
            )
            page = browser.new_page(viewport={'width': 900, 'height': 600})
            page.on('pageerror', lambda error: errors.append(str(error)))
            page.route('http://localhost/**', lambda route: route.fulfill(
                # Reproduce notebook CSS resets that previously removed all
                # button borders/padding and joined the inspection links.
                body='<html><head><style>button,select{all:unset}</style></head>'
                     '<body><input aria-label="Other input"><div id="small"></div><div style="display:flex">'
                     '<div style="flex:1"><div id="viewer"></div></div>'
                     '<div style="flex:1">Adjacent notebook output</div></div></body></html>',
                content_type='text/html'))
            page.goto('http://localhost/')

            def send(message):
                pager._on_message(widget, message, [])
                return {'page': widget.page, 'preview': widget.preview}

            page.expose_function('kernelSend', send)
            page.evaluate('html => document.querySelector("#small").innerHTML = html',
                          E('(x^2+2/3)/(1+sqrt(y))')._repr_html_())
            page.evaluate('value => { window.state = value; window.listeners = new Map(); window.sent = 0; }',
                          {'page': widget.page, 'preview': widget.preview})
            page.add_script_tag(type='module', content=source.read_text().replace(
                'export default { render };', '''
                window.cleanup = render({
                  el: document.querySelector('#viewer'),
                  model: {
                    get: key => window.state[key],
                    on: (name, callback) => window.listeners.set(name, callback),
                    off: name => window.listeners.delete(name),
                    send: async message => {
                      window.sent++;
                      window.state = await window.kernelSend(message);
                      window.listeners.get(message.action === 'preview' ? 'change:preview' : 'change:page')?.();
                    }
                  }
                });
                '''))
            expression = page.get_by_role('region', name='Expression', exact=True)
            inspect = expression.get_by_role('button', name='Open sum [1]', exact=True)
            inspect.wait_for()
            assert page.locator('#viewer').bounding_box()['width'] <= 450
            assert page.evaluate('document.body.scrollWidth <= window.innerWidth')
            assert page.get_by_role('navigation', name='Subexpressions').count() == 0
            assert page.get_by_role('button', name='Inspect sum [1]', exact=True).count() == 0
            assert page.locator('#viewer math').evaluate(
                '(el) => getComputedStyle(el).overflowY') == 'visible'
            page.evaluate("""async () => {
                await document.fonts.ready;
                await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
            }""")
            messages = page.evaluate('window.sent')
            original_height = page.locator('#viewer math').bounding_box()['height']
            expression = page.get_by_role('region', name='Expression', exact=True)
            def scale():
                return float(page.locator('#viewer .math-content').evaluate('(el) => getComputedStyle(el).zoom'))
            assert page.get_by_role('button', name='Zoom in', exact=True).count() == 0
            page.get_by_role('textbox', name='Other input').fill('+ - 0')
            assert scale() == 1
            expression.press('=')
            assert page.locator('#viewer math').bounding_box()['height'] > original_height * 1.2
            assert page.evaluate('window.sent') == messages
            assert scale() == 1.25
            marker = expression.get_by_role('button', name='Open sum [1]', exact=True)
            assert marker.inner_text() == '1'
            assert marker.evaluate('(el) => el.parentElement.previousElementSibling.textContent') == '+'
            main = page.locator('#viewer .math-content')
            original = main.text_content()
            marker.hover()
            tooltip = page.get_by_role('tooltip')
            layout = int(main.get_attribute('data-layout'))
            selected = pager._selected_page({'layout': layout})
            start = selected['marker_starts'][0]
            tooltip.filter(has_text=f'Sum · terms {start+1}–{start+5} of 151').wait_for()
            assert tooltip.locator('math').count() == 1
            assert main.text_content() == original
            assert pager._path == []
            # Hover content is bounded and previews the omitted tail, not the
            # terms already visible in the parent expression.
            assert widget.preview['start'] == start and widget.preview['end'] == start + 5
            marker.press('Escape')
            assert tooltip.is_hidden()
            marker.click()
            popup = page.get_by_role('dialog')
            popup.wait_for()
            popup.get_by_text('Sum · terms 1–25 of 151', exact=True).wait_for()
            assert main.text_content() == original
            assert pager._path == []
            assert page.get_by_role('navigation', name='Expression pages', exact=True).is_hidden()
            popup.get_by_role('button', name='Next', exact=True).click()
            popup.get_by_text('Sum · terms 26–50 of 151', exact=True).wait_for()
            popup.get_by_role('button', name='Previous', exact=True).click()
            popup.get_by_text('Sum · terms 1–25 of 151', exact=True).wait_for()
            popup.get_by_label('Items per page').select_option('100')
            popup.get_by_text('Sum · terms 1–100 of 151', exact=True).wait_for()
            assert pager._size == 25  # popup size is local to this marker
            assert main.text_content() == original
            # Pinning survives mouse movement and keyboard focus inside controls.
            page.get_by_role('textbox', name='Other input').hover()
            assert popup.is_visible()
            popup.get_by_role('button', name='Inspect', exact=True).click()
            page.get_by_role('status').filter(has_text='Items 1–100 of 151').wait_for()
            assert popup.is_hidden()
            assert scale() == 1.25
            page.get_by_role('button', name='Next', exact=True).click()
            page.get_by_role('status').filter(has_text='Items 101–151 of 151').wait_for()
            page.get_by_role('button', name='Previous', exact=True).click()
            page.get_by_role('status').filter(has_text='Items 1–100 of 151').wait_for()
            assert page.locator('math').count() == 2
            if output := os.environ.get('MATHML_SCREENSHOT'):
                page.screenshot(path=output)
            page.get_by_role('button', name='Back to parent').click()
            expression.get_by_role('button', name='Open sum [1]', exact=True).wait_for()
            assert pager._size == 25
            # Keyboard activation opens the popup, preserving the chosen size.
            expression.get_by_role('button', name='Open sum [1]', exact=True).press('Enter')
            popup.get_by_text('Sum · terms 1–100 of 151', exact=True).wait_for()
            popup.get_by_role('button', name='Close', exact=True).click()
            assert popup.is_hidden()
            assert expression.get_by_role('button', name='Open sum [1]', exact=True).evaluate('(el) => el === el.getRootNode().activeElement')
            # Escape and outside clicks dismiss a pinned popup without navigation.
            expression.get_by_role('button', name='Open sum [1]', exact=True).click()
            popup.wait_for()
            popup.press('Escape')
            assert popup.is_hidden()
            expression.get_by_role('button', name='Open sum [1]', exact=True).click()
            popup.wait_for()
            page.get_by_role('textbox', name='Other input').click()
            assert popup.is_hidden()
            assert pager._path == []
            expression.press('0')
            assert scale() == 1
            expression.press('+')
            assert scale() == 1.25
            expression.press('-')
            assert scale() == 1
            # Modifier shortcuts belong to the browser, not the expression.
            assert not expression.evaluate("""el => {
                const event = new KeyboardEvent('keydown', {key:'+', ctrlKey:true, bubbles:true, cancelable:true});
                el.dispatchEvent(event); return event.defaultPrevented;
            }""")
            assert scale() == 1
            messages = page.evaluate('window.sent')
            for _ in range(25):
                expression.press('+')
            assert scale() == 32
            for _ in range(25):
                expression.press('-')
            assert scale() == .25
            expression.press('0')
            assert scale() == 1
            assert page.evaluate('window.sent') == messages
            page.locator('#viewer .zoom-feedback').wait_for(state='hidden')
            # Expand in place without entering inspection or changing the
            # parent's page size. Explicit expansion survives width fitting.
            before = main.locator('mi').count()
            expression.get_by_role('button', name='Open sum [1]', exact=True).click()
            popup.get_by_role('button', name='Show more here', exact=True).click()
            page.wait_for_function("document.querySelector('#viewer').shadowRoot.querySelectorAll('.math-content mi').length > " + str(before))
            assert pager._path == [] and pager._parents == []
            assert expression.evaluate('(el) => el.scrollWidth <= el.clientWidth + 1')
            assert page.get_by_role('button', name='Back to parent').is_hidden()
            assert popup.is_hidden()
            # A short last page still needs navigation back to earlier terms.
            expression.get_by_role('button', name='Open sum [1]', exact=True).click()
            popup.get_by_text('Sum · terms 1–100 of 151', exact=True).wait_for()
            popup.get_by_role('button', name='Next', exact=True).click()
            popup.get_by_text('Sum · terms 101–151 of 151', exact=True).wait_for()
            assert popup.get_by_role('button', name='Previous', exact=True).is_visible()
            assert popup.get_by_label('Items per page').is_visible()
            # Once all terms fit, both arrows and the size selector disappear.
            popup.get_by_label('Items per page').select_option('250')
            popup.get_by_text('Sum · terms 1–151 of 151', exact=True).wait_for()
            assert popup.get_by_role('button', name='Previous', exact=True).is_hidden()
            assert popup.get_by_role('button', name='Next', exact=True).is_hidden()
            assert popup.get_by_label('Items per page').is_hidden()
            assert popup.get_by_role('button', name='Inspect', exact=True).is_visible()
            assert popup.get_by_role('button', name='Show more here', exact=True).is_visible()
            popup.get_by_role('button', name='Close', exact=True).click()
            # Both inline links and the enclosing function structure fit a narrow
            # notebook column, even though the underlying sums have 151/121 terms.
            left = '+'.join(f'{i+1}*x^{i}' for i in range(151))
            right = '+'.join(f'{i+2}*y^{i}' for i in range(121))
            nested = formatted_pager(E(f'A((a+b)*({left}),B(({right})/(1+z^2),f(x,y,z)))'))
            page.evaluate("value => { window.state.page = value; window.listeners.get('change:page')(); }", nested._page())
            nested.close()
            assert expression.evaluate('(el) => el.scrollWidth <= el.clientWidth + 1')
            bounds = expression.bounding_box()
            for number in (1, 2):
                box = page.get_by_role('button', name=f'Open sum [{number}]', exact=True).bounding_box()
                assert bounds['x'] <= box['x'] and box['x'] + box['width'] <= bounds['x'] + bounds['width'] + 1
            # A moderate expression fits completely in a wide notebook. Only a
            # narrower viewport introduces markers, preserving f(..., z), and
            # widening it reveals all terms again without a kernel round trip.
            responsive = formatted_pager(E('f(a*(' + '+'.join(f'q{i}' for i in range(12)) + '),z)'))
            page.set_viewport_size({'width': 2200, 'height': 600})
            page.evaluate("value => { window.state.page = value; window.listeners.get('change:page')(); }", responsive._page())
            page.wait_for_function("document.querySelector('#viewer').shadowRoot.querySelector('.math-content').dataset.layout === '0'")
            assert page.locator('#viewer button.omission').count() == 0
            full_count = main.locator('mi').count()
            messages = page.evaluate('window.sent')
            page.set_viewport_size({'width': 650, 'height': 600})
            page.locator('#viewer button.omission').first.wait_for()
            assert main.locator('mi').count() < full_count
            assert 'f' in main.text_content() and 'z' in main.text_content()
            assert expression.evaluate('(el) => el.scrollWidth <= el.clientWidth + 1')
            page.set_viewport_size({'width': 2200, 'height': 600})
            page.wait_for_function("document.querySelector('#viewer').shadowRoot.querySelector('.math-content').dataset.layout === '0'")
            assert page.locator('#viewer button.omission').count() == 0
            assert main.locator('mi').count() == full_count
            assert page.evaluate('window.sent') == messages
            responsive.close()
            # Small expressions have neither a zoom toolbar nor a pager border.
            small = formatted_pager(E('x+y'))
            page.evaluate("value => { window.state.page = value; window.listeners.get('change:page')(); }", small._page())
            small.close()
            assert page.get_by_role('navigation', name='Expression pages', exact=True).is_hidden()
            assert expression.evaluate('(el) => getComputedStyle(el).borderTopWidth') == '0px'
            assert page.locator('#viewer button:visible').count() == 0
            expression.press('+')
            assert scale() == 1.25
            # Fraction/derivative ink extends beyond MathML's line box. It must
            # fit vertically even when horizontal scrolling is needed at zoom.
            for source_expression in (
                'der(0,1,f,x,y)/((a+b)^2*(1+x^2))+2*x*y/(3*(1+z)^2)',
                '1/(1+1/(1+x^2))+x^(y^z)',
            ):
                sample = formatted_pager(E(source_expression))
                expression.press('0')
                page.evaluate("value => { window.state.page = value; window.listeners.get('change:page')(); }", sample._page())
                sample.close()
                for steps in (0, 1, 6, 25):
                    expression.press('0')
                    for _ in range(steps):
                        expression.press('+')
                    assert expression.evaluate('(el) => el.scrollHeight === el.clientHeight')
                assert expression.evaluate('(el) => el.scrollWidth > el.clientWidth')
            page.evaluate('window.cleanup()')
            page.wait_for_function('window.listeners.size === 0')
            assert not errors, errors
            browser.close()
    finally:
        pager.close()
    print('MathML browser navigation passed')


if __name__ == '__main__':
    main()
