use crate::proto::csi::v1::controller_client::ControllerClient;
use crate::proto::csi::v1::node_client::NodeClient;
use crate::proto::csi::v1::node_service_capability;
use crate::proto::csi::v1::node_service_capability::rpc::Type as NodeServiceCapabilityType;
use crate::proto::csi::v1::volume_capability::access_mode::Mode;
use crate::proto::csi::v1::volume_capability::{AccessMode, AccessType, BlockVolume, MountVolume};
use crate::proto::csi::v1::{
    CreateVolumeRequest, DeleteVolumeRequest, NodeGetCapabilitiesRequest, NodePublishVolumeRequest,
    NodeStageVolumeRequest, NodeUnpublishVolumeRequest, NodeUnstageVolumeRequest, VolumeCapability,
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
    Filesystem,
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
                .map(|access_mode| volume_capability(access_mode, access_type))
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
        staging_target_path: Option<String>,
        secrets: HashMap<String, String>,
        volume_context: HashMap<String, String>,
        publish_context: HashMap<String, String>,
    ) -> Result<(), error::Error> {
        let req = NodePublishVolumeRequest {
            volume_id,
            target_path,
            volume_capability: Some(volume_capability(access_mode, access_type)),
            readonly: read_only,
            secrets,
            volume_context,
            publish_context,
            staging_target_path: staging_target_path.unwrap_or_default(),
        };

        let mut client = connect_node_client(socket_path).await?;
        client
            .node_publish_volume(req)
            .await
            .map(|_| ())
            .map_err(map_grpc_error)
    }

    pub async fn stage(
        &self,
        socket_path: &str,
        volume_id: String,
        staging_target_path: String,
        access_mode: CsiAccessMode,
        access_type: CsiAccessType,
        secrets: HashMap<String, String>,
        volume_context: HashMap<String, String>,
        publish_context: HashMap<String, String>,
    ) -> Result<(), error::Error> {
        let req = NodeStageVolumeRequest {
            volume_id,
            publish_context,
            staging_target_path,
            volume_capability: Some(volume_capability(access_mode, access_type)),
            secrets,
            volume_context,
        };

        let mut client = connect_node_client(socket_path).await?;
        client
            .node_stage_volume(req)
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

    pub async fn unstage(
        &self,
        socket_path: &str,
        volume_id: String,
        staging_target_path: String,
    ) -> Result<(), error::Error> {
        let req = NodeUnstageVolumeRequest {
            volume_id,
            staging_target_path,
        };

        let mut client = connect_node_client(socket_path).await?;
        client
            .node_unstage_volume(req)
            .await
            .map(|_| ())
            .map_err(map_grpc_error)
    }
}

fn volume_capability(access_mode: CsiAccessMode, access_type: CsiAccessType) -> VolumeCapability {
    VolumeCapability {
        access_mode: Some(AccessMode {
            mode: match access_mode {
                CsiAccessMode::ReadOnlyMany => Mode::MultiNodeReaderOnly,
                CsiAccessMode::ReadWriteOnce => Mode::SingleNodeWriter,
                CsiAccessMode::ReadWriteMany => Mode::MultiNodeMultiWriter,
            } as i32,
        }),
        access_type: Some(match access_type {
            CsiAccessType::Block => AccessType::Block(BlockVolume {}),
            CsiAccessType::Filesystem => AccessType::Mount(MountVolume {
                fs_type: String::new(),
                mount_flags: Vec::new(),
                volume_mount_group: String::new(),
            }),
        }),
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

#[cfg(test)]
mod tests {
    use super::{CsiAccessMode, CsiAccessType, TugboatCsiOperator, volume_capability};
    use crate::proto::csi::v1::node_server::{Node, NodeServer};
    use crate::proto::csi::v1::volume_capability::AccessType;
    use crate::proto::csi::v1::{
        NodeExpandVolumeRequest, NodeExpandVolumeResponse, NodeGetCapabilitiesRequest,
        NodeGetCapabilitiesResponse, NodeGetInfoRequest, NodeGetInfoResponse,
        NodeGetVolumeStatsRequest, NodeGetVolumeStatsResponse, NodePublishVolumeRequest,
        NodePublishVolumeResponse, NodeStageVolumeRequest, NodeStageVolumeResponse,
        NodeUnpublishVolumeRequest, NodeUnpublishVolumeResponse, NodeUnstageVolumeRequest,
        NodeUnstageVolumeResponse,
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::net::UnixListener;
    use tokio::time::{Duration, sleep};
    use tokio_stream::wrappers::UnixListenerStream;
    use tonic::{Request, Response, Status};

    #[derive(Debug, Clone)]
    enum RecordedCall {
        Stage(NodeStageVolumeRequest),
        Publish(NodePublishVolumeRequest),
        Unpublish(NodeUnpublishVolumeRequest),
        Unstage(NodeUnstageVolumeRequest),
    }

    #[derive(Clone)]
    struct FakeNodeService {
        calls: Arc<Mutex<Vec<RecordedCall>>>,
    }

    #[tonic::async_trait]
    impl Node for FakeNodeService {
        async fn node_stage_volume(
            &self,
            request: Request<NodeStageVolumeRequest>,
        ) -> Result<Response<NodeStageVolumeResponse>, Status> {
            self.calls
                .lock()
                .expect("lock should be available")
                .push(RecordedCall::Stage(request.into_inner()));
            Ok(Response::new(NodeStageVolumeResponse {}))
        }

        async fn node_unstage_volume(
            &self,
            request: Request<NodeUnstageVolumeRequest>,
        ) -> Result<Response<NodeUnstageVolumeResponse>, Status> {
            self.calls
                .lock()
                .expect("lock should be available")
                .push(RecordedCall::Unstage(request.into_inner()));
            Ok(Response::new(NodeUnstageVolumeResponse {}))
        }

        async fn node_publish_volume(
            &self,
            request: Request<NodePublishVolumeRequest>,
        ) -> Result<Response<NodePublishVolumeResponse>, Status> {
            self.calls
                .lock()
                .expect("lock should be available")
                .push(RecordedCall::Publish(request.into_inner()));
            Ok(Response::new(NodePublishVolumeResponse {}))
        }

        async fn node_unpublish_volume(
            &self,
            request: Request<NodeUnpublishVolumeRequest>,
        ) -> Result<Response<NodeUnpublishVolumeResponse>, Status> {
            self.calls
                .lock()
                .expect("lock should be available")
                .push(RecordedCall::Unpublish(request.into_inner()));
            Ok(Response::new(NodeUnpublishVolumeResponse {}))
        }

        async fn node_get_capabilities(
            &self,
            _request: Request<NodeGetCapabilitiesRequest>,
        ) -> Result<Response<NodeGetCapabilitiesResponse>, Status> {
            Ok(Response::new(NodeGetCapabilitiesResponse::default()))
        }

        async fn node_get_info(
            &self,
            _request: Request<NodeGetInfoRequest>,
        ) -> Result<Response<NodeGetInfoResponse>, Status> {
            Ok(Response::new(NodeGetInfoResponse::default()))
        }

        async fn node_get_volume_stats(
            &self,
            _request: Request<NodeGetVolumeStatsRequest>,
        ) -> Result<Response<NodeGetVolumeStatsResponse>, Status> {
            Ok(Response::new(NodeGetVolumeStatsResponse::default()))
        }

        async fn node_expand_volume(
            &self,
            _request: Request<NodeExpandVolumeRequest>,
        ) -> Result<Response<NodeExpandVolumeResponse>, Status> {
            Ok(Response::new(NodeExpandVolumeResponse::default()))
        }
    }

    async fn spawn_node_server() -> (String, Arc<Mutex<Vec<RecordedCall>>>) {
        let socket_path = std::env::temp_dir().join(format!(
            "tugboat-csi-operator-{}.sock",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be monotonic")
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).expect("listener should bind");
        let incoming = UnixListenerStream::new(listener);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let service = FakeNodeService {
            calls: calls.clone(),
        };
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(NodeServer::new(service))
                .serve_with_incoming(incoming)
                .await
                .expect("server should run");
        });
        sleep(Duration::from_millis(50)).await;
        (socket_path.display().to_string(), calls)
    }

    #[test]
    fn filesystem_access_type_is_encoded_as_mount_volume() {
        let capability = volume_capability(CsiAccessMode::ReadWriteOnce, CsiAccessType::Filesystem);
        assert!(matches!(capability.access_type, Some(AccessType::Mount(_))));
    }

    #[test]
    fn block_access_type_is_encoded_as_block_volume() {
        let capability = volume_capability(CsiAccessMode::ReadWriteOnce, CsiAccessType::Block);
        assert!(matches!(capability.access_type, Some(AccessType::Block(_))));
    }

    #[tokio::test]
    async fn can_stage_and_publish_volume_over_uds() {
        let operator = TugboatCsiOperator::default();
        let (socket_path, calls) = spawn_node_server().await;
        let publish_context = HashMap::from([("published".to_string(), "yes".to_string())]);
        let volume_context = HashMap::from([("volume".to_string(), "ctx".to_string())]);

        operator
            .stage(
                &socket_path,
                "volume-1".to_string(),
                "/staging/volume-1".to_string(),
                CsiAccessMode::ReadWriteOnce,
                CsiAccessType::Filesystem,
                HashMap::from([("token".to_string(), "secret".to_string())]),
                volume_context.clone(),
                publish_context.clone(),
            )
            .await
            .expect("stage should succeed");
        operator
            .publish(
                &socket_path,
                "volume-1".to_string(),
                "/publish/volume-1".to_string(),
                false,
                CsiAccessMode::ReadWriteOnce,
                CsiAccessType::Filesystem,
                Some("/staging/volume-1".to_string()),
                HashMap::from([("token".to_string(), "secret".to_string())]),
                volume_context.clone(),
                publish_context.clone(),
            )
            .await
            .expect("publish should succeed");

        let calls = calls.lock().expect("lock should be available").clone();
        assert_eq!(calls.len(), 2);

        let RecordedCall::Stage(stage_request) = &calls[0] else {
            panic!("first call should be stage");
        };
        assert_eq!(stage_request.staging_target_path, "/staging/volume-1");
        assert_eq!(
            stage_request.secrets.get("token"),
            Some(&"secret".to_string())
        );
        assert_eq!(stage_request.publish_context, publish_context);
        assert_eq!(stage_request.volume_context, volume_context);
        assert!(matches!(
            stage_request
                .volume_capability
                .as_ref()
                .and_then(|capability| capability.access_type.clone()),
            Some(AccessType::Mount(_))
        ));

        let RecordedCall::Publish(publish_request) = &calls[1] else {
            panic!("second call should be publish");
        };
        assert_eq!(publish_request.target_path, "/publish/volume-1");
        assert_eq!(publish_request.staging_target_path, "/staging/volume-1");
        assert_eq!(
            publish_request.secrets.get("token"),
            Some(&"secret".to_string())
        );
    }

    #[tokio::test]
    async fn can_unpublish_and_unstage_volume_over_uds() {
        let operator = TugboatCsiOperator::default();
        let (socket_path, calls) = spawn_node_server().await;

        operator
            .unpublish(
                &socket_path,
                "volume-1".to_string(),
                "/publish/volume-1".to_string(),
            )
            .await
            .expect("unpublish should succeed");
        operator
            .unstage(
                &socket_path,
                "volume-1".to_string(),
                "/staging/volume-1".to_string(),
            )
            .await
            .expect("unstage should succeed");

        let calls = calls.lock().expect("lock should be available").clone();
        assert_eq!(calls.len(), 2);

        let RecordedCall::Unpublish(unpublish_request) = &calls[0] else {
            panic!("first call should be unpublish");
        };
        assert_eq!(unpublish_request.target_path, "/publish/volume-1");

        let RecordedCall::Unstage(unstage_request) = &calls[1] else {
            panic!("second call should be unstage");
        };
        assert_eq!(unstage_request.staging_target_path, "/staging/volume-1");
    }
}
