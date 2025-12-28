use rand::SeedableRng;
use rand::distr::{Alphanumeric, SampleString};
use rand_xorshift::XorShiftRng;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) struct NameGenerator {
    rand: Arc<Mutex<XorShiftRng>>,
}

impl NameGenerator {
    pub(crate) fn new() -> Self {
        Self {
            rand: Arc::new(Mutex::new(XorShiftRng::from_os_rng())),
        }
    }

    pub(crate) async fn generate_with_len(&self, name_base: &str, length: usize) -> String {
        let mut rng = self.rand.lock().await;
        format!(
            "{name_base}{}",
            Alphanumeric.sample_string(&mut rng, length)
        )
    }

    pub(crate) async fn generate(&self, name_base: &str) -> String {
        self.generate_with_len(name_base, 5).await
    }
}
