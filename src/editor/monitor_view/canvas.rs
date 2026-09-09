use super::MonitorSession;
use crate::editor::{
    BlockKind, Position,
    viewport::{MAX_ZOOM, MIN_ZOOM, clamp_zoom, zoom_scroll},
};
use leptos::{ev, html, prelude::*};

#[component]
pub(super) fn MonitorCanvas(session: MonitorSession) -> impl IntoView {
    let viewport = NodeRef::<html::Div>::new();
    let zoom = RwSignal::new(1.0_f64);
    let pending = RwSignal::new(None::<Position>);
    let gesture = RwSignal::new(None::<f64>);
    let pan = RwSignal::new(None::<(i32, f64, f64, f64, f64)>);
    let zoom_at = Callback::new(move |(scale, anchor): (f64, Position)| {
        let Some(el) = viewport.get_untracked() else {
            return;
        };
        let scroll = pending.get_untracked().unwrap_or(Position::new(
            el.scroll_left() as f64,
            el.scroll_top() as f64,
        ));
        let next = clamp_zoom(scale);
        pending.set(Some(zoom_scroll(
            scroll,
            anchor,
            zoom.get_untracked(),
            next,
        )));
        zoom.set(next);
        leptos::leptos_dom::helpers::request_animation_frame(move || {
            if let Some(Some(p)) = pending.try_get_untracked() {
                pending.set(None);
                if let Some(Some(el)) = viewport.try_get_untracked() {
                    el.set_scroll_left(p.x.round() as i32);
                    el.set_scroll_top(p.y.round() as i32);
                }
            }
        });
    });
    let zoom_center = move |value| {
        if let Some(el) = viewport.get_untracked() {
            zoom_at.run((
                value,
                Position::new(
                    el.client_width() as f64 / 2.0,
                    el.client_height() as f64 / 2.0,
                ),
            ));
        }
    };
    let current_node = Memo::new(move |_| {
        let id = session.active.get()?;
        session.accepted.with(|s| {
            s.as_ref()?
                .details
                .iter()
                .find(|d| d.instance.uuid == id)?
                .instance
                .current_node_id
                .clone()
        })
    });
    // Center on entry initially and on the active instance as its node changes.
    Effect::new(move |_| {
        let current = current_node.get();
        let position = session.document.with_untracked(|d| {
            current
                .as_ref()
                .and_then(|id| d.positions.get(id))
                .copied()
                .or_else(|| {
                    d.definition
                        .nodes
                        .iter()
                        .find(|n| n.is_start())
                        .and_then(|n| d.positions.get(n.id().get_id()))
                        .copied()
                })
                .unwrap_or_default()
        });
        if let Some(el) = viewport.get() {
            el.set_scroll_left(
                ((position.x + 112.0) * zoom.get_untracked() - el.client_width() as f64 / 2.0)
                    .max(0.0) as i32,
            );
            el.set_scroll_top((position.y * zoom.get_untracked() - 80.0).max(0.0) as i32);
        }
    });
    let size = Memo::new(move |_| session.document.with(|d| d.canvas_size()));
    view! {<main class="fp-canvas-wrap fm-canvas-wrap"><div class="fp-canvas-caption"><span class="fp-canvas-dot"></span>"LIVE DIAGRAM"<span>"VERSION-WIDE COUNTS"</span></div>
        <div class="fp-canvas" node_ref=viewport tabindex="0" aria-label="Live process diagram" class:fp-panning=move ||pan.get().is_some()
            on:wheel=move |event:ev::WheelEvent|{
                let Some(el)=viewport.get_untracked()else{return;};let unit=match event.delta_mode(){1=>16.0,2=>el.client_height() as f64,_=>1.0};
                if event.ctrl_key(){event.prevent_default();event.stop_propagation();if gesture.get_untracked().is_none(){let rect=el.get_bounding_client_rect();zoom_at.run((zoom.get_untracked()*(-event.delta_y()*unit*0.008).clamp(-1.0,1.0).exp(),Position::new(event.client_x() as f64-rect.left(),event.client_y() as f64-rect.top())));}}
                else if event.meta_key(){event.prevent_default();el.set_scroll_left(el.scroll_left()+(event.delta_x()*unit) as i32);el.set_scroll_top(el.scroll_top()+(event.delta_y()*unit) as i32);}
            }
            on:gesturestart=move |event:ev::Event|{event.prevent_default();gesture.set(Some(zoom.get_untracked()));}
            on:gesturechange=move |event:ev::Event|{event.prevent_default();if let (Some(initial),Some(el))=(gesture.get_untracked(),viewport.get_untracked()){let rect=el.get_bounding_client_rect();let num=|key:&str|js_sys::Reflect::get(event.as_ref(),&key.into()).ok().and_then(|v|v.as_f64()).filter(|v|v.is_finite());zoom_at.run((initial*num("scale").unwrap_or(1.0),Position::new(num("clientX").map(|x|x-rect.left()).unwrap_or(el.client_width() as f64/2.0),num("clientY").map(|y|y-rect.top()).unwrap_or(el.client_height() as f64/2.0))));}}
            on:gestureend=move |event:ev::Event|{event.prevent_default();gesture.set(None);}
            on:pointerdown=move |event:ev::PointerEvent|{if event.meta_key()&&event.button()==0{if let Some(el)=viewport.get_untracked(){event.prevent_default();pan.set(Some((event.pointer_id(),event.client_x() as f64,event.client_y() as f64,el.scroll_left() as f64,el.scroll_top() as f64)));let _=el.set_pointer_capture(event.pointer_id());}}}
            on:pointermove=move |event:ev::PointerEvent|{if let (Some((id,x,y,sx,sy)),Some(el))=(pan.get_untracked(),viewport.get_untracked()){if id==event.pointer_id(){el.set_scroll_left((sx+x-event.client_x() as f64) as i32);el.set_scroll_top((sy+y-event.client_y() as f64) as i32);}}}
            on:pointerup=move |_:ev::PointerEvent|pan.set(None) on:pointercancel=move |_:ev::PointerEvent|pan.set(None) on:lostpointercapture=move |_:ev::PointerEvent|pan.set(None)>
            <div class="fp-world" style=move ||{let (w,h)=size.get();format!("width:{}px;height:{}px",w*zoom.get(),h*zoom.get())}>
                <div class="fp-scene" style=move ||{let(w,h)=size.get();format!("width:{w}px;height:{h}px;transform:scale({});transform-origin:0 0",zoom.get())}>
                    <svg class="fp-connections" width=move ||size.get().0 height=move ||size.get().1 role="img" aria-label="Monitor process connections">
                        {move ||session.document.with(|d|d.connections().into_iter().filter_map(|edge|{let path=d.connection_path(&edge)?;let branch=d.branch_port(&edge);let color=branch.map(|(_,color)|color).or_else(||edge.special_outlet().map(|(_,color,_)|color)).unwrap_or("#acb9cd");let target=d.positions.get(&edge.target)?;let source=d.positions.get(&edge.source)?;let x=target.x+112.0;let y=target.y;let arrow=format!("M {} {} L {x} {y} L {} {}",x-4.0,y-7.0,x+4.0,y-7.0);let label=if branch.is_some(){String::new()}else{edge.label.chars().take(28).collect::<String>()};Some(view!{<g><title>{format!("{} → {}: {}",edge.source,edge.target,edge.label)}</title><path d=path fill="none" stroke=color stroke-width="2"/><path d=arrow fill="none" stroke=color stroke-width="2"/><text x=source.x+120.0 y=source.y+112.0 fill="#7d8ca1" font-size="10">{label}</text></g>})}).collect_view())}
                    </svg>
                    <For each=move ||session.document.with(|d|d.definition.nodes.clone()) key=|n|n.id().to_string() children=move |node|{
                        let id=StoredValue::new(node.id().to_string());let kind=BlockKind::of(&node);
                        let count=Memo::new(move |_|session.accepted.with(|s|s.as_ref().and_then(|s|s.node_counts.iter().find(|c|c.node_id==id.get_value())).cloned()));
                        view!{<button type="button" class="fm-node" class:fm-node-current=move ||current_node.get().as_deref()==Some(id.get_value().as_str())
                            class:fm-node-occupied=move ||count.get().is_some_and(|c|c.instance_count>0)
                            class:fm-node-branched=move ||session.document.with(|d|!d.branch_routes(&id.get_value()).is_empty())
                            class:fm-node-error=move ||count.get().is_some_and(|c|c.errors>0) class:fm-node-escalation=move ||count.get().is_some_and(|c|c.escalations>0)
                            style=move ||session.document.with(|d|{let p=d.positions.get(&id.get_value()).copied().unwrap_or_default();format!("left:{}px;top:{}px;height:{}px",p.x,p.y,d.node_height(&id.get_value()))})
                            aria-label=move ||format!("{}: {} instances",id.get_value(),count.get().map(|c|c.instance_count.to_string()).unwrap_or("unknown".into()))
                            on:click=move |event|{if !event.meta_key(){session.active.set(None);session.query(|q|q.node_id=Some(id.get_value()));}}>
                            <span class="fm-node-icon">{kind.symbol()}</span><span class="fm-node-title"><strong>{kind.label()}</strong><small>{id.get_value()}</small></span>
                            <span class="fm-node-count">{move ||count.get().map(|c|c.instance_count.to_string()).unwrap_or("—".into())}</span>
                            <crate::editor::components::BranchRows routes=Signal::derive(move ||session.document.with(|d|d.branch_routes(&id.get_value())))/>
                            <crate::editor::components::SpecialRoutePorts routes=Signal::derive(move ||session.document.with(|d|d.special_routes(&id.get_value())))/>
                            <span class="fm-node-alerts"><Show when=move ||count.get().is_some_and(|c|c.errors>0)><span class="fm-issue-error">{move ||format!("! {} errors",count.get().map_or(0,|c|c.errors))}</span></Show><Show when=move ||count.get().is_some_and(|c|c.escalations>0)><span class="fm-issue-escalation">{move ||format!("↑ {} escalated",count.get().map_or(0,|c|c.escalations))}</span></Show></span>
                        </button>}
                    }/>
                </div>
            </div>
        </div>
        <div class="fp-canvas-navigation"><span class="fm-legend"><i class="fm-legend-current"></i>"Current block"<i class="fm-legend-error"></i>"Error"<i class="fm-legend-escalation"></i>"Escalation"</span>
            <button type="button" aria-label="Monitor zoom out" disabled={move ||zoom.get()<=MIN_ZOOM} on:click=move |_|zoom_center(zoom.get_untracked()/1.2)>"−"</button>
            <button type="button" aria-label="Monitor reset zoom" on:click=move |_|zoom_center(1.0)>{move ||format!("{:.0}%",zoom.get()*100.0)}</button>
            <button type="button" aria-label="Monitor zoom in" disabled={move ||zoom.get()>=MAX_ZOOM} on:click=move |_|zoom_center(zoom.get_untracked()*1.2)>"+"</button>
        </div>
    </main>}
}
