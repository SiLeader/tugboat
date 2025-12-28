use crate::error::Error;
use crate::serializer::Serializer;
use tugboat_resources::manifests::meta::v1::TypeMeta;

#[derive(Default)]
pub struct ProtobufSerializer;

impl Serializer for ProtobufSerializer {
    fn is_supported(&self, type_meta: &TypeMeta) -> bool {
        !type_meta
            .api_version
            .as_ref()
            .is_some_and(|v| v.contains("/")) // core
    }

    fn serialize_protobuf<T: prost::Message>(
        &self,
        type_meta: &TypeMeta,
        value: &T,
    ) -> Result<Vec<u8>, Error> {
        if !self.is_supported(type_meta) {
            return Err(Error::UnsupportedType);
        }
        Ok(value.encode_to_vec())
    }

    fn deserialize_protobuf<T>(&self, type_meta: &TypeMeta, data: &[u8]) -> Result<T, Error>
    where
        T: prost::Message + Default,
    {
        if !self.is_supported(type_meta) {
            return Err(Error::UnsupportedType);
        }
        Ok(T::decode(data)?)
    }
}
