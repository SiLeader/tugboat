use crate::data::{CreateResponse, StatusResponse};
use crate::endpoints::v1_core::namespace_type_meta;
use crate::operator::ApiOperator;
use actix_web::post;
use actix_web::web::{Data, Json};
use tugboat_resources::manifests::core::v1::Namespace;
use tugboat_resources::manifests::meta::v1::ObjectMeta;
use uuid::Uuid;

#[utoipa::path()]
#[post("/v1/namespaces")]
pub(super) async fn handle_namespace_create(
    json: Json<Namespace>,
    operator: Data<ApiOperator>,
) -> Result<CreateResponse<Namespace>, StatusResponse> {
    let namespace = json.into_inner();

    let Some(object_meta) = namespace.object_meta.clone() else {
        return Err(StatusResponse::bad_request("metadata is required", None));
    };
    if object_meta.namespace.is_some() {
        return Err(StatusResponse::bad_request(
            "metadata.namespace cannot be set",
            None,
        ));
    }

    let object_meta = ObjectMeta {
        namespace: None,
        uid: Some(Uuid::new_v4().to_string()),
        ..object_meta
    };

    for _ in 0..5 {
        let name = match object_meta.name.clone() {
            None => match &object_meta.generate_name {
                None => {
                    return Err(StatusResponse::bad_request(
                        "metadata.name or metadata.generateName is required",
                        None,
                    ));
                }
                Some(base_name) => operator.name_generator.generate(&base_name).await,
            },
            Some(name) => name,
        };

        let namespace = {
            let mut ns = namespace.clone();
            ns.type_meta = Some(namespace_type_meta());
            ns.object_meta = Some(ObjectMeta {
                name: Some(name),
                ..object_meta.clone()
            });
            ns
        };
        if let Some(data) = operator.store.put_if_not_exists(namespace.clone()).await? {
            return Ok(CreateResponse::Created(data.apply_revision()));
        }
    }
    Err(StatusResponse::conflict("Generate name failed", None))
}
