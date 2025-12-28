use crate::endpoints::v1_core::namespace_create::handle_namespace_create;
use crate::endpoints::v1_core::namespace_list::handle_namespace_list;
use crate::endpoints::v1_core::namespace_read::handle_namespace_read;
use tugboat_resources::manifests::meta::v1::TypeMeta;
use utoipa_actix_web::service_config::ServiceConfig;

mod namespace_create;
mod namespace_list;
mod namespace_read;

pub(super) fn register_v1_core(service: &mut ServiceConfig) {
    service
        .service(handle_namespace_create)
        .service(handle_namespace_list)
        .service(handle_namespace_read);
}

fn namespace_type_meta() -> TypeMeta {
    TypeMeta {
        kind: Some("Namespace".to_string()),
        api_version: Some("v1".to_string()),
    }
}
