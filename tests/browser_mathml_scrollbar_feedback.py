"""A cell scrollbar must not make responsive layouts alternate forever."""
import os
from pathlib import Path

from playwright.sync_api import sync_playwright
from symbolica import MathMLPage, MathMLViewer


class SizedPages:
    """Fixed-size MathML makes the scrollbar threshold independent of fonts."""
    def __init__(self):
        self.calls = []

    def page(self, path, start, size, layout=0, expansions=()):
        self.calls.append((path, start, size, layout))
        width, height = (758, 820) if layout == 0 else (500, 100)
        return MathMLPage(f'<math><mspace width="{width}px" height="{height}px"/></math>',
                          0, 1, 1)


source = SizedPages()
viewer = MathMLViewer(source, layout_count=2)
script_path = os.environ.get('MATHML_VIEWER_SCRIPT',
    str(Path(__file__).parents[1] / 'src/api/python/display/paging.js'))
script = Path(script_path).read_text()
# Observe completed fit passes and the dimensions entering each pass. Stop a
# broken implementation after 20 passes so the regression cannot hang the test.
script = script.replace('  function update() {', '''  function update() {
    if (++window.fitCount > 20) return;
''').replace('available = math.clientWidth;', '''available = math.clientWidth;
    window.fitInputs.push({width:available,height:content.scrollHeight});''')
try:
    with sync_playwright() as p:
        browser = p.chromium.launch(executable_path=os.environ.get('CHROMIUM_EXECUTABLE'),
                                   headless=True, args=['--no-sandbox'])
        page = browser.new_page(viewport={'width': 1200, 'height': 1000})
        page.route('http://localhost/**', lambda route: route.fulfill(
            body='<div id="card" style="box-sizing:border-box;width:770px;max-height:800px;overflow:auto">'
                 '<div id="viewer"></div></div>', content_type='text/html'))
        page.goto('http://localhost/')
        page.evaluate('value=>{window.state=value;window.listeners=new Map();window.fitCount=0;window.fitInputs=[];window.sent=0}',
                      {'page': viewer._page(), 'preview': {}})
        page.add_script_tag(type='module', content=script.replace('export default { render };', '''
            window.cleanup=render({el:document.querySelector('#viewer'),model:{
              get:key=>state[key],on:(key,fn)=>listeners.set(key,fn),off:key=>listeners.delete(key),
              send:message=>{if(!['attach','dispose'].includes(message.action))window.sent++;}
            }});
            const card=document.querySelector('#card');
            const shadow=document.querySelector('#viewer').shadowRoot;
            const math=shadow.querySelector('.math'),content=shadow.querySelector('.math-content');
            // Headless Chromium uses overlay scrollbars. Model the 15px gutter
            // of a classic scrollbar when a tall candidate overflows the cell.
            // Apply it on geometry reads and after restoring a shorter layout.
            const clientWidth=Object.getOwnPropertyDescriptor(Element.prototype,'clientWidth').get;
            const rect=content.getBoundingClientRect.bind(content);
            function scrollbar(){card.style.paddingRight=content.scrollHeight>800?'15px':'0px';}
            Object.defineProperty(math,'clientWidth',{get(){scrollbar();return clientWidth.call(math);}});
            content.getBoundingClientRect=()=>{scrollbar();return rect();};
            window.gutterObserver=new MutationObserver(scrollbar);
            gutterObserver.observe(content,{childList:true,subtree:true,characterData:true});
            scrollbar();
        '''))
        page.wait_for_timeout(600)
        assert page.evaluate('fitCount <= 4'), page.evaluate('fitInputs')
        main = page.locator('.math-content')
        assert main.get_attribute('data-layout') == '1'
        settled = page.evaluate('fitCount')
        page.wait_for_timeout(150)
        assert page.evaluate('fitCount') == settled
        # An actual resize must still recover detail, and shrinking back must
        # settle again instead of permanently locking the compact alternative.
        page.locator('#card').evaluate('el=>el.style.width="900px"')
        page.wait_for_function('document.querySelector("#viewer").shadowRoot.querySelector(".math-content").dataset.layout==="0"')
        page.wait_for_timeout(150)
        page.locator('#card').evaluate('el=>el.style.width="770px"')
        page.wait_for_function('document.querySelector("#viewer").shadowRoot.querySelector(".math-content").dataset.layout==="1"')
        page.wait_for_timeout(150)
        settled = page.evaluate('fitCount')
        page.wait_for_timeout(150)
        assert page.evaluate('fitCount') == settled
        assert page.get_by_role('region', name='Expression', exact=True).evaluate(
            'el=>el.scrollWidth<=el.clientWidth+1')
        assert page.evaluate('window.sent') == 0
        assert len(source.calls) == 2
        page.evaluate('gutterObserver.disconnect();cleanup()')
        browser.close()
finally:
    viewer.close()
print('Scrollbar feedback settles, resizing restores detail, and fitting stays local.')
