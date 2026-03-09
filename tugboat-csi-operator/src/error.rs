use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Grpc transport error: {0}")]
    GrpcTransport(#[from] tonic::transport::Error),
    #[error("Target path already exists")]
    TargetPathAlreadyExists,
    #[error("Failed precondition")]
    FailedPrecondition,
    #[error("Grpc error: {0}")]
    Grpc(tonic::Status),
}
