"""Exercise the shared frontend with a third-party provider, without Expression."""
import os
from pathlib import Path
from playwright.sync_api import sync_playwright
from mathml_viewer_api import example
from symbolica import MathMLViewer

calls = []
viewer = MathMLViewer(example.TensorComponentSource(10_000, lambda i, j: calls.append((i, j)) or i + j), page_size=10)
widget = viewer._get_widget()
try:
    with sync_playwright() as p:
        browser = p.chromium.launch(executable_path=os.environ.get('CHROMIUM_EXECUTABLE'), headless=True, args=['--no-sandbox'])
        page = browser.new_page(viewport={'width':900, 'height':800})
        errors = []
        page.on('pageerror', lambda error: errors.append(str(error)))
        page.route('http://localhost/**', lambda route: route.fulfill(body='<div id="viewer"></div>', content_type='text/html'))
        page.goto('http://localhost/')
        def send(message):
            viewer._on_message(widget, message, [])
            return {'page': widget.page, 'preview': widget.preview}
        page.expose_function('kernelSend', send)
        page.evaluate('value => { window.state=value; window.listeners=new Map(); }', {'page':widget.page, 'preview':widget.preview})
        script = (Path(__file__).parents[1] / 'src/api/python/display/paging.js').read_text()
        page.add_script_tag(type='module', content=script.replace('export default { render };', '''
        window.cleanup = render({el:document.querySelector('#viewer'),model:{
          get:key=>state[key], on:(key,fn)=>listeners.set(key,fn), off:key=>listeners.delete(key),
          send:async message=>{window.state=await kernelSend(message);
            listeners.get(message.action==='preview'?'change:preview':'change:page')?.();}
        }});
        '''))
        marker = page.get_by_role('button', name='Open components [1]', exact=True)
        marker.wait_for()
        assert calls == []
        marker.click()
        popup = page.get_by_role('dialog')
        popup.get_by_text('Tensor · components 1–10 of 100000000', exact=True).wait_for()
        assert popup.get_by_role('button', name='Show more here').is_hidden()
        popup.get_by_role('button', name='Next', exact=True).click()
        popup.get_by_text('Tensor · components 11–20 of 100000000', exact=True).wait_for()
        popup.get_by_label('Items per page').select_option('25')
        popup.get_by_text('Tensor · components 1–25 of 100000000', exact=True).wait_for()
        popup.get_by_role('button', name='Inspect', exact=True).click()
        page.get_by_role('status').filter(has_text='Items 1–25 of 100000000').wait_for()
        page.get_by_role('button', name='Back to parent', exact=True).click()
        marker.wait_for()
        assert len(calls) < 200
        assert not errors, errors
        page.evaluate('window.cleanup()')
        browser.close()
finally:
    viewer.close()
print('Public MathML provider browser integration passed')
