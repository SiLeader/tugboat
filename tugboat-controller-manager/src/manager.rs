use crate::base::TugboatController;
use tracing::info;

pub(crate) struct TugboatControllerManager {
    controllers: Vec<Box<dyn TugboatController>>,
}

impl TugboatControllerManager {
    pub(crate) fn new() -> Self {
        Self {
            controllers: vec![],
        }
    }

    pub(crate) fn add_controller<T>(&mut self, controller: T)
    where
        T: TugboatController + 'static,
    {
        self.controllers.push(Box::new(controller));
    }

    pub(crate) async fn setup(&mut self) {
        info!("Setting up controllers");
        for controller in self.controllers.iter_mut() {
            info!("Setting up '{}' controller", controller.name());
            controller.setup().await;
        }
    }

    pub(crate) async fn run(self) {
        info!("Running controllers");
        let mut handle = Vec::with_capacity(self.controllers.len());
        for controller in self.controllers {
            info!("Spawning '{}' controller", controller.name());
            handle.push(tokio::spawn(async move { controller.run().await }));
        }
        for handle in handle {
            match handle.await {
                Ok(_) => {}
                Err(e) => tracing::error!("Controller task join error: {:?}", e),
            }
        }
    }
}
