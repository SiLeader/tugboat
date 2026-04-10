use crate::base::TugboatController;
use crate::error::ControllerError;
use rand::distr::{Alphanumeric, SampleString};
use rand::rng;
use std::collections::{BTreeSet, HashMap};
use tugboat_client::runtime::{Action, Controller, ReconcileEvent, Reconciler};
use tugboat_client::{Api, TugboatClient};
use tugboat_resources::manifests::core::v1::{Secret, ServiceAccount};
use tugboat_resources::manifests::meta::v1::{ObjectMeta, ObjectReference};
use tugboat_resources::{
    ObjectMetaResource, Resource, SERVICE_ACCOUNT_NAME_ANNOTATION,
    SERVICE_ACCOUNT_TOKEN_SECRET_TYPE,
};
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
            ReconcileEvent::Applied(service_account) => {
                self.reconcile_applied(service_account).await
            }
            ReconcileEvent::Deleted(service_account) => {
                self.reconcile_deleted(service_account).await
            }
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
            .filter(|secret| managed_token_secret_for_service_account(secret, &name))
            .collect();

        let (authoritative_secret, redundant_secrets) =
            split_authoritative_secret(&service_account, owned_secrets);
        let token_secret = match authoritative_secret {
            Some(secret) => self.ensure_token_secret(secret_api.clone(), secret).await?,
            None => {
                self.create_token_secret(secret_api.clone(), &namespace, &name)
                    .await?
            }
        };
        let redundant_secret_names = secret_names(&redundant_secrets);
        for secret_name in &redundant_secret_names {
            delete_secret_ignore_not_found(&secret_api, secret_name).await?;
        }

        self.ensure_secret_reference(&service_account, &token_secret, &redundant_secret_names)
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
            if !managed_token_secret_for_service_account(&secret, &name) {
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
        service_account: &ServiceAccount,
        secret: &Secret,
        redundant_secret_names: &BTreeSet<String>,
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
        let secret_uid = secret
            .object_meta()
            .as_ref()
            .and_then(|meta| meta.uid.clone())
            .unwrap_or_default();

        let service_account_api: Api<ServiceAccount> =
            Api::namespaced(self.client.clone(), &namespace);
        let Some(mut latest) = service_account_api.get(&service_account_name).await? else {
            return Ok(());
        };

        if !reconcile_token_secret_references(
            &mut latest.secrets,
            &namespace,
            &secret_name,
            &secret_uid,
            redundant_secret_names,
        ) {
            return Ok(());
        }

        service_account_api
            .replace(&service_account_name, latest)
            .await?;
        Ok(())
    }
}

fn split_authoritative_secret(
    service_account: &ServiceAccount,
    owned_secrets: Vec<Secret>,
) -> (Option<Secret>, Vec<Secret>) {
    let authoritative_name = choose_authoritative_secret_name(service_account, &owned_secrets);
    let mut authoritative = None;
    let mut redundant = Vec::new();

    for secret in owned_secrets {
        let is_authoritative = authoritative_name
            .as_deref()
            .is_some_and(|name| secret.name() == Some(name));
        if authoritative.is_none() && is_authoritative {
            authoritative = Some(secret);
        } else {
            redundant.push(secret);
        }
    }

    (authoritative, redundant)
}

fn choose_authoritative_secret_name(
    service_account: &ServiceAccount,
    owned_secrets: &[Secret],
) -> Option<String> {
    let owned_secret_names = secret_names(owned_secrets);
    if owned_secret_names.is_empty() {
        return None;
    }

    service_account
        .secrets
        .iter()
        .filter(|reference| {
            reference.kind == "Secret"
                && reference.api_version == "v1"
                && reference.namespace.as_deref() == service_account.namespace()
                && owned_secret_names.contains(&reference.name)
        })
        .map(|reference| reference.name.clone())
        .min()
}

fn secret_names(secrets: &[Secret]) -> BTreeSet<String> {
    secrets
        .iter()
        .filter_map(|secret| secret.name().map(str::to_string))
        .collect()
}

fn managed_token_secret_for_service_account(secret: &Secret, service_account_name: &str) -> bool {
    secret.r#type == SERVICE_ACCOUNT_TOKEN_SECRET_TYPE
        && secret
            .object_meta()
            .as_ref()
            .and_then(|meta| meta.annotations.get(SERVICE_ACCOUNT_NAME_ANNOTATION))
            .is_some_and(|name| name == service_account_name)
}

fn reconcile_token_secret_references(
    references: &mut Vec<ObjectReference>,
    namespace: &str,
    secret_name: &str,
    secret_uid: &str,
    redundant_secret_names: &BTreeSet<String>,
) -> bool {
    let mut changed = false;
    let mut kept_authoritative = false;

    references.retain(|reference| {
        if reference.kind != "Secret" || reference.namespace.as_deref() != Some(namespace) {
            return true;
        }
        if reference.name == secret_name {
            if kept_authoritative {
                changed = true;
                return false;
            }
            kept_authoritative = true;
            return true;
        }
        if redundant_secret_names.contains(&reference.name) {
            changed = true;
            return false;
        }
        true
    });

    if let Some(reference) = references.iter_mut().find(|reference| {
        reference.kind == "Secret"
            && reference.name == secret_name
            && reference.namespace.as_deref() == Some(namespace)
    }) {
        if reference.api_version != "v1" {
            reference.api_version = "v1".to_string();
            changed = true;
        }
        if reference.uid != secret_uid {
            reference.uid = secret_uid.to_string();
            changed = true;
        }
    } else {
        references.push(ObjectReference {
            kind: "Secret".to_string(),
            namespace: Some(namespace.to_string()),
            name: secret_name.to_string(),
            uid: secret_uid.to_string(),
            api_version: "v1".to_string(),
        });
        changed = true;
    }

    changed
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
    Alphanumeric
        .sample_string(&mut rng(), 5)
        .to_ascii_lowercase()
}

fn generate_token() -> String {
    Alphanumeric.sample_string(&mut rng(), 32)
}

#[cfg(test)]
mod tests {
    use super::{
        SERVICE_ACCOUNT_NAME_ANNOTATION, SERVICE_ACCOUNT_TOKEN_SECRET_TYPE, TOKEN_DATA_KEY,
        choose_authoritative_secret_name, generate_suffix, generate_token,
        managed_token_secret_for_service_account, reconcile_token_secret_references,
        split_authoritative_secret,
    };
    use std::collections::{BTreeSet, HashMap};
    use tugboat_resources::ObjectMetaResource;
    use tugboat_resources::manifests::core::v1::{Secret, ServiceAccount};
    use tugboat_resources::manifests::meta::v1::{ObjectMeta, ObjectReference};

    #[test]
    fn matches_managed_token_secret_from_annotation_and_type() {
        let secret = Secret {
            object_meta: Some(ObjectMeta {
                annotations: HashMap::from([(
                    SERVICE_ACCOUNT_NAME_ANNOTATION.to_string(),
                    "builder".to_string(),
                )]),
                ..Default::default()
            }),
            r#type: SERVICE_ACCOUNT_TOKEN_SECRET_TYPE.to_string(),
            ..Default::default()
        };

        assert!(managed_token_secret_for_service_account(&secret, "builder"));
        assert!(!managed_token_secret_for_service_account(
            &secret, "default"
        ));
        assert!(!managed_token_secret_for_service_account(
            &Secret::default(),
            "builder"
        ));
    }

    #[test]
    fn generates_expected_token_material() {
        let token = generate_token();
        let suffix = generate_suffix();

        assert_eq!(token.len(), 32);
        assert_eq!(suffix.len(), 5);
        assert!(
            suffix
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
    }

    #[test]
    fn secret_type_and_token_key_constants_match_task_contract() {
        assert_eq!(
            SERVICE_ACCOUNT_TOKEN_SECRET_TYPE,
            "tugboat.io/service-account-token"
        );
        assert_eq!(TOKEN_DATA_KEY, "token");
    }

    #[test]
    fn prefers_referenced_owned_secret_as_authoritative() {
        let service_account = ServiceAccount {
            object_meta: Some(ObjectMeta {
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            secrets: vec![ObjectReference {
                kind: "Secret".to_string(),
                namespace: Some("default".to_string()),
                name: "builder-token-b".to_string(),
                uid: String::new(),
                api_version: "v1".to_string(),
            }],
            ..Default::default()
        };
        let owned_secrets = vec![
            named_secret("builder-token-a"),
            named_secret("builder-token-b"),
            named_secret("builder-token-c"),
        ];

        let chosen =
            choose_authoritative_secret_name(&service_account, &owned_secrets).expect("secret");
        let (authoritative, redundant) =
            split_authoritative_secret(&service_account, owned_secrets);

        assert_eq!(chosen, "builder-token-b");
        assert_eq!(
            authoritative.and_then(|secret| secret.name().map(str::to_string)),
            Some(chosen)
        );
        assert_eq!(
            redundant
                .iter()
                .filter_map(|secret| secret.name().map(str::to_string))
                .collect::<Vec<_>>(),
            vec!["builder-token-a".to_string(), "builder-token-c".to_string()]
        );
    }

    #[test]
    fn does_not_adopt_unreferenced_managed_secret() {
        let service_account = ServiceAccount {
            object_meta: Some(ObjectMeta {
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let owned_secrets = vec![
            named_secret("builder-token-z"),
            named_secret("builder-token-a"),
        ];

        assert_eq!(
            choose_authoritative_secret_name(&service_account, &owned_secrets),
            None
        );
        let (authoritative, redundant) =
            split_authoritative_secret(&service_account, owned_secrets);
        assert!(authoritative.is_none());
        assert_eq!(
            redundant
                .iter()
                .filter_map(|secret| secret.name().map(str::to_string))
                .collect::<Vec<_>>(),
            vec!["builder-token-z".to_string(), "builder-token-a".to_string()]
        );
    }

    #[test]
    fn reconciles_secret_references_to_single_authoritative_entry() {
        let mut references = vec![
            ObjectReference {
                kind: "Secret".to_string(),
                namespace: Some("default".to_string()),
                name: "builder-token-b".to_string(),
                uid: "old-uid".to_string(),
                api_version: "v1beta1".to_string(),
            },
            ObjectReference {
                kind: "Secret".to_string(),
                namespace: Some("default".to_string()),
                name: "builder-token-a".to_string(),
                uid: String::new(),
                api_version: "v1".to_string(),
            },
            ObjectReference {
                kind: "Secret".to_string(),
                namespace: Some("default".to_string()),
                name: "config-secret".to_string(),
                uid: "config".to_string(),
                api_version: "v1".to_string(),
            },
        ];

        let changed = reconcile_token_secret_references(
            &mut references,
            "default",
            "builder-token-b",
            "new-uid",
            &BTreeSet::from(["builder-token-a".to_string()]),
        );

        assert!(changed);
        assert_eq!(references.len(), 2);
        assert!(references.iter().any(|reference| {
            reference.name == "builder-token-b"
                && reference.uid == "new-uid"
                && reference.api_version == "v1"
        }));
        assert!(
            references
                .iter()
                .any(|reference| reference.name == "config-secret")
        );
        assert!(
            !references
                .iter()
                .any(|reference| reference.name == "builder-token-a")
        );
    }

    fn named_secret(name: &str) -> Secret {
        Secret {
            object_meta: Some(ObjectMeta {
                name: Some(name.to_string()),
                ..Default::default()
            }),
            r#type: SERVICE_ACCOUNT_TOKEN_SECRET_TYPE.to_string(),
            ..Default::default()
        }
    }
}
