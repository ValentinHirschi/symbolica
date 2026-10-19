"""Expose enum defaults that CPython cannot parse in extension text signatures."""

from functools import wraps
from inspect import signature


def _with_default_mode(method, default_mode):
    native_signature = signature(method)

    @wraps(method)
    def wrapped(*args, **kwargs):
        return method(*args, **kwargs)

    wrapped.__signature__ = native_signature.replace(parameters=[
        parameter.replace(default=default_mode) if parameter.name == "mode" else parameter
        for parameter in native_signature.parameters.values()
    ])
    wrapped.__text_signature__ = method.__text_signature__.replace(
        "mode=...", f"mode={default_mode!r}"
    )
    return wrapped


def install(expression, default_mode):
    for name in ("format", "formatted"):
        method = getattr(expression, name)
        if not hasattr(method, "__signature__"):
            setattr(expression, name, _with_default_mode(method, default_mode))
