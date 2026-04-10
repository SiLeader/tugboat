use crate::base::TugboatController;
use crate::error::ControllerError;
use tugboat_client::runtime::{Action, Controller, ReconcileEvent, Reconciler};
use tugboat_client::{Api, TugboatClient};
use tugboat_resources::manifests::core::v1::{Namespace, ServiceAccount};
use tugboat_resources::manifests::meta::v1::ObjectMeta;
use tugboat_resources::{ObjectMetaResource, Resource};

const DEFAULT_SERVICE_ACCOUNT_NAME: &str = "default";

#[derive(Clone)]
struct NamespaceDefaultServiceAccountReconciler {
    client: TugboatClient,
}

pub(crate) struct NamespaceDefaultServiceAccountController {
    controller: Controller<Namespace>,
    reconciler: NamespaceDefaultServiceAccountReconciler,
}

impl NamespaceDefaultServiceAccountController {
    pub(crate) fn new(client: TugboatClient) -> Self {
        Self {
            controller: Controller::new(Api::all(client.clone())),
            reconciler: NamespaceDefaultServiceAccountReconciler { client },
        }
    }
}

#[async_trait::async_trait]
impl TugboatController for NamespaceDefaultServiceAccountController {
    fn name(&self) -> &str {
        "namespace-default-service-account"
    }

    async fn setup(&mut self) {}

    async fn run(&self) {
        self.controller.clone().run(self.reconciler.clone()).await;
    }
}

#[async_trait::async_trait]
impl Reconciler<Namespace> for NamespaceDefaultServiceAccountReconciler {
    type Error = ControllerError;

    async fn reconcile(&self, event: ReconcileEvent<Namespace>) -> Result<Action, Self::Error> {
        match event {
            ReconcileEvent::Applied(namespace) => self.reconcile_applied(namespace).await,
            ReconcileEvent::Deleted(_) => Ok(Action::await_change()),
        }
    }
}

impl NamespaceDefaultServiceAccountReconciler {
    async fn reconcile_applied(&self, namespace: Namespace) -> Result<Action, ControllerError> {
        if namespace.deletion_timestamp().is_some() {
            return Ok(Action::await_change());
        }

        let namespace_name = namespace
            .name()
            .ok_or(ControllerError::MissingName("Namespace"))?;
        let service_account_api: Api<ServiceAccount> =
            Api::namespaced(self.client.clone(), namespace_name);

        let service_account = build_default_service_account(namespace_name);
        match service_account_api.create(service_account).await {
            Ok(_) => Ok(Action::await_change()),
            Err(tugboat_client::Error::Api(status)) if status.code == 409 => {
                Ok(Action::await_change())
            }
            Err(err) => Err(err.into()),
        }
    }
}

fn build_default_service_account(namespace: &str) -> ServiceAccount {
    ServiceAccount {
        type_meta: Some(ServiceAccount::type_meta()),
        object_meta: Some(ObjectMeta {
            name: Some(DEFAULT_SERVICE_ACCOUNT_NAME.to_string()),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_SERVICE_ACCOUNT_NAME, build_default_service_account};
    use tugboat_resources::Resource;

    #[test]
    fn builds_default_service_account_in_namespace() {
        let service_account = build_default_service_account("demo");

        assert_eq!(
            service_account.type_meta,
            Some(tugboat_resources::manifests::core::v1::ServiceAccount::type_meta())
        );
        assert_eq!(
            service_account
                .object_meta
                .as_ref()
                .and_then(|meta| meta.name.as_deref()),
            Some(DEFAULT_SERVICE_ACCOUNT_NAME)
        );
        assert_eq!(
            service_account
                .object_meta
                .as_ref()
                .and_then(|meta| meta.namespace.as_deref()),
            Some("demo")
        );
        assert!(service_account.secrets.is_empty());
    }

    #[test]
    fn default_service_account_name_constant_matches_expected_value() {
        assert_eq!(DEFAULT_SERVICE_ACCOUNT_NAME, "default");
    }
}
