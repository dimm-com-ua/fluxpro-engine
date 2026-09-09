mod canvas;
mod details;
use crate::{EDITOR_CSS, EditorDocument, monitor::*};
use canvas::MonitorCanvas;
use details::InstanceDetails;
use leptos::prelude::*;

#[derive(Clone, Copy)]
struct MonitorSession {
    document: Signal<EditorDocument>,
    request: RwSignal<MonitorRequest>,
    accepted: RwSignal<Option<MonitorSnapshot>>,
    error: RwSignal<Option<String>>,
    active: RwSignal<Option<String>>,
    tabs: RwSignal<Vec<MonitorInstance>>,
    live: RwSignal<bool>,
}
fn next_revision() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl MonitorSession {
    fn change(self, f: impl FnOnce(&mut MonitorRequest)) {
        self.error.set(None);
        self.request.update(|r| {
            f(r);
            r.revision = next_revision();
        });
    }
    fn query(self, f: impl FnOnce(&mut MonitorQuery)) {
        self.change(|r| {
            f(&mut r.query);
            r.query.offset = 0;
        });
    }
    fn open(self, instance: MonitorInstance) {
        if !self.request.with_untracked(|r| r.scope.contains(&instance)) {
            return;
        }
        let id = instance.uuid.clone();
        if !self
            .tabs
            .with_untracked(|tabs| tabs.iter().any(|tab| tab.uuid == id))
        {
            if self.tabs.with_untracked(|tabs| tabs.len() >= 25) {
                self.error.set(Some(
                    "Close an instance tab before opening another (maximum 25).".into(),
                ));
                return;
            }
            self.tabs.update(|tabs| tabs.push(instance));
            self.change(|r| {
                r.instances.push(MonitorInstanceRequest {
                    uuid: id.clone(),
                    log_limit: 50,
                    signal_limit: 50,
                })
            });
        }
        self.active.set(Some(id));
    }
    fn close(self, id: &str) {
        self.tabs.update(|tabs| tabs.retain(|tab| tab.uuid != id));
        self.change(|r| r.instances.retain(|tab| tab.uuid != id));
        if self.active.get_untracked().as_deref() == Some(id) {
            self.active.set(
                self.tabs
                    .with_untracked(|tabs| tabs.last().map(|tab| tab.uuid.clone())),
            );
        }
    }
    fn health_summary(self) -> String {
        self.accepted.with(|snapshot| {
            let Some(snapshot)=snapshot else { return "Waiting for version data".into(); };
            let complete=self.document.with(|doc| doc.definition.nodes.iter().all(|node| snapshot.node_counts.iter().any(|count|count.node_id==node.id().get_id())));
            let total=snapshot.node_counts.iter().map(|n|n.instance_count).sum::<u64>();
            let errors=snapshot.node_counts.iter().map(|n|n.errors).sum::<u64>();
            let escalated=snapshot.node_counts.iter().map(|n|n.escalations).sum::<u64>();
            let escalated = if snapshot.escalation_counts_available { escalated.to_string() } else { "unknown".into() };
            let kind = if self.request.with(|r|r.active_counts_only) { "active" } else { "assigned" };
            if complete { format!("{total} {kind} · {errors} errors · {escalated} escalated") }
            else { format!("Partial counts · {total} {kind} reported · {errors} errors · {escalated} escalated") }
        })
    }
    fn pending(self) -> bool {
        self.accepted
            .with(|s| s.as_ref().is_none_or(|s| s.request != self.request.get()))
            && self.error.get().is_none()
    }
}

/// Embeddable, read-only monitoring of one exact FluxPro definition version.
///
/// Supply reactive `document` and `snapshot` signals. The host handles each
/// `on_request` using its authenticated API and echoes the complete request in
/// the response. Polling defaults to five seconds; set it to zero for host-pushed
/// updates. Counters cover the whole version, independent of list filtering.
/// Tabs are internal, closable views identified by instance UUID.
///
/// ```rust,no_run
/// use fluxpro_editor::{EditorDocument, ProcessMonitor, MonitorSnapshot, MonitorRequest};
/// use leptos::prelude::*;
/// # fn example() -> impl IntoView {
/// let snapshot = RwSignal::new(MonitorSnapshot::default());
/// let on_request = Callback::new(move |request: MonitorRequest| {
///     // Fetch through your application's API, then snapshot.set(response).
///     // The response must echo `request`, including its scope and revision.
/// });
/// view! { <ProcessMonitor document=EditorDocument::default()
///     snapshot=snapshot on_request=on_request/> }
/// # }
/// ```
#[component]
pub fn ProcessMonitor(
    /// Exclude completed instances from graph counts (list filters remain independent).
    #[prop(default = false)]
    active_counts_only: bool,
    /// Reactive definition and saved layout. A version change resets open tabs.
    #[prop(into)]
    document: Signal<EditorDocument>,
    /// Reactive version-scoped server snapshot. Stale responses are ignored.
    #[prop(into)]
    snapshot: Signal<MonitorSnapshot>,
    /// Read-only request callback for counts, instances and open-tab details.
    on_request: Callback<MonitorRequest>,
    /// Polling period in milliseconds; zero disables polling. Minimum is 1000.
    #[prop(default = 5000)]
    refresh_interval_ms: u32,
    /// Show the instance list and detail panes; disable for definition-only access.
    #[prop(default = true)]
    show_instances: bool,
    /// Host-driven instance selection; foreign-version instances are ignored.
    #[prop(optional, into)]
    open_instance: Signal<Option<MonitorInstance>>,
    /// Optional host actions rendered in the Events section of each instance pane.
    #[prop(optional)]
    instance_actions: Option<Callback<Signal<Option<MonitorInstanceDetails>>, AnyView>>,
) -> impl IntoView {
    view! {<section class="fluxpro-editor fluxpro-monitor" aria-label="FluxPro process monitor"><style>{EDITOR_CSS}</style><style>{include_str!("monitor.css")}</style>
        <For each=move ||vec![MonitorScope::from_document(&document.get())] key=|scope|scope.clone() children=move |scope|view!{<MonitorBody document=document snapshot=snapshot on_request=on_request scope=scope refresh_interval_ms=refresh_interval_ms open_instance=open_instance instance_actions=instance_actions show_instances=show_instances active_counts_only=active_counts_only/>}/>
    </section>}
}
#[component]
fn MonitorBody(
    active_counts_only: bool,
    document: Signal<EditorDocument>,
    snapshot: Signal<MonitorSnapshot>,
    on_request: Callback<MonitorRequest>,
    scope: MonitorScope,
    refresh_interval_ms: u32,
    show_instances: bool,
    open_instance: Signal<Option<MonitorInstance>>,
    instance_actions: Option<Callback<Signal<Option<MonitorInstanceDetails>>, AnyView>>,
) -> impl IntoView {
    let session = MonitorSession {
        document,
        request: RwSignal::new(MonitorRequest {
            active_counts_only,
            scope,
            revision: next_revision(),
            ..Default::default()
        }),
        accepted: RwSignal::new(None),
        error: RwSignal::new(None),
        active: RwSignal::new(None),
        tabs: RwSignal::new(Vec::new()),
        live: RwSignal::new(refresh_interval_ms > 0),
    };
    // Apply the initial selection before the first request, including during SSR.
    if let Some(instance) = open_instance.get_untracked().filter(|_| show_instances) {
        session.open(instance);
    }
    Effect::new(move |_| {
        if let Some(instance) = open_instance.get().filter(|_| show_instances) {
            session.open(instance);
        }
    });
    Effect::new(move |_| {
        on_request.run(session.request.get());
    });
    Effect::new(move |_| {
        let mut incoming = snapshot.get();
        let request = session.request.get_untracked();
        if incoming.request != request {
            return;
        }
        if !valid_snapshot(&incoming, &request) {
            session.error.set(Some(
                "The server returned instances from a different process version.".into(),
            ));
            return;
        }
        if let Some(error) = incoming.error.clone() {
            session.error.set(Some(error));
            return;
        }
        // A partial history failure must not erase the last readable details.
        if let Some(previous) = session.accepted.get_untracked() {
            for detail in &mut incoming.details {
                if let Some(error) = detail.error.clone() {
                    if let Some(old) = previous
                        .details
                        .iter()
                        .find(|old| old.instance.uuid == detail.instance.uuid)
                    {
                        *detail = old.clone();
                        detail.error = Some(error);
                    }
                }
            }
        }
        session.error.set(None);
        session.tabs.update(|tabs| {
            for tab in tabs {
                if let Some(current) = incoming
                    .details
                    .iter()
                    .map(|d| &d.instance)
                    .chain(incoming.instances.items.iter())
                    .find(|i| i.uuid == tab.uuid)
                {
                    *tab = current.clone();
                }
            }
        });
        session.accepted.set(Some(incoming));
    });
    #[cfg(target_arch = "wasm32")]
    if refresh_interval_ms > 0 {
        if let Ok(handle) = leptos::leptos_dom::helpers::set_interval_with_handle(
            move || {
                if session.live.get_untracked() && !untrack(move || session.pending()) {
                    session.change(|_| {});
                }
            },
            std::time::Duration::from_millis(refresh_interval_ms.max(1000) as u64),
        ) {
            on_cleanup(move || handle.clear());
        }
    }
    view! {
        <header class="fp-toolbar"><div class="fp-brand"><span class="fp-brand-mark">"◉"</span><div><strong>"Process monitor"</strong><span>"FLUXPRO"</span></div></div>
            <div class="fp-process-name">{move ||document.with(|d|d.definition.name.clone())}<span class="fp-draft">{move ||document.with(|d|format!("v{}",d.definition.version))}</span></div>
            <div class="fp-actions"><Show when={move ||refresh_interval_ms>0}><button type="button" aria-pressed=move ||!session.live.get() on:click=move |_|session.live.update(|v|*v=!*v)>{move ||if session.live.get(){"Ⅱ Pause updates"}else{"▶ Resume updates"}}</button></Show>
                <button type="button" on:click=move |_|session.change(|_|{})>"↻ Refresh"</button>
            </div>
        </header>
        <div class="fm-health"><span class:fm-online=move ||!session.pending()&&session.error.get().is_none()>{move ||if session.error.get().is_some(){"● Update failed"}else if session.pending(){"◌ Updating…"}else if session.live.get(){"● Live"}else{"● Snapshot"}}</span>
            <span>{move ||session.health_summary()}</span>
            <span class="fm-observed">{move ||session.accepted.with(|s|s.as_ref().and_then(|s|s.observed_at.as_ref()).map(|s|format!("Observed {s}")).unwrap_or_default())}</span>
        </div>
        <nav hidden=!show_instances class="fm-tabs" aria-label="Monitor tabs"><button type="button" class:fm-active=move ||session.active.get().is_none() aria-pressed=move ||session.active.get().is_none() on:click=move |_|session.active.set(None)>"Instances"</button>
            <For each=move ||session.tabs.get() key=|i|i.uuid.clone() children=move |instance|{let id=StoredValue::new(instance.uuid);let title=StoredValue::new(instance.process_id);view!{<div class="fm-instance-tab" class:fm-active=move ||session.active.get().as_deref()==Some(id.get_value().as_str())>
                <button type="button" aria-pressed=move ||session.active.get().as_deref()==Some(id.get_value().as_str()) on:click=move |_|session.active.set(Some(id.get_value()))>{title.get_value()}</button>
                <button type="button" aria-label=format!("Close instance {}",title.get_value()) on:click=move |_|session.close(&id.get_value())>"×"</button>
            </div>}}/>
        </nav>
        <Show when=move ||session.error.get().is_some()><div class="fm-error" role="alert">{move ||session.error.get().unwrap_or_default()}" Previous data may be stale; use Refresh to retry."</div></Show>
        <div class="fm-workspace"><MonitorCanvas session=session/><div class="fm-inspection" hidden=!show_instances>
            <div class="fm-pane" hidden=move ||session.active.get().is_some()><InstanceList session=session/></div>
            <For each=move ||session.tabs.get() key=|i|i.uuid.clone() children=move |instance|{let id=StoredValue::new(instance.uuid.clone());view!{<div class="fm-pane" hidden=move ||session.active.get().as_deref()!=Some(id.get_value().as_str())><InstanceDetails session=session instance=instance actions=instance_actions/></div>}}/>
        </div></div>
        <footer class="fp-status"><span>"Read-only · "{move ||session.request.with(|r|format!("{} / {}",r.scope.key,r.scope.version))}</span><span>"Pinch to zoom · ⌘ + drag to pan · Select a block to filter"</span></footer>
    }
}
#[component]
fn InstanceList(session: MonitorSession) -> impl IntoView {
    let search = RwSignal::new(String::new());
    let page = Memo::new(move |_| {
        session.accepted.with(|s| {
            s.as_ref()
                .filter(|s| s.request.query == session.request.get().query)
                .map(|s| s.instances.clone())
        })
    });
    let run_search = move || session.query(|q| q.search = search.get_untracked().trim().into());
    view! {<section class="fm-instance-list"><div class="fm-section-heading"><span class="fp-eyebrow">"CURRENT VERSION"</span><h2>"Process instances"</h2></div>
        <form class="fm-search" on:submit=move |ev|{ev.prevent_default();run_search();}><input aria-label="Search instances by key or token" placeholder="Business key, token or instance UUID…" prop:value=move ||search.get() on:input=move |ev|search.set(event_target_value(&ev))/><button class="fp-primary" type="submit">"Search"</button></form>
        <div class="fm-filters"><label>"State"<select aria-label="Instance state" prop:value=move ||session.request.with(|r|r.query.state.clone().unwrap_or_default()) on:change=move |ev|{let v=event_target_value(&ev);session.query(|q|q.state=(!v.is_empty()).then_some(v));}><option value="">"All states"</option>{["created","running","waiting","suspended","failed","completed"].into_iter().map(|state|view!{<option value=state>{state}</option>}).collect_view()}</select></label>
            <label>"Block"<select aria-label="Instance block" prop:value=move ||session.request.with(|r|r.query.node_id.clone().unwrap_or_default()) on:change=move |ev|{let v=event_target_value(&ev);session.query(|q|q.node_id=(!v.is_empty()).then_some(v));}><option value="">"All blocks"</option>{move ||session.document.with(|d|d.definition.nodes.iter().map(|n|{let id=n.id().to_string();view!{<option value=id.clone()>{id.clone()}</option>}}).collect_view())}</select></label>
            <button type="button" on:click=move |_|{search.set(String::new());session.query(|q|*q=MonitorQuery::default());}>"Clear"</button>
        </div>
        <Show when=move ||page.get().is_none()><p role="status" class="fm-empty">"Loading matching instances…"</p></Show>
        <Show when=move ||page.get().is_some_and(|p|p.items.is_empty())><p class="fm-empty">"No instances match this version and search."</p></Show>
        <div class="fm-table-scroll"><table class="fm-table"><thead><tr><th>"Business key / token"</th><th>"State / block"</th><th>"Issues"</th></tr></thead><tbody>
            <For each=move ||page.get().map(|p|p.items).unwrap_or_default() key=|i|serde_json::to_string(i).unwrap_or_default() children=move |instance|{let item=StoredValue::new(instance);view!{<tr>
                <td><button class="fm-link" type="button" on:click=move |_|session.open(item.get_value())>{item.get_value().process_id}</button><code>{item.get_value().token}</code><small>{item.get_value().created_at}</small></td>
                <td><span class="fm-state">{item.get_value().state}</span><small>{item.get_value().current_node_id.unwrap_or_else(||"Not assigned".into())}</small></td>
                <td>{item.get_value().issues.into_iter().map(|issue|view!{<span class=if issue.kind=="error"{"fm-issue fm-issue-error"}else{"fm-issue fm-issue-escalation"} title=issue.message>{match issue.kind.as_str(){"error"=>"! Error","escalation"=>"↑ Escalation",_=>"Warning"}}</span>}).collect_view()}</td>
            </tr>}}/>
        </tbody></table></div>
        <div class="fm-pagination"><span>{move ||page.get().map(|p|if p.total==0{"0 instances".into()}else{let offset=session.request.with(|r|r.query.offset);format!("{}–{} of {}",offset+1,offset+p.items.len() as u64,p.total)}).unwrap_or_default()}</span>
            <button type="button" disabled=move ||session.request.with(|r|r.query.offset==0)||page.get().is_none() on:click=move |_|session.change(|r|r.query.offset=r.query.offset.saturating_sub(r.query.limit as u64))>"Previous"</button>
            <button type="button" disabled=move ||page.get().is_none_or(|p|session.request.with(|r|r.query.offset+r.query.limit as u64>=p.total)) on:click=move |_|session.change(|r|r.query.offset+=r.query.limit as u64)>"Next"</button>
        </div>
    </section>}
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> MonitorSession {
        let document = EditorDocument::default();
        let scope = MonitorScope::from_document(&document);
        MonitorSession {
            document: Signal::stored(document),
            request: RwSignal::new(MonitorRequest {
                scope,
                revision: next_revision(),
                ..Default::default()
            }),
            accepted: RwSignal::new(None),
            error: RwSignal::new(None),
            active: RwSignal::new(None),
            tabs: RwSignal::new(Vec::new()),
            live: RwSignal::new(false),
        }
    }
    #[test]
    fn tabs_use_uuid_preserve_queries_and_close_without_changing_the_definition() {
        Owner::new().with(|| {
            let s = session();
            let original = s.document.get_untracked().to_project_yaml().unwrap();
            let scope = s.request.get_untracked().scope;
            let instance = MonitorInstance {
                uuid: "instance-a".into(),
                process_id: "same-business-key".into(),
                process_key: scope.key,
                process_version: scope.version,
                ..Default::default()
            };
            s.query(|q| q.search = "same-business-key".into());
            s.open(instance.clone());
            s.open(instance.clone());
            assert_eq!(s.tabs.get_untracked().len(), 1);
            let mut second = instance.clone();
            second.uuid = "instance-b".into();
            s.open(second);
            assert_eq!(s.tabs.get_untracked().len(), 2);
            assert_eq!(s.request.get_untracked().instances.len(), 2);
            s.close("instance-b");
            assert_eq!(s.active.get_untracked(), Some("instance-a".into()));
            s.close("instance-a");
            assert!(s.active.get_untracked().is_none());
            assert!(s.request.get_untracked().instances.is_empty());
            assert_eq!(s.request.get_untracked().query.search, "same-business-key");
            assert_eq!(
                s.document.get_untracked().to_project_yaml().unwrap(),
                original
            );
            let mut foreign = instance;
            foreign.process_version = "foreign".into();
            s.open(foreign);
            assert!(s.tabs.get_untracked().is_empty());
        });
    }
    #[test]
    fn returning_to_a_scope_cannot_reuse_an_old_request_revision() {
        Owner::new().with(|| {
            let first = session();
            let second = session();
            assert_ne!(
                first.request.get_untracked().revision,
                second.request.get_untracked().revision
            );
            let old = first.request.get_untracked();
            first.change(|_| {});
            assert_ne!(first.request.get_untracked(), old);
        });
    }
    #[cfg(feature = "ssr")]
    #[test]
    fn monitor_renders_counts_issue_labels_and_read_only_controls() {
        Owner::new().with(|| {
            let s = session();
            s.accepted.set(Some(MonitorSnapshot {
                request: s.request.get_untracked(),
                node_counts: vec![MonitorNodeCount {
                    node_id: "start".into(),
                    instance_count: 8,
                    errors: 2,
                    escalations: 3,
                }],
                ..Default::default()
            }));
            assert!(untrack(move || s.health_summary()).starts_with("Partial counts"));
            let html = view! {<MonitorCanvas session=s/>}.to_html();
            for text in [
                "start: 8 instances",
                "finish: unknown instances",
                "2 errors",
                "3 escalated",
                "Monitor zoom in",
                "Monitor process connections",
            ] {
                assert!(html.contains(text), "missing {text}");
            }
            assert!(!html.contains("draggable=\"true\""));
            assert!(!html.contains("Add block"));
            assert!(!html.contains("collect::"));
            // Only populated nodes are highlighted; unknown and zero counts are neutral.
            assert_eq!(html.matches("fm-node-occupied").count(), 1);
            s.accepted.update(|snapshot| {
                snapshot.as_mut().unwrap().node_counts[0].instance_count = 0;
            });
            let empty = view! {<MonitorCanvas session=s/>}.to_html();
            assert!(empty.contains("start: 0 instances"));
            assert!(!empty.contains("fm-node-occupied"));
        });
    }
}
