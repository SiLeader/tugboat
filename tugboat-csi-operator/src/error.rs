use thiserror::Error;

use crate::ControllerCapability;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Grpc transport error: {0}")]
    GrpcTransport(#[from] tonic::transport::Error),
    #[error("Timed out connecting to CSI socket")]
    SocketConnectionTimeout,
    #[error("Invalid CSI socket path: {0}")]
    InvalidSocketPath(String),
    #[error("Invalid CSI volume name: {0}")]
    InvalidVolumeName(String),
    #[error("Invalid CSI snapshot name: {0}")]
    InvalidSnapshotName(String),
    #[error("Invalid CSI requested capacity bytes: {0}")]
    InvalidCapacityBytes(i64),
    #[error("CSI access mode list cannot be empty")]
    MissingAccessModes,
    #[error("Timed out waiting for CSI RPC response")]
    RpcTimeout,
    #[error("Target path already exists")]
    TargetPathAlreadyExists,
    #[error("Target path not found")]
    TargetPathNotFound,
    #[error("Volume already exists")]
    VolumeAlreadyExists,
    #[error("Volume not found")]
    VolumeNotFound,
    #[error("Snapshot already exists")]
    SnapshotAlreadyExists,
    #[error("Snapshot not found")]
    SnapshotNotFound,
    #[error("CreateVolume response is missing volume details")]
    MissingVolume,
    #[error("CSI response is missing volume ID")]
    MissingVolumeId,
    #[error("CreateSnapshot response is missing snapshot details")]
    MissingSnapshot,
    #[error("CSI response is missing snapshot ID")]
    MissingSnapshotId,
    #[error("CSI controller capability is not supported: {0:?}")]
    UnsupportedControllerCapability(ControllerCapability),
    #[error("Failed precondition")]
    FailedPrecondition,
    #[error("Grpc error: {0}")]
    Grpc(tonic::Status),
}
