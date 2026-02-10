use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::backend::Backend;
use crate::Result;

#[derive(Clone, Default)]
pub struct MockBackend {
    pub name: String,
    pub responses: Arc<Mutex<Vec<String>>>,
    pub call_count: Arc<Mutex<usize>>,
}

impl MockBackend {
    pub fn new(name: &str, responses: Vec<String>) -> Self {
        Self {
            name: name.to_owned(),
            responses: Arc::new(Mutex::new(responses)),
            call_count: Arc::new(Mutex::new(0)),
        }
    }

    pub async fn call_count(&self) -> usize {
        *self.call_count.lock().await
    }
}

#[async_trait]
impl Backend for MockBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, _prompt: &str) -> Result<String> {
        let mut count = self.call_count.lock().await;
        *count += 1;
        drop(count);

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
