//! Business examples execute through the same persisted dispatcher as production tasks.
#[path = "../../examples/support/business.rs"]
mod business;

#[test]
fn business_definitions_are_valid_and_have_no_unreachable_nodes() {
    for (name, _) in business::DEFINITIONS {
        let definition = business::definition(name).unwrap();
        definition.validate().unwrap();
        assert!(definition.unreachable_nodes().is_empty(), "{name}");
    }
    let scenarios = business::scenarios().unwrap();
    let names: std::collections::HashSet<_> = scenarios.iter().map(|s| &s.name).collect();
    assert_eq!(names.len(), scenarios.len());
    for (name, _) in business::DEFINITIONS {
        assert!(scenarios.iter().filter(|s| s.workflow == *name).count() >= 2);
    }
}

macro_rules! scenario_test {
    ($name:ident) => {
        #[sqlx::test(migrations = false)]
        #[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
        async fn $name(pool: sqlx::PgPool) {
            let scenario = business::scenarios()
                .unwrap()
                .into_iter()
                .find(|s| s.name == stringify!($name))
                .unwrap();
            tokio::time::timeout(
                std::time::Duration::from_secs(30),
                business::run(pool, scenario),
            )
            .await
            .unwrap()
            .unwrap();
        }
    };
}

scenario_test!(pizza_delivered);
scenario_test!(pizza_out_of_stock);
scenario_test!(pizza_payment_timeout);
scenario_test!(pizza_delivery_refund);
scenario_test!(repair_completed);
scenario_test!(repair_rework_and_payment_reminder);
scenario_test!(repair_quote_declined);
scenario_test!(repair_no_technician);
scenario_test!(school_graduation);
scenario_test!(school_absence_and_remedial_exam);
scenario_test!(school_offer_declined);
scenario_test!(loan_repaid);
scenario_test!(loan_manual_review_overdue_and_closure_recheck);
scenario_test!(loan_scoring_declined);
scenario_test!(loan_offer_expired);
scenario_test!(loan_funding_failed);
scenario_test!(return_refund_reconciled);
scenario_test!(return_ineligible);
scenario_test!(claim_settled_after_escalation);
scenario_test!(claim_rejected);
scenario_test!(subscription_renewed);
scenario_test!(subscription_paid_in_grace);
scenario_test!(subscription_grace_expired);
