use crate::proto::csi::v1::controller_client::ControllerClient;
use crate::proto::csi::v1::node_client::NodeClient;
use crate::proto::csi::v1::node_service_capability;
use crate::proto::csi::v1::node_service_capability::rpc::Type as NodeServiceCapabilityType;
use crate::proto::csi::v1::volume_capability::access_mode::Mode;
use crate::proto::csi::v1::volume_capability::{AccessMode, AccessType, BlockVolume};
use crate::proto::csi::v1::{
    CreateVolumeRequest, DeleteVolumeRequest, NodeGetCapabilitiesRequest, NodePublishVolumeRequest,
    NodeUnpublishVolumeRequest, VolumeCapability,
};
pub use error::Error;
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::io;
use tokio::net::UnixStream;
use tonic::Code;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

mod error;
mod proto;

#[derive(Clone, Default)]
pub struct TugboatCsiOperator {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsiAccessMode {
    ReadOnlyMany,
    ReadWriteOnce,
    ReadWriteMany,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsiAccessType {
    Block,
    // Filesystem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeCapability {
    StageUnstageVolume,
    GetVolumeStats,
    ExpandVolume,
    VolumeCondition,
    SingleNodeMultiWriter,
    VolumeMountGroup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionedVolume {
    pub volume_id: String,
    pub capacity_bytes: i64,
    pub volume_context: HashMap<String, String>,
}

impl TugboatCsiOperator {
    pub async fn create_volume(
        &self,
        socket_path: &str,
        name: String,
        parameters: HashMap<String, String>,
        access_modes: Vec<CsiAccessMode>,
        access_type: CsiAccessType,
    ) -> Result<ProvisionedVolume, error::Error> {
        let req = CreateVolumeRequest {
            name,
            capacity_range: None,
            volume_capabilities: access_modes
                .into_iter()
                .map(|access_mode| VolumeCapability {
                    access_mode: Some(AccessMode {
                        mode: match access_mode {
                            CsiAccessMode::ReadOnlyMany => Mode::MultiNodeReaderOnly,
                            CsiAccessMode::ReadWriteOnce => Mode::SingleNodeWriter,
                            CsiAccessMode::ReadWriteMany => Mode::MultiNodeMultiWriter,
                        } as i32,
                    }),
                    access_type: Some(match access_type {
                        CsiAccessType::Block => AccessType::Block(BlockVolume {}),
                    }),
                })
                .collect(),
            parameters,
            secrets: Default::default(),
            volume_content_source: None,
            accessibility_requirements: None,
            mutable_parameters: Default::default(),
        };

        let mut client = connect_controller_client(socket_path).await?;
        let response = client
            .create_volume(req)
            .await
            .map_err(map_controller_grpc_error)?
            .into_inner();
        let volume = response.volume.ok_or(error::Error::MissingVolume)?;

        Ok(ProvisionedVolume {
            volume_id: volume.volume_id,
            capacity_bytes: volume.capacity_bytes,
            volume_context: volume.volume_context,
        })
    }

    pub async fn delete_volume(
        &self,
        socket_path: &str,
        volume_id: String,
    ) -> Result<(), error::Error> {
        let req = DeleteVolumeRequest {
            volume_id,
            secrets: Default::default(),
        };

        let mut client = connect_controller_client(socket_path).await?;
        client
            .delete_volume(req)
            .await
            .map(|_| ())
            .map_err(map_controller_grpc_error)
    }

    pub async fn node_capabilities(
        &self,
        socket_path: &str,
    ) -> Result<Vec<NodeCapability>, error::Error> {
        let mut client = connect_node_client(socket_path).await?;
        let response = client
            .node_get_capabilities(NodeGetCapabilitiesRequest {})
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        Ok(response
            .capabilities
            .into_iter()
            .filter_map(|capability| match capability.r#type {
                Some(node_service_capability::Type::Rpc(rpc)) => {
                    match NodeServiceCapabilityType::try_from(rpc.r#type).ok()? {
                        NodeServiceCapabilityType::Unknown => None,
                        NodeServiceCapabilityType::StageUnstageVolume => {
                            Some(NodeCapability::StageUnstageVolume)
                        }
                        NodeServiceCapabilityType::GetVolumeStats => {
                            Some(NodeCapability::GetVolumeStats)
                        }
                        NodeServiceCapabilityType::ExpandVolume => {
                            Some(NodeCapability::ExpandVolume)
                        }
                        NodeServiceCapabilityType::VolumeCondition => {
                            Some(NodeCapability::VolumeCondition)
                        }
                        NodeServiceCapabilityType::SingleNodeMultiWriter => {
                            Some(NodeCapability::SingleNodeMultiWriter)
                        }
                        NodeServiceCapabilityType::VolumeMountGroup => {
                            Some(NodeCapability::VolumeMountGroup)
                        }
                    }
                }
                None => None,
            })
            .collect())
    }

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
                        CsiAccessMode::ReadOnlyMany => Mode::MultiNodeReaderOnly,
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
    Ok(NodeClient::new(connect_channel(socket_path).await?))
}

async fn connect_controller_client(
    socket_path: &str,
) -> Result<ControllerClient<Channel>, error::Error> {
    Ok(ControllerClient::new(connect_channel(socket_path).await?))
}

async fn connect_channel(socket_path: &str) -> Result<Channel, error::Error> {
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
    Ok(channel)
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

fn map_controller_grpc_error(error: tonic::Status) -> error::Error {
    match error.code() {
        Code::Ok => unreachable!("successful responses are not routed through map_grpc_error"),
        Code::AlreadyExists => error::Error::VolumeAlreadyExists,
        Code::NotFound => error::Error::VolumeNotFound,
        Code::FailedPrecondition => error::Error::FailedPrecondition,
        _ => error::Error::Grpc(error),
    }
}
