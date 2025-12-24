#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Unsupported type")]
    UnsupportedType,
    #[error("Protobuf serialization error: {0}")]
    ProtobufDeserialization(#[from] prost::DecodeError),
    #[error("Object meta missing")]
    ObjectMetaMissing,
    #[error("Etcd error: {0}")]
    Etcd(#[from] etcd_client::Error),
}
