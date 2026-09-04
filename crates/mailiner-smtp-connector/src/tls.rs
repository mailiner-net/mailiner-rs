//! Extra CA PEM parsing and rustls root-store construction.

use rustls::RootCertStore;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::CertificateDer;

/// Parse one or more PEM-encoded certificates.
///
/// Empty / whitespace-only input yields an empty vec. Invalid PEM returns a
/// user-facing error (never panics).
pub fn parse_pem_certificates(pem: &str) -> Result<Vec<CertificateDer<'static>>, String> {
    let trimmed = pem.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let mut certs = Vec::new();
    for (i, item) in CertificateDer::pem_slice_iter(trimmed.as_bytes()).enumerate() {
        let cert = item.map_err(|e| format!("invalid PEM certificate (entry {}): {e}", i + 1))?;
        certs.push(cert);
    }
    if certs.is_empty() {
        return Err("no certificates found in PEM data".into());
    }
    Ok(certs)
}

/// Add user-imported extra CA PEMs to an existing root store.
///
/// Returns the number of certificates added.
pub fn add_extra_ca_pems(
    store: &mut RootCertStore,
    extra_ca_pems: &[String],
) -> Result<usize, String> {
    let mut added = 0;
    for (i, pem) in extra_ca_pems.iter().enumerate() {
        if pem.trim().is_empty() {
            continue;
        }
        let certs = parse_pem_certificates(pem).map_err(|e| format!("extra CA {}: {e}", i + 1))?;
        for cert in certs {
            store
                .add(cert)
                .map_err(|e| format!("extra CA {} is not a valid trust anchor: {e}", i + 1))?;
            added += 1;
        }
    }
    Ok(added)
}

/// webpki roots plus any extra user-imported CA PEMs.
pub fn root_cert_store(extra_ca_pems: &[String]) -> Result<RootCertStore, String> {
    let mut store = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    add_extra_ca_pems(&mut store, extra_ca_pems)?;
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_CA_PEM: &str = "-----BEGIN CERTIFICATE-----
MIIDFzCCAf+gAwIBAgIUO+6CL3p49lL/DAZ7JoXn3PwO/EYwDQYJKoZIhvcNAQEL
BQAwGzEZMBcGA1UEAwwQTWFpbGluZXIgVGVzdCBDQTAeFw0yNjA5MDQwNTE3MjBa
Fw0zNjA5MDEwNTE3MjBaMBsxGTAXBgNVBAMMEE1haWxpbmVyIFRlc3QgQ0EwggEi
MA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQCxzv/LVpg7NNfe69mqiVMd3RLT
qCRC7rtWG5p2SqSrdivyI3PiYBFkvZ6ATAKNbFfZ7hMNhllK+Piipf51AkEU96j5
vMPKzMbzEJjsBZlr1CRRCzoaV1HqPLI/Dq0C1myKwgC7ROJRRHNDdPm6vyhbXa5d
857z+RQcDlPcd3opdgHASZboQK+EugRYeMOQ/Cb7Qv237dlhC/29OqJh+Xt/3Z4J
wJ9SeIJV44ZMecRFbHcKPcOiBoFa8s+xllutVE0VV9hk06SH8ShWn5Chwvys6e+y
Iv29cPA7PdnjhG7m5XGoTVjd3LwSTT4LCElZG1doNWUpYlKV3dIk6XbKWp37AgMB
AAGjUzBRMB0GA1UdDgQWBBSAU6I1kMQhOQ/f2asKVD50yH2jWTAfBgNVHSMEGDAW
gBSAU6I1kMQhOQ/f2asKVD50yH2jWTAPBgNVHRMBAf8EBTADAQH/MA0GCSqGSIb3
DQEBCwUAA4IBAQBU014e32DvhQmdLXw+hNpcU288m3jBB6viGfxj2qvLRo1Z5On8
YQWgywe7vBIZ15+2zGieDQQlkahLR+ZhRmyW3SLEBL3izfUnQyJFYHKUTG/wsiNx
HG0ysqio9/x8oMv6quNwfE6LlTbYHhZxpyZLIfL47Xbv1J5ieEUr91naa9PSeG/P
jxqLaQgrhy4NGyFRZkLX7NtLiZfb3L1GOfKzitV7h7Sa+kLkf5oZrrjgoD7gGFCx
13nLK36fqa7TdSarmCTjaUnk5P0oyLpkeNJSiZF+XHTejL/3jAho/l90ji0F9KxC
nJwqI0fvxoBNVYHtAzKsaIAL9lb6rzzsbkDB
-----END CERTIFICATE-----";

    #[test]
    fn parse_empty_pem_is_ok() {
        assert!(parse_pem_certificates("").unwrap().is_empty());
        assert!(parse_pem_certificates("   \n\t  ").unwrap().is_empty());
    }

    #[test]
    fn parse_valid_pem_succeeds() {
        let certs = parse_pem_certificates(TEST_CA_PEM).unwrap();
        assert_eq!(certs.len(), 1);
    }

    #[test]
    fn parse_concatenated_pems_succeeds() {
        let doubled = format!("{TEST_CA_PEM}\n{TEST_CA_PEM}");
        let certs = parse_pem_certificates(&doubled).unwrap();
        assert_eq!(certs.len(), 2);
    }

    #[test]
    fn parse_invalid_text_fails() {
        let err = parse_pem_certificates("this is not a certificate").unwrap_err();
        assert!(
            err.contains("no certificates found") || err.contains("invalid PEM"),
            "{err}"
        );
    }

    #[test]
    fn parse_broken_pem_fails_without_panic() {
        let broken = "-----BEGIN CERTIFICATE-----\nnot-valid-base64!!!\n-----END CERTIFICATE-----";
        let err = parse_pem_certificates(broken).unwrap_err();
        assert!(err.contains("invalid PEM certificate"), "{err}");
    }

    #[test]
    fn extra_ca_is_added_to_root_store() {
        let baseline = root_cert_store(&[]).unwrap();
        let with_extra = root_cert_store(&[TEST_CA_PEM.to_string()]).unwrap();
        assert_eq!(with_extra.roots.len(), baseline.roots.len() + 1);
    }

    #[test]
    fn invalid_extra_ca_does_not_mutate_store() {
        let mut store = root_cert_store(&[]).unwrap();
        let before = store.roots.len();
        let err = add_extra_ca_pems(&mut store, &["not a pem".into()]).unwrap_err();
        assert!(err.contains("extra CA 1"), "{err}");
        assert_eq!(store.roots.len(), before);
    }
}
