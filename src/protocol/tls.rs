use std::sync::{Arc, LazyLock};
use tokio_rustls::TlsConnector;

static SHARED_TLS_CONFIG: LazyLock<Arc<rustls::ClientConfig>> = LazyLock::new(|| {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    // Prefer HTTP/2, fallback to HTTP/1.1
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Arc::new(config)
});

pub fn get_tls_connector() -> TlsConnector {
    TlsConnector::from(SHARED_TLS_CONFIG.clone())
}
