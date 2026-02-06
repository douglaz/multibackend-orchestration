use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::backend::Backend;
use crate::Result;

#[derive(Clone, Default)]
pub struct MockBackend {
    pub name: String,
    pub responses: Arc<Mutex<Vec<String>>>,
}

impl MockBackend {
    pub fn new(name: &str, responses: Vec<String>) -> Self {
        Self {
            name: name.to_owned(),
            responses: Arc::new(Mutex::new(responses)),
        }
    }
}

#[async_trait]
impl Backend for MockBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, _prompt: &str) -> Result<String> {
        let mut guard = self.responses.lock().await;
        if guard.is_empty() {
            return Ok(String::new());
        }
        Ok(guard.remove(0))
    }

    async fn health_check(&self) -> Result<()> {
        Ok(())
    }
}
