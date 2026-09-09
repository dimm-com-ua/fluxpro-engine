//! Executes the user's exact process graphs with controlled host results.
use super::*;
use fluxpro_engine::models::process_def::{Next, Node};
use std::collections::HashSet;
use std::sync::Mutex;

fn submitted(name: &str, reviewed: bool) -> ProcessDefinition {
    let yaml = match (name, reviewed) {
        ("cash_loan", false) => include_str!("../fixtures/submitted/cash_loan.yaml"),
        ("cash_loan", true) => include_str!("../../examples/definitions/reviewed/cash_loan.yaml"),
        ("tk_online", false) => include_str!("../fixtures/submitted/tk_online.yaml"),
        ("tk_online", true) => include_str!("../../examples/definitions/reviewed/tk_online.yaml"),
        _ => unreachable!(),
    };
    serde_yaml::from_str(yaml).unwrap()
}
struct Host {
    name: IdField,
    calls: Arc<Mutex<Vec<(String, String)>>>,
}
#[async_trait]
impl FluxproServiceHandler for Host {
    fn get_name(&self) -> IdField {
        self.name.clone()
    }
    async fn process_node(
        &self,
        _: &IdField,
        _: &ContextMap,
        _: Option<&ContextMap>,
    ) -> anyhow::Result<HandleNodeResult> {
        unreachable!("execution-aware entry required by this test")
    }
    async fn process_node_with_execution(
        &self,
        _: &IdField,
        _: &ContextMap,
        _: Option<&ContextMap>,
        execution: &fluxpro_engine::traits::node_handlers::service_node_handler::HandlerExecution,
    ) -> anyhow::Result<HandleNodeResult> {
        self.calls
            .lock()
            .unwrap()
            .push((self.name.to_string(), execution.operation_id.clone()));
        Ok(HandleNodeResult::success())
    }
}
fn host(
    d: &ProcessDefinition,
) -> (
    Arc<FluxproHandlersContainer>,
    Arc<Mutex<Vec<(String, String)>>>,
) {
    let mut names: HashSet<IdField> = d
        .nodes
        .iter()
        .filter_map(|n| {
            if let Node::ServiceTask { handler, .. } = n {
                Some(handler.clone())
            } else {
                None
            }
        })
        .collect();
    if let Some(h) = &d.special_handlers {
        for callback in [
            &h.on_stage_change,
            &h.on_show_form,
            &h.on_hide_form,
            &h.on_process_complete,
        ]
        .into_iter()
        .flatten()
        {
            names.insert(callback.handler());
        }
    }
    let calls = Arc::new(Mutex::new(vec![]));
    let handlers = names
        .into_iter()
        .map(|name| {
            Arc::new(Host {
                name,
                calls: calls.clone(),
            }) as Arc<dyn FluxproServiceHandler + Send + Sync>
        })
        .collect();
    (Arc::new(FluxproHandlersContainer::new(handlers)), calls)
}
async fn begin(f: &Fixture, d: &ProcessDefinition, context: ContextMap) -> IdField {
    f.service
        .start_process_instance(
            d.key.clone(),
            StartProcessInstance {
                process_id: IdField::generate(),
                version: Some(d.version.clone()),
                context,
            },
        )
        .await
        .unwrap()
}
async fn drain(f: &Fixture, handlers: Arc<FluxproHandlersContainer>) -> Vec<String> {
    let mut entered = vec![];
    for _ in 0..100 {
        let Some(task) =
            f.db.fetch_queue_task(&IdField::generate(), 30_000)
                .await
                .unwrap()
        else {
            return entered;
        };
        if let FluxproQueueTask::ProcessNode { node, .. } = &task.task {
            entered.push(node.id().to_string());
        }
        let outcome = f
            .service
            .process_task(task, handlers.clone())
            .await
            .unwrap();
        assert_eq!(outcome, TaskProcessOutcome::Completed);
    }
    panic!("workflow did not reach a wait within 100 steps")
}
async fn emit(f: &Fixture, token: &IdField, name: &str, key: Option<(&str, &str)>) {
    let mut signal = identified_signal(&Uuid::new_v4().to_string(), name);
    if let Some((key, value)) = key {
        signal
            .context
            .0
            .insert(id(key), ContextValue::string(value.into()));
    }
    f.service.post_signal(token.clone(), signal).await.unwrap();
}
async fn expire_current(f: &Fixture, token: &IdField) {
    let changed=sqlx::query("update fluxpro.queue_runner q set run_after=clock_timestamp()-interval '1 second' where q.task ? 'ProcessEvent' and q.node_visit_id=(select node_visit_id from fluxpro.process_instance where token=$1)")
        .bind(token.get_id()).execute(&f.pool).await.unwrap().rows_affected();
    assert_eq!(changed, 1);
}
async fn assert_no_accidental_replays(
    f: &Fixture,
    token: &IdField,
    calls: &Arc<Mutex<Vec<(String, String)>>>,
) {
    let all = calls.lock().unwrap();
    let distinct: HashSet<_> = all.iter().map(|(_, op)| op).collect();
    assert_eq!(
        distinct.len(),
        all.len(),
        "same logical host operation executed twice"
    );
    let duplicates:i64=sqlx::query_scalar("select count(*) from (select queue_task_uuid from fluxpro.process_instance_log where event_type='node.entered' and process_instance_uuid=(select uuid from fluxpro.process_instance where token=$1) group by queue_task_uuid having count(*)>1) repeated")
        .bind(token.get_id()).fetch_one(&f.pool).await.unwrap();
    assert_eq!(duplicates, 0);
    assert!(f.service.get_open_incident(token).await.unwrap().is_none());
}

#[test]
fn exact_submissions_validate_and_all_branches_have_a_matching_input() {
    use fluxpro_engine::service::expressions::evaluate_condition;
    let mut branches = 0;
    for name in ["cash_loan", "tk_online"] {
        let d = submitted(name, false);
        d.validate().unwrap();
        assert!(d.unreachable_nodes().is_empty());
        let pattern = regex::Regex::new(r"ctx\.(\w+)\s*==\s*(?:'([^']*)'|(true|false))").unwrap();
        for node in &d.nodes {
            let routes = match node {
                Node::Gateway { branches, .. } => branches.as_slice(),
                Node::ServiceTask {
                    next: Some(Next::Routes(routes)),
                    ..
                } => &routes.branches,
                _ => &[],
            };
            for (index, branch) in routes.iter().enumerate() {
                branches += 1;
                let mut context = ContextMap(
                    [
                        (id("_last_signal"), ContextValue::string("none".into())),
                        (id("operator_action"), ContextValue::string("none".into())),
                        (id("decision"), ContextValue::string("none".into())),
                    ]
                    .into(),
                );
                for capture in pattern.captures_iter(&branch.when) {
                    let value = if let Some(value) = capture.get(2) {
                        ContextValue::string(value.as_str().into())
                    } else {
                        ContextValue::boolean(&capture[3] == "true")
                    };
                    context.0.insert(id(&capture[1]), value);
                }
                assert!(
                    evaluate_condition(&branch.when, &context).unwrap(),
                    "{name}: {}",
                    branch.when
                );
                let selected = routes
                    .iter()
                    .position(|branch| evaluate_condition(&branch.when, &context).unwrap());
                assert_eq!(
                    selected,
                    Some(index),
                    "shadowed branch in {name}/{}",
                    node.id()
                );
            }
        }
    }
    assert!(branches > 50);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn cash_success_follows_exact_nodes_and_stages(pool: PgPool) {
    let d = submitted("cash_loan", true);
    let (handlers, calls) = host(&d);
    let f = Fixture::new(pool, d.clone()).await;
    let token = begin(
        &f,
        &d,
        ContextMap(
            [
                (id("is_finmon_required"), ContextValue::boolean(true)),
                (
                    id("final_payout_status"),
                    ContextValue::string("success".into()),
                ),
            ]
            .into(),
        ),
    )
    .await;
    let mut path = drain(&f, handlers.clone()).await;
    for (signal, payload, waiting) in [
        ("phone_verified", None, "select_terms_open"),
        ("terms_selected", None, "bankid"),
        ("bankid_ok", None, "add_card"),
        ("card_added", None, "finmon_form"),
        ("finmon_done", None, "wait_decision"),
        (
            "decision_ready",
            Some(("decision", "approve")),
            "final_terms",
        ),
        ("terms_viewed", None, "sign_otp_wait"),
        ("otp_verified", None, "payout_wait"),
        ("payout_confirmed", None, "end_success"),
    ] {
        emit(&f, &token, signal, payload).await;
        path.extend(drain(&f, handlers.clone()).await);
        assert_eq!(f.current(&token).await.as_deref(), Some(waiting));
    }
    assert_eq!(
        path,
        vec![
            "start",
            "init_params",
            "phone_verification",
            "phone_route",
            "save_phone_to_order",
            "close_ticket_phone",
            "select_terms_open",
            "select_terms_open_route",
            "close_ticket_terms",
            "bankid",
            "bankid_route",
            "close_ticket_bankid",
            "add_card",
            "card_route",
            "close_ticket_card",
            "is_finmon_required",
            "finmon_form",
            "finmon_route",
            "close_ticket_finmon",
            "run_scoring",
            "wait_decision",
            "decision_route",
            "close_ticket_decision_approve",
            "final_terms",
            "final_terms_route",
            "close_ticket_final_terms",
            "sign_otp_send",
            "sign_otp_wait",
            "otp_route",
            "close_ticket_otp_then_sign_docs",
            "sign_docs_and_send_to_erp",
            "make_payout",
            "payout_wait",
            "payout_route",
            "send_payout_to_erp",
            "close_ticket_payout_then_check",
            "payout_check",
            "finalize_success",
            "send_link",
            "end_success"
        ]
    );
    let stages:Vec<String>=sqlx::query_scalar("select s.stage_id from fluxpro.process_instance_stage_log l join fluxpro.process_stage s on s.uuid=l.stage_uuid order by l.created_at,l.uuid").fetch_all(&f.pool).await.unwrap();
    assert_eq!(
        stages,
        vec![
            "created",
            "created",
            "phone_verification",
            "calculator",
            "identification",
            "card_tokenization",
            "fin_mon",
            "scoring",
            "credit_info",
            "signing",
            "payout",
            "done_ok"
        ]
    );
    assert_no_accidental_replays(&f, &token, &calls).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn tk_success_waits_for_page_open_and_documents(pool: PgPool) {
    let d = submitted("tk_online", false);
    let (handlers, calls) = host(&d);
    let f = Fixture::new(pool, d.clone()).await;
    let token = begin(&f, &d, ContextMap::default()).await;
    drain(&f, handlers.clone()).await;
    assert_eq!(f.current(&token).await.as_deref(), Some("credit_info_open"));
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner").await,
        0
    );
    for (signal, waiting) in [
        ("credit_info_opened", "credit_info"),
        ("terms_viewed", "bankid"),
        ("bankid_ok", "sign_otp_wait"),
        ("otp_verified", "wait_documents_to_sign"),
        ("docs_arrived", "end_success"),
    ] {
        emit(&f, &token, signal, None).await;
        drain(&f, handlers.clone()).await;
        assert_eq!(f.current(&token).await.as_deref(), Some(waiting));
        if waiting == "wait_documents_to_sign" {
            assert_eq!(
                f.count("select count(*) from fluxpro.queue_runner").await,
                0
            );
        }
    }
    assert_no_accidental_replays(&f, &token, &calls).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn tk_escalation_operator_return_and_rejection_follow_graph(pool: PgPool) {
    let d = submitted("tk_online", false);
    let (handlers, calls) = host(&d);
    let f = Fixture::new(pool, d.clone()).await;
    let token = begin(&f, &d, ContextMap::default()).await;
    drain(&f, handlers.clone()).await;
    emit(&f, &token, "credit_info_opened", None).await;
    drain(&f, handlers.clone()).await;
    expire_current(&f, &token).await;
    drain(&f, handlers.clone()).await;
    assert_eq!(
        f.current(&token).await.as_deref(),
        Some("credit_info_support_wait")
    );
    emit(&f, &token, "terms_viewed", None).await;
    drain(&f, handlers.clone()).await;
    expire_current(&f, &token).await;
    drain(&f, handlers.clone()).await;
    emit(
        &f,
        &token,
        "operator_identification_resolution",
        Some(("operator_action", "back_to_credit_information")),
    )
    .await;
    drain(&f, handlers.clone()).await;
    assert_eq!(f.current(&token).await.as_deref(), Some("credit_info"));
    expire_current(&f, &token).await;
    drain(&f, handlers.clone()).await;
    emit(
        &f,
        &token,
        "operator_credit_info_resolution",
        Some(("operator_action", "reject")),
    )
    .await;
    drain(&f, handlers.clone()).await;
    assert_eq!(f.current(&token).await.as_deref(), Some("end_reject"));
    assert_no_accidental_replays(&f, &token, &calls).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn cash_original_start_body_lacks_required_phone_flag(pool: PgPool) {
    let d = submitted("cash_loan", false);
    let (handlers, _) = host(&d);
    let f = Fixture::new(pool, d.clone()).await;
    let token = begin(
        &f,
        &d,
        ContextMap(
            [(
                id("product_name"),
                ContextValue::string("cash online".into()),
            )]
            .into(),
        ),
    )
    .await;
    let start = f.claim().await;
    f.service
        .process_task(start, handlers.clone())
        .await
        .unwrap();
    let init = f.claim().await;
    assert_eq!(
        f.service.process_task(init, handlers).await.unwrap(),
        TaskProcessOutcome::Suspended
    );
    assert_eq!(
        f.db.get_open_incident(&token).await.unwrap().unwrap().kind,
        "condition.failed"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn rapid_calculator_events_expose_the_single_wait_delivery_contract(pool: PgPool) {
    let d = submitted("cash_loan", true);
    let (handlers, _) = host(&d);
    let f = Fixture::new(pool, d.clone()).await;
    let token = begin(
        &f,
        &d,
        ContextMap([(id("is_phone_confirmed"), ContextValue::boolean(true))].into()),
    )
    .await;
    drain(&f, handlers.clone()).await;
    assert_eq!(
        f.current(&token).await.as_deref(),
        Some("select_terms_open")
    );
    // Both events target the still-open visit. Preserving the second for a future
    // wait needs an explicit delivery rule or an acknowledgement from the client.
    emit(&f, &token, "calculator_interacted", None).await;
    emit(&f, &token, "terms_selected", None).await;
    drain(&f, handlers).await;
    assert_eq!(f.current(&token).await.as_deref(), Some("select_terms"));
    assert_eq!(f.count("select count(*) from fluxpro.process_instance_log where event_type='signal.not_accepted'").await,1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn cash_manual_decision_reminder_and_payout_escalation_follow_graph(pool: PgPool) {
    let d = submitted("cash_loan", true);
    let (handlers, calls) = host(&d);
    let f = Fixture::new(pool, d.clone()).await;
    let token = begin(
        &f,
        &d,
        ContextMap(
            [
                (id("is_phone_confirmed"), ContextValue::boolean(true)),
                (id("is_finmon_required"), ContextValue::boolean(false)),
                (
                    id("final_payout_status"),
                    ContextValue::string("success".into()),
                ),
            ]
            .into(),
        ),
    )
    .await;
    drain(&f, handlers.clone()).await;
    for signal in ["terms_selected", "bankid_ok", "card_added"] {
        emit(&f, &token, signal, None).await;
        drain(&f, handlers.clone()).await;
    }
    assert_eq!(f.current(&token).await.as_deref(), Some("wait_decision"));
    expire_current(&f, &token).await;
    drain(&f, handlers.clone()).await;
    emit(
        &f,
        &token,
        "operator_decision_resolution",
        Some(("operator_action", "manual")),
    )
    .await;
    drain(&f, handlers.clone()).await;
    assert_eq!(
        f.current(&token).await.as_deref(),
        Some("manual_review_wait")
    );
    emit(&f, &token, "decision_ready", Some(("decision", "approve"))).await;
    drain(&f, handlers.clone()).await;
    expire_current(&f, &token).await;
    drain(&f, handlers.clone()).await;
    assert_eq!(
        f.current(&token).await.as_deref(),
        Some("final_terms_after_reminder")
    );
    expire_current(&f, &token).await;
    drain(&f, handlers.clone()).await;
    assert_eq!(
        f.current(&token).await.as_deref(),
        Some("final_terms_support_wait")
    );
    emit(&f, &token, "terms_viewed", None).await;
    drain(&f, handlers.clone()).await;
    emit(&f, &token, "otp_verified", None).await;
    drain(&f, handlers.clone()).await;
    emit(&f, &token, "payout_failed", None).await;
    drain(&f, handlers.clone()).await;
    assert_eq!(
        f.current(&token).await.as_deref(),
        Some("payout_support_wait")
    );
    emit(&f, &token, "payout_confirmed", None).await;
    drain(&f, handlers.clone()).await;
    assert_eq!(f.current(&token).await.as_deref(), Some("end_success"));
    assert_no_accidental_replays(&f, &token, &calls).await;
}
