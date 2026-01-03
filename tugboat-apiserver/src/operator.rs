use crate::name_generator::NameGenerator;
use tugboat_resource_store::ResourceStore;
use tugboat_resources::manifests::meta::v1::ObjectMeta;
use uuid::Uuid;

pub(crate) struct ApiOperator {
    pub(crate) store: ResourceStore,
    pub(crate) name_generator: NameGenerator,
}

impl ApiOperator {
    pub(crate) fn new(store: ResourceStore) -> Self {
        Self {
            store,
            name_generator: NameGenerator::new(),
        }
    }

    pub(crate) fn apply_namespace(
        &self,
        mut object_meta: ObjectMeta,
        namespace: String,
    ) -> ObjectMeta {
        object_meta.namespace = Some(namespace);
        object_meta
    }

    pub(crate) fn apply_uid(&self, mut object_meta: ObjectMeta) -> ObjectMeta {
        object_meta.uid = Some(Uuid::new_v4().to_string());
        object_meta
    }
}
