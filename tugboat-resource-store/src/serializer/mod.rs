mod protobuf;

use crate::error::Error;
use crate::serializer::protobuf::ProtobufSerializer;
use tugboat_resources::manifests::core::v1::{Namespace, Node, Ship, ShipClass};
use tugboat_resources::manifests::meta::v1::TypeMeta;
use tugboat_resources::{Resource, StaticResource};

pub trait Serializable: Resource + Sized {
    fn serialize(&self) -> Result<Vec<u8>, Error>;
    fn deserialize(data: &[u8]) -> Result<Self, Error>;
}

pub trait StaticSerializable: Serializable + StaticResource {}

pub trait Serializer {
    fn is_supported(&self, type_meta: &TypeMeta) -> bool;

    fn serialize_protobuf<T: prost::Message>(
        &self,
        _type_meta: &TypeMeta,
        _value: &T,
    ) -> Result<Vec<u8>, Error> {
        Err(Error::UnsupportedType)
    }

    fn deserialize_protobuf<T>(&self, _type_meta: &TypeMeta, _data: &[u8]) -> Result<T, Error>
    where
        T: prost::Message + Default,
    {
        Err(Error::UnsupportedType)
    }
}

macro_rules! protobuf_serializable {
    ($ty:ident) => {
        impl Serializable for $ty {
            fn serialize(&self) -> Result<Vec<u8>, Error> {
                ProtobufSerializer.serialize_protobuf(&Self::type_meta(), self)
            }

            fn deserialize(data: &[u8]) -> Result<Self, Error> {
                ProtobufSerializer.deserialize_protobuf(&Self::type_meta(), data)
            }
        }

        impl StaticSerializable for $ty {}
    };
}

protobuf_serializable!(Namespace);
protobuf_serializable!(Node);
protobuf_serializable!(Ship);
protobuf_serializable!(ShipClass);
