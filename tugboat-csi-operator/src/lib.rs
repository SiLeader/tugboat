use crate::proto::csi::v1::node_client::NodeClient;
use crate::proto::csi::v1::volume_capability::access_mode::Mode;
use crate::proto::csi::v1::volume_capability::{AccessMode, AccessType, BlockVolume};
use crate::proto::csi::v1::{NodePublishVolumeRequest, VolumeCapability};
pub use error::Error;
use tonic::Code;

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

        let mut client = NodeClient::connect(socket_path.to_string()).await?;

        if let Err(e) = client.node_publish_volume(req).await {
            match e.code() {
                Code::Ok => Ok(()),
                Code::AlreadyExists => Err(error::Error::TargetPathAlreadyExists),
                Code::FailedPrecondition => Err(error::Error::FailedPrecondition),
                _ => Err(error::Error::Grpc(e)),
            }
        } else {
            Ok(())
        }
    }
}
