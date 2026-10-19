"""Verify multiline product popups and expansion against the native renderer."""
import os
from pathlib import Path

from playwright.sync_api import sync_playwright
from symbolica import E
from mathml_display import formatted_pager

sums = ['+'.join(f'{j+1}*q{i}^{j}' for j in range(24)) for i in range(1, 33)]
pager = formatted_pager(E('Factorized(' + '*'.join(f'({s})' for s in sums) + ')'))
widget = pager._get_widget()
source = (Path(__file__).parents[1] / 'src/api/python/display/paging.js').read_text()
try:
    with sync_playwright() as p:
        browser = p.chromium.launch(executable_path=os.environ.get('CHROMIUM_EXECUTABLE'),
                                   headless=True, args=['--no-sandbox'])
        page = browser.new_page(viewport={'width': 1200, 'height': 1100})
        errors = []
        page.on('pageerror', lambda error: errors.append(str(error)))
        page.route('http://localhost/**', lambda route: route.fulfill(
            body='<div id="viewer" style="width:982px"></div>', content_type='text/html'))
        page.goto('http://localhost/')

        def send(message):
            pager._on_message(widget, message, [])
            return {'page': widget.page, 'preview': widget.preview}

        page.expose_function('kernelSend', send)
        page.evaluate('value=>{window.state=value;window.listeners=new Map();}',
                      {'page': widget.page, 'preview': widget.preview})
        page.add_script_tag(type='module', content=source.replace('export default { render };', '''
            window.cleanup = render({el:document.querySelector('#viewer'), model:{
              get:key=>state[key], on:(key,fn)=>listeners.set(key,fn), off:key=>listeners.delete(key),
              send:async message=>{
                window.state=await kernelSend(message);
                listeners.get(message.action==='preview'?'change:preview':'change:page')?.();
              }
            }});
        '''))
        region = page.get_by_role('region', name='Expression', exact=True)
        marker = region.get_by_role('button', name='Open product [1]', exact=True)
        marker.wait_for()
        page.evaluate('document.fonts.ready')
        marker.click()
        popup = page.get_by_role('dialog')
        popup.locator('math').wait_for()
        popup.get_by_label('Items per page').select_option('50')
        page.wait_for_function("state.preview.request && state.preview.end > 1")
        product = popup.locator('[data-symbolica-product]').first
        assert product.locator('[data-symbolica-factor]').count() > 1
        assert len(set(round(y) for y in product.locator('[data-symbolica-factor]').evaluate_all(
            'els=>els.map(el=>el.getBoundingClientRect().top)'))) > 1
        assert product.evaluate("el=>[...el.querySelectorAll('[data-symbolica-factor]')].slice(1).every(f=>f.firstElementChild?.textContent==='·')")
        assert popup.locator('.preview-math').evaluate('(el)=>el.scrollWidth<=el.clientWidth+1')
        # A narrow popup folds the large sums and keeps their markers live.
        popup.evaluate('el=>el.style.width="420px"')
        popup.get_by_label('Items per page').select_option('5')
        popup.get_by_text('Product · factors 1–5 of 32', exact=True).wait_for()
        nested = popup.get_by_role('button', name='Open sum [1]', exact=True)
        nested.wait_for()
        assert popup.locator('.preview-math').evaluate('(el)=>el.scrollWidth<=el.clientWidth+1')
        assert popup.locator('.preview-math').evaluate('''el=>{
            const glyphs=[...el.querySelectorAll('mi,mn')].map(g=>g.getBoundingClientRect());
            return [...el.querySelectorAll('button.omission')].every(button=>{
                const box=button.getBoundingClientRect();
                return glyphs.every(g=>Math.min(g.right,box.right)-Math.max(g.left,box.left)<=1
                  || Math.min(g.bottom,box.bottom)-Math.max(g.top,box.top)<=1);
            });
        }''')
        nested.click()
        popup.get_by_text('Sum · terms 1–5 of 24', exact=True).wait_for()
        popup.get_by_role('button', name='Next', exact=True).click()
        popup.get_by_text('Sum · terms 6–10 of 24', exact=True).wait_for()
        popup.get_by_role('button', name='Back', exact=True).click()
        popup.get_by_text('Product · factors 1–5 of 32', exact=True).wait_for()
        nested.click()
        popup.get_by_text('Sum · terms 1–5 of 24', exact=True).wait_for()
        popup.get_by_role('button', name='Inspect', exact=True).click()
        page.get_by_role('status').filter(has_text='Items 1–5 of 24').wait_for()
        page.get_by_role('button', name='Back to parent', exact=True).click()
        region.get_by_role('button', name='Open product [1]', exact=True).click()
        popup.get_by_text('Product · factors 1–5 of 32', exact=True).wait_for()
        popup.get_by_label('Items per page').select_option('50')
        popup.get_by_role('button', name='Show more here', exact=True).click()
        product = region.locator('[data-symbolica-product]').first
        product.wait_for()
        assert product.locator('[data-symbolica-factor]').count() == 32
        assert region.get_by_role('button', name='Open product [1]', exact=True).count() == 0
        assert region.locator('button.omission').count() == 32  # sums remain individually explorable
        assert region.evaluate('(el)=>el.scrollWidth<=el.clientWidth+1')
        assert region.evaluate('(el)=>el.scrollHeight===el.clientHeight')
        # Native MathML rows must stay inside the function's line box; flex
        # layout can otherwise put the first row halfway down a tall fence.
        assert product.evaluate('''el=>{
            const box=el.getBoundingClientRect();
            const factors=el.querySelectorAll('[data-symbolica-factor]');
            return factors[0].getBoundingClientRect().top < box.top+30
              && factors[factors.length-1].getBoundingClientRect().bottom <= box.bottom+1;
        }''')
        assert len(set(round(y) for y in product.locator('[data-symbolica-factor]').evaluate_all(
            'els=>els.map(el=>el.getBoundingClientRect().top)'))) > 1
        assert product.evaluate("el=>[...el.querySelectorAll('[data-symbolica-factor]')].slice(1).every(f=>f.firstElementChild?.textContent==='·')")
        assert pager._path == [] and pager._parents == []
        assert not errors, errors
        page.evaluate('cleanup()')
        browser.close()
finally:
    pager.close()
print('Product popups wrap, continuation lines have dots, and inline expansion reveals all factors.')
