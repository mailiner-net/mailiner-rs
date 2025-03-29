use crate::kernel::MailinerError;
use crate::kernel::Result;
use async_trait::async_trait;

#[async_trait]
pub trait CacheStorage: Send + Sync + 'static {
    async fn get(&self, key: &str) -> Result<Option<String>>;
    async fn set(&self, key: &str, value: &str) -> Result<()>;
    async fn remove(&self, key: &str) -> Result<()>;
    async fn clear(&self) -> Result<()>;
    async fn keys(&self, prefix: &str) -> Result<Vec<String>>;
}

// Web implementation using localStorage
#[derive(Default)]
pub struct WebLocalStorage;

#[async_trait]
impl CacheStorage for WebLocalStorage {
    async fn get(&self, key: &str) -> Result<Option<String>> {
        let window = web_sys::window().expect("no global `window` exists");
        let storage = window.local_storage().map_err(|e| {
            MailinerError::Cache(format!("Failed to get localStorage: {:?}", e))
        })?
        .expect("no localStorage exists");
        
        match storage.get_item(key).map_err(|e| {
            MailinerError::Cache(format!("Failed to get item: {:?}", e))
        })? {
            Some(value) => Ok(Some(value)),
            None => Ok(None),
        }
    }
    
    async fn set(&self, key: &str, value: &str) -> Result<()> {
        let window = web_sys::window().expect("no global `window` exists");
        let storage = window.local_storage().map_err(|e| {
            MailinerError::Cache(format!("Failed to get localStorage: {:?}", e))
        })?
        .expect("no localStorage exists");
        
        storage.set_item(key, value).map_err(|e| {
            MailinerError::Cache(format!("Failed to set item: {:?}", e))
        })?;
        
        Ok(())
    }
    
    async fn remove(&self, key: &str) -> Result<()> {
        let window = web_sys::window().expect("no global `window` exists");
        let storage = window.local_storage().map_err(|e| {
            MailinerError::Cache(format!("Failed to get localStorage: {:?}", e))
        })?
        .expect("no localStorage exists");

        storage.remove_item(key).map_err(|e| {
            MailinerError::Cache(format!("Failed to remove item: {:?}", e))
        })?;

        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        let window = web_sys::window().expect("no global `window` exists");
        let storage = window.local_storage().map_err(|e| {
            MailinerError::Cache(format!("Failed to get localStorage: {:?}", e))
        })?
        .expect("no localStorage exists");

        storage.clear().map_err(|e| {
            MailinerError::Cache(format!("Failed to clear localStorage: {:?}", e))
        })?;

        Ok(())
    }

    async fn keys(&self, prefix: &str) -> Result<Vec<String>> {
        let window = web_sys::window().expect("no global `window` exists");
        let storage = window.local_storage().map_err(|e| {
            MailinerError::Cache(format!("Failed to get localStorage: {:?}", e))
        })?
        .expect("no localStorage exists");

        let length = storage.length().map_err(|e| {
            MailinerError::Cache(format!("Failed to get length: {:?}", e))
        })?;
        let mut keys = Vec::with_capacity(length as usize);
        for i in 0..length {
            if let Ok(Some(key)) = storage.key(i) {
                if key.starts_with(prefix) {
                    keys.push(key);
                }
            }
        }

        Ok(keys)
    }
}