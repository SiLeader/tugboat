use crate::ObjectMetaResource;
use crate::validators::Validator;

pub struct NamespaceRequiredValidator;
pub struct NamespaceProhibitedValidator;

impl<T> Validator<T> for NamespaceRequiredValidator
where
    T: ObjectMetaResource,
{
    fn validate(&self, value: &T) -> bool {
        let Some(meta) = value.object_meta() else {
            return false;
        };
        meta.namespace.is_some()
    }
}

impl<T> Validator<T> for NamespaceProhibitedValidator
where
    T: ObjectMetaResource,
{
    fn validate(&self, value: &T) -> bool {
        let Some(meta) = value.object_meta() else {
            return true;
        };
        meta.namespace.is_none()
    }
}
