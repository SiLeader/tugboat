use crate::runtime::RuntimeOperator;
use crate::runtime::error::RuntimeError;
use crate::runtime::runtime::Runtime;
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::error;
use tugboat_resources::manifests::core::v1::{CpuSpec, Ship, ShipClass};
use tugboat_resources::sized::SizedString;

#[derive(Debug, Clone, Serialize)]
struct VmConfig {
    image: String,
    cpu: CpuSpec,
    memory: u64,
    id: String,
}

impl RuntimeOperator {
    pub(crate) async fn run(&self, ship: Ship, ship_class: ShipClass) -> Result<(), RuntimeError> {
        let Some(ship_id) = ship.object_meta.and_then(|o| o.uid) else {
            return Err(RuntimeError::MissingField(
                "v1.Ship.metadata.uid".to_string(),
            ));
        };
        let Some(ship_class_spec) = ship_class.spec else {
            return Err(RuntimeError::MissingField("v1.ShipClass.spec".to_string()));
        };
        let Some(ship_spec) = ship.spec else {
            return Err(RuntimeError::MissingField("v1.Ship.spec".to_string()));
        };
        let Some(cpu) = ship_class_spec.cpu else {
            return Err(RuntimeError::MissingField(
                "v1.ShipClass.spec.cpu".to_string(),
            ));
        };
        let Some(memory) = ship_class_spec.memory else {
            return Err(RuntimeError::MissingField(
                "v1.ShipClass.spec.memory".to_string(),
            ));
        };
        let image = self.registry.pull(ship_spec.image, None).await?;

        let memory_size = SizedString(memory.size.clone());

        let vm_config = VmConfig {
            id: ship_id.clone(),
            image: image.location,
            cpu,
            memory: memory_size
                .as_byte_length()
                .ok_or(RuntimeError::MemorySize(memory.size))?,
        };
        let vm_config = serde_json::to_string(&vm_config)?;
        let mut child = Command::new(&self.config.executable)
            .args(self.config.args.as_slice())
            .args(["run", "-"])
            .stdin(Stdio::piped())
            .spawn()?;
        match &mut child.stdin {
            Some(stdin) => {
                if let Err(e) = stdin.write_all(vm_config.as_bytes()).await {
                    error!("Cannot write config: {e}");
                    kill_impl(child).await?;
                    Err(RuntimeError::Io(e))
                } else {
                    let mut children = self.children.write().await;
                    children.insert(ship_id.clone(), Runtime::new(ship_id, child));
                    Ok(())
                }
            }
            None => {
                kill_impl(child).await?;
                Err(RuntimeError::RunVm)
            }
        }
    }
}

async fn kill_impl(mut child: tokio::process::Child) -> Result<(), RuntimeError> {
    child.kill().await?;
    tokio::spawn(async move {
        if let Err(e) = child.wait().await {
            error!("Cannot wait child: {e}");
        }
    });
    Ok(())
}
