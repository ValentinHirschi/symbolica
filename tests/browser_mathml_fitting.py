"""Check local width fitting and navigation across mixed-detail subtrees."""
import os
from pathlib import Path

from playwright.sync_api import sync_playwright
from symbolica import E, S
from mathml_display import formatted_pager

expr = E('exp(sin(x^2)/(1+cos(x+y)^2))*log(1+(x+y)^2)/(1+f(x,g(y,z))^2)')
pager = formatted_pager(expr.derivative(S('x')).derivative(S('x')))
widget = pager._get_widget()
source = (Path(__file__).parents[1] / 'src/api/python/display/paging.js').read_text()
bootstrap = '''
window.cleanup = render({el:document.querySelector('#viewer'), model:{
 get:key=>state[key], on:(key,fn)=>listeners.set(key,fn), off:key=>listeners.delete(key),
 send:async message=>{ if (!['attach','dispose'].includes(message.action)) window.sent++;
   window.state=await kernelSend(message);
   listeners.get(message.action==='preview'?'change:preview':'change:page')?.(); }
}});
'''
try:
    with sync_playwright() as p:
        browser = p.chromium.launch(executable_path=os.environ.get('CHROMIUM_EXECUTABLE'),
                                   headless=True, args=['--no-sandbox'])
        page = browser.new_page(viewport={'width':900, 'height':1000})
        errors = []
        page.on('pageerror', lambda error: errors.append(str(error)))
        page.route('http://localhost/**', lambda route: route.fulfill(
            body='<div id="viewer" style="width:600px"></div>', content_type='text/html'))
        page.goto('http://localhost/')
        def send(message):
            # The harness remounts the same source to compare two renderers.
            if message['action'] not in ('attach', 'dispose'):
                pager._on_message(widget, message, [])
            return {'page':widget.page, 'preview':widget.preview}
        page.expose_function('kernelSend', send)
        page.evaluate('value=>{window.state=value;window.listeners=new Map();window.sent=0;}',
                      {'page':widget.page, 'preview':widget.preview})
        # Measure the previous whole-expression selection with identical input.
        baseline = source.replace('    revealFittingParts(layouts, available);', '')
        page.add_script_tag(type='module', content=baseline.replace('export default { render };', bootstrap))
        page.locator('.math-content').wait_for()
        page.evaluate('document.fonts.ready')
        page.wait_for_timeout(100)
        before = page.locator('.math-content mi').filter(has_text='cos').count()
        page.evaluate('cleanup()')
        page.add_script_tag(type='module', content=source.replace('export default { render };', bootstrap))
        page.wait_for_timeout(100)
        after = page.locator('.math-content mi').filter(has_text='cos').count()
        assert after > before, (before, after)
        region = page.get_by_role('region', name='Expression', exact=True)
        assert region.evaluate('(el)=>el.scrollWidth<=el.clientWidth+1')
        assert page.evaluate('window.sent') == 0
        # Local replacements retain their own source layout for kernel messages.
        layout = page.locator('.math-content').get_attribute('data-layout')
        mixed = page.locator(f'button.omission:not([data-layout="{layout}"])')
        assert mixed.count() > 0
        mixed.first.click()
        popup = page.get_by_role('dialog')
        popup.locator('math').wait_for()
        assert 'Unknown' not in popup.inner_text()
        popup.get_by_role('button', name='Inspect', exact=True).click()
        page.get_by_role('button', name='Back to parent', exact=True).wait_for()
        page.get_by_role('button', name='Back to parent', exact=True).click()
        page.wait_for_timeout(100)
        assert page.locator('.math-content mi').filter(has_text='cos').count() == after
        # Refitting remains local and bounded across viewport changes.
        sent = page.evaluate('window.sent')
        page.locator('#viewer').evaluate('(el)=>el.style.width="420px"')
        page.wait_for_timeout(100)
        assert region.evaluate('(el)=>el.scrollWidth<=el.clientWidth+1')
        assert page.evaluate('window.sent') == sent
        assert page.locator('.math-content *').count() <= 10000
        numbers = page.locator('button.omission').all_text_contents()
        assert numbers == [str(i+1) for i in range(len(numbers))]
        assert not errors, errors
        page.evaluate('cleanup()')
        browser.close()
finally:
    pager.close()
print('Local fitting reveals more cosines and preserves navigation and bounded rendering.')
