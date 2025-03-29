use mailiner_rs::kernel::cache::WebLocalStorage;
use mailiner_rs::kernel::cache::CacheStorage;
use wasm_bindgen_test::wasm_bindgen_test;


#[wasm_bindgen_test]
async fn test_web_local_storage() {
    let cache = WebLocalStorage::default();

    let key = "test_key";
    let value = "test_value";

    cache.set(key, value).await.unwrap();

    let result = cache.get(key).await.unwrap();
    assert_eq!(result, Some(value.to_string()));

    cache.remove(key).await.unwrap();

    let result = cache.get(key).await.unwrap();
    assert_eq!(result, None);
    
}

#[wasm_bindgen_test]
async fn test_web_local_storage_keys() {
    let cache = WebLocalStorage::default();

    let key = "test_key";
    let key2 = "key2";
    let value = "test_value";

    cache.set(key, value).await.unwrap();
    cache.set(key2, value).await.unwrap();

    let keys = cache.keys("test").await.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys, vec![key]);
}
