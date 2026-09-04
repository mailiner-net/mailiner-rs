//! Thin IndexedDB helper and [`JsonObjectStore`] backend (WASM only).

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use dioxus::logger::tracing::warn;
use futures_channel::oneshot;
use js_sys::Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{
    DomStringList, IdbDatabase, IdbFactory, IdbOpenDbRequest, IdbRequest, IdbTransactionMode,
    IdbVersionChangeEvent,
};

use crate::account_store::AccountStoreError;
use crate::mail_cache::{BrowserMailCache, MailCache};
use crate::object_cache::ObjectStoreMailCache;
use crate::offline_cache::{IDB_DB_NAME, IDB_STORES, IDB_VERSION, JsonObjectStore};

/// [`MailCache`] persisted in IndexedDB database [`IDB_DB_NAME`].
pub type IndexedDbMailCache = ObjectStoreMailCache<IdbBackend>;

/// Open [`IDB_DB_NAME`], importing a leftover localStorage blob when empty.
pub async fn open_indexed_db_mail_cache() -> Result<IndexedDbMailCache, AccountStoreError> {
    let backend = IdbBackend::open().await?;
    let cache = ObjectStoreMailCache::new(backend);
    import_local_storage_blob(&cache).await;
    Ok(cache)
}

async fn import_local_storage_blob(cache: &IndexedDbMailCache) {
    match cache.is_empty().await {
        Ok(false) => return,
        Ok(true) => {}
        Err(e) => {
            warn!("IndexedDB emptiness check failed: {e}");
            return;
        }
    }
    let Ok(ls) = BrowserMailCache::open().await else {
        return;
    };
    let Ok(blob) = ls.snapshot_blob() else {
        return;
    };
    if blob.is_empty() {
        return;
    }
    if let Err(e) = blob.replay_into(cache).await {
        warn!("mail cache import into IndexedDB failed: {e}");
    }
}

fn indexed_db_factory() -> Result<IdbFactory, AccountStoreError> {
    let window = web_sys::window().ok_or(AccountStoreError::Unavailable)?;
    window
        .indexed_db()
        .map_err(|_| AccountStoreError::Unavailable)?
        .ok_or(AccountStoreError::Unavailable)
}

fn list_contains(names: &DomStringList, name: &str) -> bool {
    for i in 0..names.length() {
        if names.get(i).as_deref() == Some(name) {
            return true;
        }
    }
    false
}

/// One IndexedDB connection used as a [`JsonObjectStore`].
pub struct IdbBackend {
    db: IdbDatabase,
}

impl IdbBackend {
    pub async fn open() -> Result<Self, AccountStoreError> {
        let factory = indexed_db_factory()?;
        let request = factory
            .open_with_u32(IDB_DB_NAME, IDB_VERSION)
            .map_err(|_| AccountStoreError::Unavailable)?;

        let on_upgrade = Closure::wrap(Box::new(move |event: IdbVersionChangeEvent| {
            let Ok(target) = event
                .target()
                .ok_or(())
                .and_then(|t| t.dyn_into::<IdbOpenDbRequest>().map_err(|_| ()))
            else {
                return;
            };
            let Ok(db) = target
                .result()
                .map_err(|_| ())
                .and_then(|v| v.dyn_into::<IdbDatabase>().map_err(|_| ()))
            else {
                return;
            };
            let names = db.object_store_names();
            for store in IDB_STORES {
                if !list_contains(&names, store)
                    && let Err(e) = db.create_object_store(store)
                {
                    warn!("IndexedDB createObjectStore({store}) failed: {e:?}");
                }
            }
        }) as Box<dyn FnMut(_)>);
        request.set_onupgradeneeded(Some(on_upgrade.as_ref().unchecked_ref()));
        // Keep the upgrade handler alive until the open request settles.
        let upgrade_hold = on_upgrade;

        let blocked = Rc::new(RefCell::new(false));
        let on_blocked = Closure::wrap({
            let blocked = blocked.clone();
            Box::new(move |_event: web_sys::Event| {
                *blocked.borrow_mut() = true;
            }) as Box<dyn FnMut(_)>
        });
        request.set_onblocked(Some(on_blocked.as_ref().unchecked_ref()));

        let result = idb_request(request.unchecked_into()).await;
        drop(upgrade_hold);
        drop(on_blocked);
        if *blocked.borrow() && result.is_err() {
            return Err(AccountStoreError::Unavailable);
        }
        let db = result?
            .dyn_into::<IdbDatabase>()
            .map_err(|_| AccountStoreError::Unavailable)?;
        Ok(Self { db })
    }

    fn txn(
        &self,
        store: &str,
        mode: IdbTransactionMode,
    ) -> Result<web_sys::IdbObjectStore, AccountStoreError> {
        let txn = self
            .db
            .transaction_with_str_and_mode(store, mode)
            .map_err(|_| AccountStoreError::Unavailable)?;
        txn.object_store(store)
            .map_err(|_| AccountStoreError::Unavailable)
    }
}

#[async_trait(?Send)]
impl JsonObjectStore for IdbBackend {
    async fn get(&self, store: &str, key: &str) -> Result<Option<String>, AccountStoreError> {
        let obj = self.txn(store, IdbTransactionMode::Readonly)?;
        let req = obj
            .get(&JsValue::from_str(key))
            .map_err(|_| AccountStoreError::Unavailable)?;
        let val = idb_request(req).await?;
        if val.is_undefined() || val.is_null() {
            return Ok(None);
        }
        match val.as_string() {
            Some(s) => Ok(Some(s)),
            None => Err(AccountStoreError::Serialization(
                "IndexedDB value is not a string".into(),
            )),
        }
    }

    async fn put(&self, store: &str, key: &str, value: &str) -> Result<(), AccountStoreError> {
        let obj = self.txn(store, IdbTransactionMode::Readwrite)?;
        let req = obj
            .put_with_key(&JsValue::from_str(value), &JsValue::from_str(key))
            .map_err(|_| AccountStoreError::Unavailable)?;
        idb_request(req).await?;
        Ok(())
    }

    async fn delete(&self, store: &str, key: &str) -> Result<(), AccountStoreError> {
        let obj = self.txn(store, IdbTransactionMode::Readwrite)?;
        let req = obj
            .delete(&JsValue::from_str(key))
            .map_err(|_| AccountStoreError::Unavailable)?;
        idb_request(req).await?;
        Ok(())
    }

    async fn keys(&self, store: &str) -> Result<Vec<String>, AccountStoreError> {
        let obj = self.txn(store, IdbTransactionMode::Readonly)?;
        let req = obj
            .get_all_keys()
            .map_err(|_| AccountStoreError::Unavailable)?;
        let val = idb_request(req).await?;
        js_string_array(&val)
    }

    async fn values(&self, store: &str) -> Result<Vec<String>, AccountStoreError> {
        let obj = self.txn(store, IdbTransactionMode::Readonly)?;
        let req = obj.get_all().map_err(|_| AccountStoreError::Unavailable)?;
        let val = idb_request(req).await?;
        js_string_array(&val)
    }

    async fn clear(&self, store: &str) -> Result<(), AccountStoreError> {
        let obj = self.txn(store, IdbTransactionMode::Readwrite)?;
        let req = obj.clear().map_err(|_| AccountStoreError::Unavailable)?;
        idb_request(req).await?;
        Ok(())
    }
}

fn js_string_array(val: &JsValue) -> Result<Vec<String>, AccountStoreError> {
    let arr = Array::from(val);
    let mut out = Vec::with_capacity(arr.length() as usize);
    for i in 0..arr.length() {
        match arr.get(i).as_string() {
            Some(s) => out.push(s),
            None => {
                return Err(AccountStoreError::Serialization(
                    "IndexedDB key/value is not a string".into(),
                ));
            }
        }
    }
    Ok(out)
}

struct IdbRequestFuture {
    _success: Closure<dyn FnMut(web_sys::Event)>,
    _error: Closure<dyn FnMut(web_sys::Event)>,
    rx: oneshot::Receiver<Result<JsValue, AccountStoreError>>,
}

impl Future for IdbRequestFuture {
    type Output = Result<JsValue, AccountStoreError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.get_mut().rx).poll(cx) {
            Poll::Ready(Ok(r)) => Poll::Ready(r),
            Poll::Ready(Err(_)) => Poll::Ready(Err(AccountStoreError::Unavailable)),
            Poll::Pending => Poll::Pending,
        }
    }
}

fn idb_request(request: IdbRequest) -> IdbRequestFuture {
    let (tx, rx) = oneshot::channel();
    let tx = Rc::new(RefCell::new(Some(tx)));

    let on_success = Closure::once({
        let tx = tx.clone();
        move |event: web_sys::Event| {
            let result = event
                .target()
                .and_then(|t| t.dyn_into::<IdbRequest>().ok())
                .and_then(|r| r.result().ok())
                .ok_or(AccountStoreError::Unavailable);
            if let Some(tx) = tx.borrow_mut().take() {
                let _ = tx.send(result);
            }
        }
    });

    let on_error = Closure::once({
        let tx = tx.clone();
        move |_event: web_sys::Event| {
            if let Some(tx) = tx.borrow_mut().take() {
                let _ = tx.send(Err(AccountStoreError::Unavailable));
            }
        }
    });

    request.set_onsuccess(Some(on_success.as_ref().unchecked_ref()));
    request.set_onerror(Some(on_error.as_ref().unchecked_ref()));

    IdbRequestFuture {
        _success: on_success,
        _error: on_error,
        rx,
    }
}
