use crate::proto::csi::v1::node_client::NodeClient;
use crate::proto::csi::v1::volume_capability::access_mode::Mode;
use crate::proto::csi::v1::volume_capability::{AccessMode, AccessType, BlockVolume};
use crate::proto::csi::v1::{
    NodePublishVolumeRequest, NodeUnpublishVolumeRequest, VolumeCapability,
};
pub use error::Error;
use hyper_util::rt::TokioIo;
use std::io;
use tokio::net::UnixStream;
use tonic::Code;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

mod error;
mod proto;

#[derive(Clone, Default)]
pub struct TugboatCsiOperator {}

pub enum CsiAccessMode {
    ReadOnlyMany,
    ReadWriteOnce,
    ReadWriteMany,
}

pub enum CsiAccessType {
    Block,
    // Filesystem,
}

impl TugboatCsiOperator {
    pub async fn publish(
        &self,
        socket_path: &str,
        volume_id: String,
        target_path: String,
        read_only: bool,
        access_mode: CsiAccessMode,
        access_type: CsiAccessType,
    ) -> Result<(), error::Error> {
        let req = NodePublishVolumeRequest {
            volume_id,
            target_path,
            volume_capability: Some(VolumeCapability {
                access_mode: Some(AccessMode {
                    mode: match access_mode {
                        CsiAccessMode::ReadOnlyMany => Mode::SingleNodeReaderOnly,
                        CsiAccessMode::ReadWriteOnce => Mode::SingleNodeWriter,
                        CsiAccessMode::ReadWriteMany => Mode::MultiNodeMultiWriter,
                    } as i32,
                }),
                access_type: Some(match access_type {
                    CsiAccessType::Block => AccessType::Block(BlockVolume {}),
                }),
            }),
            readonly: read_only,
            secrets: Default::default(),
            volume_context: Default::default(),
            publish_context: Default::default(),
            staging_target_path: "".to_string(),
        };

        let mut client = connect_node_client(socket_path).await?;
        client
            .node_publish_volume(req)
            .await
            .map(|_| ())
            .map_err(map_grpc_error)
    }

    pub async fn unpublish(
        &self,
        socket_path: &str,
        volume_id: String,
        target_path: String,
    ) -> Result<(), error::Error> {
        let req = NodeUnpublishVolumeRequest {
            volume_id,
            target_path,
        };

        let mut client = connect_node_client(socket_path).await?;
        client
            .node_unpublish_volume(req)
            .await
            .map(|_| ())
            .map_err(map_grpc_error)
    }
}

async fn connect_node_client(socket_path: &str) -> Result<NodeClient<Channel>, error::Error> {
    let socket_path = normalize_socket_path(socket_path);
    let channel = Endpoint::try_from("http://[::]:50051")
        .expect("static tonic endpoint should be valid")
        .connect_with_connector(service_fn(move |_: Uri| {
            let socket_path = socket_path.clone();
            async move {
                let stream = UnixStream::connect(socket_path).await?;
                Ok::<_, io::Error>(TokioIo::new(stream))
            }
        }))
        .await?;
    Ok(NodeClient::new(channel))
}

fn normalize_socket_path(socket_path: &str) -> String {
    socket_path
        .strip_prefix("unix://")
        .unwrap_or(socket_path)
        .to_string()
}

fn map_grpc_error(error: tonic::Status) -> error::Error {
    match error.code() {
        Code::Ok => unreachable!("successful responses are not routed through map_grpc_error"),
        Code::AlreadyExists => error::Error::TargetPathAlreadyExists,
        Code::NotFound => error::Error::TargetPathNotFound,
        Code::FailedPrecondition => error::Error::FailedPrecondition,
        _ => error::Error::Grpc(error),
    }
}
