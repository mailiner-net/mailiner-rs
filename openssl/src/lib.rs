#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_ssl()
    {
        unsafe {
            assert_eq!(OPENSSL_init_ssl(OPENSSL_INIT_NO_LOAD_SSL_STRINGS.into(), std::ptr::null()), 1);
        }
    }
}