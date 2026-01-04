use crate::manifests::meta::v1::{ObjectMeta, TypeMeta};

pub mod manifests;
#[cfg(feature = "validators")]
pub mod validators;

pub trait Resource {
    fn type_meta() -> TypeMeta;
}

pub trait StaticResource: Resource {
    fn group() -> &'static str;
    fn version() -> &'static str;
    fn kind() -> &'static str;
    fn plural() -> &'static str;
    fn singular() -> &'static str;
    fn is_cluster_scoped() -> bool;
}

pub trait ClusterScopedResource: StaticResource {}

pub trait NamespacedResource: StaticResource {}

pub trait ObjectMetaResource: Resource {
    fn object_meta(&self) -> &Option<ObjectMeta>;
    fn object_meta_mut(&mut self) -> &mut Option<ObjectMeta>;
    fn modify_object_meta(&mut self, f: impl FnOnce(&mut Option<ObjectMeta>)) {
        f(self.object_meta_mut());
    }
}

impl<T: StaticResource> Resource for T {
    fn type_meta() -> TypeMeta {
        TypeMeta {
            api_version: Some(if Self::group() == "core" || Self::group().is_empty() {
                Self::version().to_string()
            } else {
                format!("{}/{}", Self::group(), Self::version())
            }),
            kind: Some(Self::kind().to_string()),
        }
    }
}

#[macro_export]
macro_rules! apply_resource {
    ($ty:ident, $group:literal, $version:literal, $plural:literal, $singular:literal, $cluster_scoped:literal) => {
        impl $crate::StaticResource for $ty {
            fn group() -> &'static str {
                $group
            }

            fn version() -> &'static str {
                $version
            }

            fn kind() -> &'static str {
                stringify!($ty)
            }

            fn plural() -> &'static str {
                $plural
            }

            fn singular() -> &'static str {
                $singular
            }

            fn is_cluster_scoped() -> bool {
                $cluster_scoped
            }
        }

        impl $crate::ObjectMetaResource for $ty {
            fn object_meta(&self) -> &Option<$crate::manifests::meta::v1::ObjectMeta> {
                &self.object_meta
            }

            fn object_meta_mut(&mut self) -> &mut Option<$crate::manifests::meta::v1::ObjectMeta> {
                &mut self.object_meta
            }
        }
    };

    ($ty:ident, $group:literal, $version:literal, $plural:literal, $singular:literal, namespaced) => {
        apply_resource!($ty, $group, $version, $plural, $singular, false);

        impl $crate::NamespacedResource for $ty {}
    };

    ($ty:ident, $group:literal, $version:literal, $plural:literal, $singular:literal, cluster) => {
        apply_resource!($ty, $group, $version, $plural, $singular, true);

        impl $crate::ClusterScopedResource for $ty {}
    };
}
