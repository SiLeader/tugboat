use tokio::process::Child;

pub(crate) struct Runtime {
    id: String,
    child: Child,
}

impl Runtime {
    pub(super) fn new(id: String, child: Child) -> Self {
        Self { id, child }
    }
}
