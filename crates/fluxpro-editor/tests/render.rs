#![cfg(feature = "ssr")]

use fluxpro_editor::{EditorDocument, ProcessEditor};
use leptos::prelude::*;

#[test]
fn single_public_component_renders_without_browser_or_application_context() {
    let owner = Owner::new();
    let html =
        owner.with(|| view! { <ProcessEditor document=EditorDocument::default()/> }.to_html());
    for label in [
        "Process studio",
        "Block palette",
        "Process properties",
        "Connect from start",
        "Connect to finish",
        "Process connections",
        "Process sections",
        "Forms",
        "Signals",
        "Escalations",
        "General settings",
        "Identity and version",
        "Description / version notes",
        "Lifecycle handlers",
        "Business stages",
    ] {
        assert!(html.contains(label), "missing {label}");
    }
    assert!(html.contains("fp-node"));
    assert!(html.contains("Zoom in"));
    assert!(html.contains("Zoom out"));
    assert!(!html.contains("MAX_ZOOM"));
    assert!(!html.contains("NaN"));
}
