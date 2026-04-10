use crate::base::TugboatController;
use crate::error::ControllerError;
use rand::distr::{Alphanumeric, SampleString};
use rand::rng;
use std::collections::HashMap;
use tugboat_client::runtime::{Action, Controller, ReconcileEvent, Reconciler};
use tugboat_client::{Api, TugboatClient};
use tugboat_resources::manifests::core::v1::{Secret, ServiceAccount};
use tugboat_resources::manifests::meta::v1::{ObjectMeta, ObjectReference};
use tugboat_resources::{ObjectMetaResource, Resource};

const SERVICE_ACCOUNT_NAME_ANNOTATION: &str = "tugboat.io/service-account.name";
const SERVICE_ACCOUNT_TOKEN_SECRET_TYPE: &str = "tugboat.io/service-account-token";
const TOKEN_DATA_KEY: &str = "token";

#[derive(Clone)]
struct ServiceAccountTokenReconciler {
    client: TugboatClient,
}

pub(crate) struct ServiceAccountTokenController {
    controller: Controller<ServiceAccount>,
    reconciler: ServiceAccountTokenReconciler,
}

impl ServiceAccountTokenController {
    pub(crate) fn new(client: TugboatClient) -> Self {
        Self {
            controller: Controller::new(Api::all(client.clone())),
            reconciler: ServiceAccountTokenReconciler { client },
        }
    }
}

#[async_trait::async_trait]
impl TugboatController for ServiceAccountTokenController {
    fn name(&self) -> &str {
        "service-account-token"
    }

    async fn setup(&mut self) {}

    async fn run(&self) {
        self.controller.clone().run(self.reconciler.clone()).await;
    }
}

#[async_trait::async_trait]
impl Reconciler<ServiceAccount> for ServiceAccountTokenReconciler {
    type Error = ControllerError;

    async fn reconcile(
        &self,
        event: ReconcileEvent<ServiceAccount>,
    ) -> Result<Action, Self::Error> {
        match event {
            ReconcileEvent::Applied(service_account) => self.reconcile_applied(service_account).await,
            ReconcileEvent::Deleted(service_account) => self.reconcile_deleted(service_account).await,
        }
    }
}

impl ServiceAccountTokenReconciler {
    async fn reconcile_applied(
        &self,
        service_account: ServiceAccount,
    ) -> Result<Action, ControllerError> {
        if service_account.deletion_timestamp().is_some() {
            return Ok(Action::await_change());
        }

        let namespace = service_account
            .namespace()
            .ok_or(ControllerError::MissingNamespace("ServiceAccount"))?
            .to_string();
        let name = service_account
            .name()
            .ok_or(ControllerError::MissingName("ServiceAccount"))?
            .to_string();

        let secret_api: Api<Secret> = Api::namespaced(self.client.clone(), &namespace);
        let secrets = secret_api.list().await?;
        let owned_secrets: Vec<Secret> = secrets
            .into_iter()
            .filter(|secret| owned_by_service_account(secret, &name))
            .collect();

        let token_secret = match owned_secrets.into_iter().next() {
            Some(secret) => self.ensure_token_secret(secret_api.clone(), secret).await?,
            None => self.create_token_secret(secret_api.clone(), &namespace, &name).await?,
        };

        self.ensure_secret_reference(service_account, &token_secret)
            .await?;
        Ok(Action::await_change())
    }

    async fn reconcile_deleted(
        &self,
        service_account: ServiceAccount,
    ) -> Result<Action, ControllerError> {
        let Some(namespace) = service_account.namespace().map(str::to_string) else {
            return Ok(Action::await_change());
        };
        let Some(name) = service_account.name().map(str::to_string) else {
            return Ok(Action::await_change());
        };

        let secret_api: Api<Secret> = Api::namespaced(self.client.clone(), &namespace);
        for secret in secret_api.list().await? {
            if !owned_by_service_account(&secret, &name) {
                continue;
            }
            if let Some(secret_name) = secret.name() {
                delete_secret_ignore_not_found(&secret_api, secret_name).await?;
            }
        }

        Ok(Action::await_change())
    }

    async fn ensure_token_secret(
        &self,
        secret_api: Api<Secret>,
        mut secret: Secret,
    ) -> Result<Secret, ControllerError> {
        let mut changed = false;

        if secret.r#type != SERVICE_ACCOUNT_TOKEN_SECRET_TYPE {
            secret.r#type = SERVICE_ACCOUNT_TOKEN_SECRET_TYPE.to_string();
            changed = true;
        }
        if !secret.data.contains_key(TOKEN_DATA_KEY) {
            secret
                .string_data
                .insert(TOKEN_DATA_KEY.to_string(), generate_token());
            changed = true;
        }

        if !changed {
            return Ok(secret);
        }

        let name = secret
            .name()
            .ok_or(ControllerError::MissingName("Secret"))?
            .to_string();
        secret_api.replace(&name, secret).await.map_err(Into::into)
    }

    async fn create_token_secret(
        &self,
        secret_api: Api<Secret>,
        namespace: &str,
        service_account_name: &str,
    ) -> Result<Secret, ControllerError> {
        let secret_name = format!("{service_account_name}-token-{}", generate_suffix());
        let secret = Secret {
            type_meta: Some(Secret::type_meta()),
            object_meta: Some(ObjectMeta {
                name: Some(secret_name),
                namespace: Some(namespace.to_string()),
                annotations: HashMap::from([(
                    SERVICE_ACCOUNT_NAME_ANNOTATION.to_string(),
                    service_account_name.to_string(),
                )]),
                ..Default::default()
            }),
            string_data: HashMap::from([(TOKEN_DATA_KEY.to_string(), generate_token())]),
            r#type: SERVICE_ACCOUNT_TOKEN_SECRET_TYPE.to_string(),
            ..Default::default()
        };

        secret_api.create(secret).await.map_err(Into::into)
    }

    async fn ensure_secret_reference(
        &self,
        service_account: ServiceAccount,
        secret: &Secret,
    ) -> Result<(), ControllerError> {
        let namespace = service_account
            .namespace()
            .ok_or(ControllerError::MissingNamespace("ServiceAccount"))?
            .to_string();
        let service_account_name = service_account
            .name()
            .ok_or(ControllerError::MissingName("ServiceAccount"))?
            .to_string();
        let secret_name = secret
            .name()
            .ok_or(ControllerError::MissingName("Secret"))?
            .to_string();

        if service_account.secrets.iter().any(|reference| {
            reference.kind == "Secret"
                && reference.name == secret_name
                && reference.namespace.as_deref() == Some(namespace.as_str())
        }) {
            return Ok(());
        }

        let service_account_api: Api<ServiceAccount> = Api::namespaced(self.client.clone(), &namespace);
        let Some(mut latest) = service_account_api.get(&service_account_name).await? else {
            return Ok(());
        };

        if latest.secrets.iter().any(|reference| {
            reference.kind == "Secret"
                && reference.name == secret_name
                && reference.namespace.as_deref() == Some(namespace.as_str())
        }) {
            return Ok(());
        }

        latest.secrets.push(ObjectReference {
            kind: "Secret".to_string(),
            namespace: Some(namespace),
            name: secret_name,
            uid: secret
                .object_meta()
                .as_ref()
                .and_then(|meta| meta.uid.clone())
                .unwrap_or_default(),
            api_version: "v1".to_string(),
        });
        service_account_api.replace(&service_account_name, latest).await?;
        Ok(())
    }
}

fn owned_by_service_account(secret: &Secret, service_account_name: &str) -> bool {
    secret
        .object_meta()
        .as_ref()
        .and_then(|meta| meta.annotations.get(SERVICE_ACCOUNT_NAME_ANNOTATION))
        .is_some_and(|name| name == service_account_name)
}

async fn delete_secret_ignore_not_found(
    secret_api: &Api<Secret>,
    name: &str,
) -> Result<(), ControllerError> {
    match secret_api.delete(name).await {
        Ok(_) => Ok(()),
        Err(tugboat_client::Error::Api(status)) if status.code == 404 => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn generate_suffix() -> String {
    Alphanumeric.sample_string(&mut rng(), 5).to_ascii_lowercase()
}

fn generate_token() -> String {
    Alphanumeric.sample_string(&mut rng(), 32)
}

#[cfg(test)]
mod tests {
    use super::{
        SERVICE_ACCOUNT_NAME_ANNOTATION, SERVICE_ACCOUNT_TOKEN_SECRET_TYPE, TOKEN_DATA_KEY,
        generate_suffix, generate_token, owned_by_service_account,
    };
    use std::collections::HashMap;
    use tugboat_resources::manifests::core::v1::Secret;
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    #[test]
    fn matches_owned_secret_from_annotation() {
        let secret = Secret {
            object_meta: Some(ObjectMeta {
                annotations: HashMap::from([(
                    SERVICE_ACCOUNT_NAME_ANNOTATION.to_string(),
                    "builder".to_string(),
                )]),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert!(owned_by_service_account(&secret, "builder"));
        assert!(!owned_by_service_account(&secret, "default"));
    }

    #[test]
    fn generates_expected_token_material() {
        let token = generate_token();
        let suffix = generate_suffix();

        assert_eq!(token.len(), 32);
        assert_eq!(suffix.len(), 5);
        assert!(suffix.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }

    #[test]
    fn secret_type_and_token_key_constants_match_task_contract() {
        assert_eq!(SERVICE_ACCOUNT_TOKEN_SECRET_TYPE, "tugboat.io/service-account-token");
        assert_eq!(TOKEN_DATA_KEY, "token");
    }
}
