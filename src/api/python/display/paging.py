"""Kernel-backed expression viewer. Only the visible page enters widget state."""
from __future__ import annotations

from collections import OrderedDict
from dataclasses import dataclass
from typing import Sequence, get_type_hints
try:
    from typing import Protocol
except ImportError:  # Python 3.7 remains supported by the abi3 extension.
    Protocol = object
import json


WIDGET_ESM = ""  # supplied by the extension


@dataclass(frozen=True)
class MathMLMarker:
    """A numbered omission pointing to an absolute path in the source.

    ``start`` is the first omitted item, used for the hover preview. Its index
    in ``MathMLPage.markers`` must match its marker in the MathML.
    """
    path: Sequence[int]
    label: str
    start: int = 0

    @staticmethod
    def marker(index: int) -> str:
        """MathML for a clickable marker; place operators/fences around it."""
        if type(index) is not int or index < 0:
            raise ValueError("marker index must be a nonnegative integer")
        return f'<mtext data-symbolica-marker="{index}">[{index + 1}]</mtext>'


@dataclass(frozen=True)
class MathMLPage:
    """One bounded page, never the complete hidden expression.

    The half-open range [start, end) describes items in the selected source
    node; total is its item count. HTML is trusted, provider-generated MathML.
    """
    html: str
    start: int
    end: int
    total: int
    kind: str = "Expression"
    unit: str = "items"
    markers: Sequence[MathMLMarker] = ()

class MathMLPageSource(Protocol):
    """Adapter implemented by a Python or PyO3 expression renderer.

    Paths are absolute lists of nonnegative integers; [] is the root. Layout
    zero is the most detailed; higher indices are progressively more compact.
    Expansions are (relative_path, visible_item_count) pairs below ``path``.
    Sources must bound their traversal/rendering and return only the requested
    page. No notebook APIs or JavaScript are needed in the source.
    """
    def page(self, path: list[int], start: int, size: int, layout: int = 0,
             expansions: Sequence[tuple[list[int], int]] = ()) -> MathMLPage:
        ...


class MathMLViewer:
    """Reusable lazy MathML UI for Jupyter, marimo, or static HTML fallback.

    ``source.page(path, start, size, layout, expansions)`` supplies MathMLPage
    objects (native adapters may return the documented equivalent dictionary).
    Optional notebook dependencies load only when the viewer is displayed.
    Set layout_count for responsive alternatives and supports_expansion only
    when the source honors explicit expansion requests. Defaults require just
    one bounded page renderer. Call close() to release the retained source.
    """
    def __init__(self, source: MathMLPageSource, page_size: int = 25, *,
                 layout_count: int = 1, supports_expansion: bool = False):
        if not callable(getattr(source, "page", None)):
            raise TypeError("source must implement page(path, start, size, layout, expansions)")
        if type(page_size) is not int or not 1 <= page_size <= 500:
            raise ValueError("page_size must be between 1 and 500")
        if type(layout_count) is not int or not 1 <= layout_count <= 8:
            raise ValueError("layout_count must be between 1 and 8")
        self._layout_count = layout_count
        self._supports_expansion = bool(supports_expansion)
        self._source = source
        self._size = page_size
        self._path = []
        self._start = 0
        self._previous = []
        self._parents = []
        self._cache = OrderedDict()
        self._preview_pages = OrderedDict()
        self._preview_serial = 0
        self._expanded = {}
        self._widget = None
        self._views = set()

    def _source_page(self, path, start, size, layout=0, expansions=()):
        value = self._source.page(path, start, size, layout, expansions)
        if isinstance(value, MathMLPage):
            value = dict(html=value.html, start=value.start, end=value.end, total=value.total,
                         kind=value.kind, unit=value.unit,
                         markers=[(list(h.path), h.label) for h in value.markers],
                         marker_ids=list(range(len(value.markers))),
                         marker_starts=[h.start for h in value.markers])
        if not isinstance(value, dict):
            raise TypeError("source.page must return MathMLPage or a page dictionary")
        html = value.get("html")
        if not isinstance(html, str) or len(html.encode("utf-8")) > 256 * 1024:
            raise ValueError("page MathML must be a string of at most 256 KiB")
        start, end, total = (value.get(k) for k in ("start", "end", "total"))
        if not all(type(n) is int for n in (start, end, total)) or not (
                (start == end == total == 0) or 0 <= start < end <= total):
            raise ValueError("invalid page range: require 0 <= start < end <= total (or an empty page)")
        if end - start > size:
            raise ValueError("provider exceeded the requested page size")
        markers = value.get("markers", [])
        ids = list(value.get("marker_ids", range(len(markers))))
        starts = list(value.get("marker_starts", [0] * len(markers)))
        if len(markers) > 1000 or len(ids) != len(markers) or len(starts) != len(markers):
            raise ValueError("invalid omission metadata")
        if any(type(i) is not int or i < 0 for i in ids + starts) or len(set(ids)) != len(ids):
            raise ValueError("omission ids and offsets must be nonnegative integers; ids must be unique")
        for target, label in markers:
            if not isinstance(target, (list, tuple)) or any(type(i) is not int or i < 0 for i in target):
                raise ValueError("omission paths must contain nonnegative integers")
            if not isinstance(label, str):
                raise ValueError("omission labels must be strings")
        kind, unit = value.get("kind", "Expression"), value.get("unit", "items")
        if not isinstance(kind, str) or not isinstance(unit, str):
            raise ValueError("page kind and unit must be strings")
        # Whitelist transport fields: provider-owned trees/cursors stay in the
        # kernel, even if a native source includes them in its return dictionary.
        page = dict(html=html, start=start, end=end, total=total, next=end if end < total else None,
                    kind=kind, unit=unit, layout=layout,
                    markers=[(list(p), label) for p, label in markers], marker_ids=ids, marker_starts=starts)
        if len(json.dumps(page).encode("utf-8")) > 512 * 1024:
            raise ValueError("page and omission metadata exceed the 512 KiB transfer budget")
        return page

    def _render_page(self, layout=0):
        prefix = tuple(self._path)
        expansions = [(list(path[len(prefix):]), count) for path, count in self._expanded.items()
                      if len(path) > len(prefix) and path[:len(prefix)] == prefix]
        return self._source_page(self._path, self._start, self._size, layout, expansions)

    def _page(self):
        if self._source is None:
            raise RuntimeError("This expression viewer is closed")
        key = (tuple(self._path), self._start, self._size, tuple(sorted(self._expanded.items())))
        if key not in self._cache:
            page = self._render_page()
            # Each alternative is independently bounded. Resize/font loading can
            # select a layout locally, including in multiple views of one widget.
            layouts = []
            previous_html = page["html"]
            for layout in range(1, self._layout_count):
                candidate = self._render_page(layout)
                if candidate["html"] != previous_html:
                    layouts.append(candidate)
                    previous_html = candidate["html"]
            self._cache[key] = dict(page, layouts=layouts)
        self._cache.move_to_end(key)
        while len(self._cache) > 3:
            self._cache.popitem(last=False)
        return dict(self._cache[key], previous=bool(self._previous),
                    back=bool(self._parents), page_size=self._size,
                    supports_expansion=self._supports_expansion)

    def _selected_page(self, message):
        context = message.get("context")
        if context is None:
            page = self._page()
        else:
            if not isinstance(context, str) or context not in self._preview_pages:
                raise ValueError("Unknown or expired popup")
            page = self._preview_pages[context]
            # Keep the active chain live while paging deep inside a popup.
            current, visited = context, set()
            while current in self._preview_pages and current not in visited:
                visited.add(current)
                self._preview_pages.move_to_end(current)
                current = self._preview_pages[current].get("origin_context")
        layout = message.get("layout", 0)
        if type(layout) is not int:
            raise ValueError("Unknown layout")
        if layout == 0:
            return page
        for candidate in page["layouts"]:
            if candidate["layout"] == layout:
                return dict(page, **candidate)
        raise ValueError("Unknown layout")

    def _action(self, message):
        page = self._selected_page(message)
        action = message.get("action")
        if action == "next" and page["next"] is not None:
            self._previous.append(self._start)
            self._start = page["next"]
        elif action == "previous" and self._previous:
            self._start = self._previous.pop()
        elif action == "size":
            size = message.get("value")
            if type(size) is not int or not 1 <= size <= 500:
                raise ValueError("page_size must be between 1 and 500")
            self._size = size
            self._start = 0
            self._previous = []
        elif action == "expand":
            if not self._supports_expansion:
                raise ValueError("this source does not support in-place expansion")
            path = message.get("value")
            paths = [p for p, _ in page["markers"]]
            if path not in paths:
                raise ValueError("Unknown subexpression")
            size = message.get("size", self._size)
            if type(size) is not int or not 1 <= size <= 500:
                raise ValueError("page_size must be between 1 and 500")
            if path == self._path:
                self._size = min(500, self._size + size)
            else:
                key = tuple(path)
                count = max(page["marker_starts"][paths.index(path)], self._expanded.get(key, 0))
                self._expanded[key] = min(500, count + size)
        elif action == "open":
            path = message.get("value")
            if path not in [p for p, _ in page["markers"]]:
                raise ValueError("Unknown subexpression")
            size = message.get("size", self._size)
            if type(size) is not int or not 1 <= size <= 500:
                raise ValueError("page_size must be between 1 and 500")
            self._parents.append((self._path, self._start, self._previous, self._size))
            self._path, self._start, self._previous, self._size = path, 0, [], size
        elif action == "back" and self._parents:
            self._path, self._start, self._previous, self._size = self._parents.pop()
        return self._page()

    def _on_message(self, widget, content, buffers):
        if content.get("action") == "preview":
            try:
                page = self._selected_page(content)
                path = content.get("value")
                index = [p for p, _ in page["markers"]].index(path)
                start, size = page["marker_starts"][index], min(5, self._size)
                if content.get("popup"):
                    start, size = content.get("start", 0), content.get("size", self._size)
                    if type(start) is not int or not 0 <= start < 2**63:
                        raise ValueError("Invalid preview offset")
                    if type(size) is not int or not 1 <= size <= 500:
                        raise ValueError("page_size must be between 1 and 500")
                preview = self._source_page(path, start, size)
                layouts = []
                previous_html = preview["html"]
                for layout in range(1, self._layout_count):
                    candidate = self._source_page(path, start, size, layout)
                    if candidate["html"] != previous_html:
                        layouts.append(candidate)
                        previous_html = candidate["html"]
                preview["layouts"] = layouts
                shown = self._size if path == self._path else max(
                    page["marker_starts"][index], self._expanded.get(tuple(path), 0))
                preview["can_expand"] = self._supports_expansion and shown < 500 and len(path) - len(self._path) < 64
                self._preview_serial += 1
                preview["context"] = f"preview-{self._preview_serial}"
                # Retain only marker capabilities, never popup HTML or hidden
                # source trees. A nested request must originate in a page that
                # this viewer actually returned, including its source layout.
                def capability(value):
                    return {key: value[key] for key in ("markers", "marker_starts", "layout")}
                self._preview_pages[preview["context"]] = dict(
                    capability(preview), layouts=[capability(p) for p in layouts],
                    origin_context=content.get("context"))
                while len(self._preview_pages) > 32:
                    self._preview_pages.popitem(last=False)
            except Exception as error:
                # Provider/compiler failures belong in the popup, not in an
                # unhandled comm callback that leaves it permanently loading.
                preview = {"error": str(error)[:1024]}
            widget.preview = dict(preview, request=content.get("request"))
            return
        if content.get("action") == "attach":
            self._views.add(content.get("view"))
            widget.page = self._page()
            return
        if content.get("action") == "dispose":
            self._views.discard(content.get("view"))
            if not self._views:
                self._cache.clear()
                self._preview_pages.clear()
                widget.on_msg(self._on_message, remove=True)
                widget.close()
                self._widget = None
            return
        state = (self._path, self._start, list(self._previous), list(self._parents),
                 self._size, dict(self._expanded))
        try:
            widget.page = self._action(content)
        except Exception as error:
            # Failed custom renderers must not strand navigation at a bad node
            # or be called again while reporting their original failure.
            (self._path, self._start, self._previous, self._parents,
             self._size, self._expanded) = state
            widget.page = dict(widget.page, error=str(error)[:1024])

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
        if self._widget is not None:
            widget, self._widget = self._widget, None
            widget.on_msg(self._on_message, remove=True)
            widget.close()
        self._cache.clear()
        self._preview_pages.clear()
        self._expanded.clear()
        self._parents.clear()
        self._previous.clear()
        self._source = None
        self._views.clear()

    def _repr_html_(self):
        page = self._page()
        if page["start"] == 0 and page["next"] is None and not page["markers"]:
            return page["html"]
        return ("<div>" + page["html"] +
                "<p>Expression preview · items {}–{} of {}. "
                "Numbered markers identify omitted subexpressions. Navigation requires a live "
                "notebook with symbolica[notebook].</p></div>".format(
                    page["start"] + 1, page["end"], page["total"]))

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

    def __repr__(self):
        return "MathML viewer (use a live notebook to navigate)"


# Resolve dataclass field annotations before the extension exports these types
# under its public module name (which does not contain our typing imports).
for _class in (MathMLMarker, MathMLPage):
    _class.__annotations__ = get_type_hints(_class)
