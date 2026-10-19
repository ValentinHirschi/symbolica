//! Exercise the public API as a downstream Rust/PyO3 crate would.
#![cfg(feature = "python_export")]

use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use symbolica::api::python::{
    MathMLMarker, MathMLPage, MathMLPageRequest, MathMLPageSource, MathMLViewer,
};

struct Components {
    rendered: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}

impl Drop for Components {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

impl MathMLPageSource for Components {
    fn page(&mut self, _py: Python<'_>, request: MathMLPageRequest<'_>) -> PyResult<MathMLPage> {
        assert!(request.layout < 2);
        assert!(request.expansions.is_empty());
        if request.path.is_empty() {
            let mut page = MathMLPage::new(
                format!("<math><mi>T</mi>{}</math>", MathMLMarker::marker(0)),
                0,
                1,
                1,
            );
            // A provider-owned focus handle, rather than an expression-tree path.
            page.markers
                .push(MathMLMarker::new(vec![42], "Inspect components [1]", 0));
            return Ok(page);
        }
        if request.path != [42] {
            return Err(PyValueError::new_err("unknown focus"));
        }
        let end = (request.start + request.size).min(17);
        self.rendered
            .fetch_add(end - request.start, Ordering::SeqCst);
        let mut page = MathMLPage::new(
            format!("<math><mn>{}</mn></math>", request.start),
            request.start,
            end,
            17,
        );
        page.kind = "Tensor".into();
        page.unit = "components".into();
        Ok(page)
    }
}

#[test]
fn native_provider_constructs_python_objects_and_pages_lazily() -> PyResult<()> {
    Python::initialize();
    Python::attach(|py| {
        let marker = MathMLMarker::new(vec![42], "Inspect components [1]", 5).into_python(py)?;
        assert_eq!(marker.getattr(py, "start")?.extract::<usize>(py)?, 5);
        assert_eq!(marker.bind(py).get_type().name()?, "MathMLMarker");
        let mut page = MathMLPage::new("<math/>", 0, 1, 1);
        page.markers
            .push(MathMLMarker::new(vec![42], "Inspect components [1]", 0));
        assert_eq!(
            page.into_python(py)?
                .getattr(py, "markers")?
                .bind(py)
                .len()?,
            1
        );

        let rendered = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicUsize::new(0));
        let viewer = MathMLViewer::new(Components {
            rendered: rendered.clone(),
            dropped: dropped.clone(),
        })
        .page_size(5)
        .layout_count(2)
        .into_python(py)?;
        assert_eq!(rendered.load(Ordering::SeqCst), 0);
        viewer.call_method0(py, "_page")?;
        assert_eq!(rendered.load(Ordering::SeqCst), 0);
        let action = PyDict::new(py);
        action.set_item("action", "open")?;
        action.set_item("value", vec![42])?;
        let first = viewer.call_method1(py, "_action", (&action,))?;
        assert_eq!(first.bind(py).get_item("end")?.extract::<usize>()?, 5);
        assert_eq!(rendered.load(Ordering::SeqCst), 10); // Two bounded layouts.
        action.set_item("action", "next")?;
        let next = viewer.call_method1(py, "_action", (&action,))?;
        assert_eq!(next.bind(py).get_item("start")?.extract::<usize>()?, 5);
        action.set_item("action", "previous")?;
        viewer.call_method1(py, "_action", (&action,))?;
        assert_eq!(rendered.load(Ordering::SeqCst), 20); // Previous was cached.
        viewer.call_method0(py, "close")?;
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert!(viewer.call_method0(py, "_page").is_err());
        Ok(())
    })
}

#[test]
fn native_viewer_options_use_shared_validation() {
    Python::initialize();
    Python::attach(|py| {
        let source = || Components {
            rendered: Arc::default(),
            dropped: Arc::default(),
        };
        assert!(
            MathMLViewer::new(source())
                .page_size(0)
                .into_python(py)
                .is_err()
        );
        assert!(
            MathMLViewer::new(source())
                .layout_count(9)
                .into_python(py)
                .is_err()
        );
    });
}
