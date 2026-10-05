use serde_json::json;
use uuid::Uuid;
use vox_connections::{
    conformance::{ReferencePlatform, canonical_suite, run_suite},
    connected_apps::{ConnectedAppsOptions, ConnectedAppsService},
    identity::{DeploymentId, RequestContext, RequestScope, RequestSubject, UserContextId, UserId},
    providers::expedia::integration_declaration,
    remote_extensions::{
        RemoteExtensionService,
        adapters::{ProtocolRouter, privacy::scan_for_prohibited_content},
    },
    skills::SkillService,
};

struct IndependentHost {
    context: RequestContext,
}

impl RequestScope for IndependentHost {
    fn request_context(&self) -> RequestContext {
        self.context
    }
}

#[tokio::test]
async fn independent_host_can_construct_connector_services() {
    let host = IndependentHost {
        context: RequestContext {
            id: UserContextId(Uuid::new_v4()),
            user_id: UserId(Uuid::new_v4()),
            subject: RequestSubject {
                deployment_id: DeploymentId(Uuid::new_v4()),
            },
        },
    };
    assert_eq!(host.request_context().id, host.context.id);
    assert_eq!(integration_declaration("independent").capabilities.len(), 3);

    let db = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://user:password@localhost/connectors")
        .expect("valid connection settings");
    let _extensions = RemoteExtensionService::new(db.clone());
    let _skills = SkillService::new(db.clone());
    let _apps = ConnectedAppsService::from_options(db, ConnectedAppsOptions::default());
}

#[test]
fn connector_boundary_redacts_and_requires_declared_guarantees() {
    let payload = json!({"search": "hello", "api_token": "sensitive", "nested": {"message": "Bearer hidden"}});
    let safe = ProtocolRouter::minimize_context(
        &payload,
        &["search".into(), "api_token".into(), "nested".into()],
    );
    assert_eq!(
        safe,
        json!({"search": "hello", "nested": {"message": "[REDACTED]"}})
    );
    assert!(scan_for_prohibited_content(&json!({"approval_id": "x"})).is_err());
    assert!(
        ProtocolRouter::validate_guarantees(&["cancellation_supported".into()], &json!({}))
            .is_err()
    );
}

#[test]
fn bundled_conformance_suite_passes_reference_platform() {
    let suite = canonical_suite();
    let report = run_suite(&suite, ReferencePlatform::default);
    assert!(report.is_conformant(), "{:#?}", report.failures);
    assert!(report.steps > 0);
}
