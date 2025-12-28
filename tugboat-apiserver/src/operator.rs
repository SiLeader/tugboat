use crate::name_generator::NameGenerator;
use tugboat_resource_store::ResourceStore;

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
}
