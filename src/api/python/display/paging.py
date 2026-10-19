"""Notebook adapters only; paging and validation live in Rust."""
try:
    from typing import Protocol
except ImportError:  # Python 3.7 remains supported by the abi3 extension.
    Protocol = object


class MathMLPageSource(Protocol):
    """A provider implementing page(path, start, size, layout, expansions)."""
    def page(self, path, start, size, layout=0, expansions=()):
        ...


class MathMLViewer(_MathMLViewer):
    """Rust-backed viewer with lazy Jupyter and marimo adapters."""
    def __init__(self, source, page_size=25, *, layout_count=1, supports_expansion=False):
        self._widget = None

    def _on_message(self, widget, content, buffers):
        response = self._message(content)
        if response is not None:
            trait, value = response
            if trait:
                setattr(widget, trait, value)
            else:
                self._close_widget()

    def _close_widget(self):
        if self._widget is not None:
            widget, self._widget = self._widget, None
            widget.on_msg(self._on_message, remove=True)
            widget.close()

    def _get_widget(self):
        if self._widget is None:
            import anywidget
            import traitlets

            class MathPages(anywidget.AnyWidget):
                _esm = WIDGET_ESM
                page = traitlets.Dict().tag(sync=True)
                preview = traitlets.Dict().tag(sync=True)

            self._widget = MathPages(page=self._page())
            self._widget.on_msg(self._on_message)
        return self._widget

    def close(self):
        self._close_widget()
        self._close()

    def _repr_mimebundle_(self, include=None, exclude=None):
        def wanted(mime):
            return (include is None or mime in include) and (exclude is None or mime not in exclude)
        result = {}
        if wanted("text/plain"):
            result["text/plain"] = repr(self)
        if wanted("text/html"):
            result["text/html"] = self._repr_html_()
        if wanted("application/vnd.jupyter.widget-view+json"):
            try:
                bundle = self._get_widget()._repr_mimebundle_(include=include, exclude=exclude)
                if isinstance(bundle, tuple):
                    bundle = bundle[0]
                result.update({k: v for k, v in (bundle or {}).items() if wanted(k)})
                if wanted("text/plain"):
                    result["text/plain"] = repr(self)
            except ImportError:
                pass
        return result

    def _display_(self):
        import marimo as mo
        try:
            return mo.ui.anywidget(self._get_widget())
        except ImportError:
            return mo.Html(self._repr_html_())
