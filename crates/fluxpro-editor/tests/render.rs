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

#[test]
fn monitor_accepts_a_host_selected_instance_and_action_slot() {
    use fluxpro_editor::{MonitorInstance, MonitorScope, MonitorSnapshot, ProcessMonitor};
    Owner::new().with(|| {
        let document=EditorDocument::default();
        let scope=MonitorScope::from_document(&document);
        let selected=MonitorInstance { uuid:"selected-instance".into(), process_id:"ORDER-SELECTED".into(), process_key:scope.key, process_version:scope.version, ..Default::default() };
        let html=view! {<ProcessMonitor document snapshot=MonitorSnapshot::default() on_request=Callback::new(|_|{}) open_instance=Some(selected) instance_actions=Callback::new(|_|view!{<button>"Host signal action"</button>}.into_any())/>}.to_html();
        assert!(html.contains("Close instance ORDER-SELECTED"));
        // The Events section mounts host actions lazily.
        assert!(!html.contains("Host signal action"));
        assert!(html.contains("Instance detail sections"));
    });
}
