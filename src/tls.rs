//! **The fleet authority, and presenting the operator's own certificate.**
//!
//! `SPEC-01` §4's second credential. It is the same triple `cargo rivet backup`
//! reads from the same directory — `browser.crt`, `browser.key`, `root.crt` —
//! and the same posture: the root is **specific**, never the platform's trust
//! store. A fleet authority is in no public store and must not be, and trusting
//! the platform's roots here would mean accepting anything a public CA vouches
//! for in place of the one authority this fleet knows.

use std::path::Path;
use std::sync::Arc;

/// The operator's certificate, as `rivet-provision ca` names it.
pub const OPERATOR_CERTIFICATE: &str = "browser.crt";
/// Its private key.
pub const OPERATOR_KEY: &str = "browser.key";
/// The authority both ends chain to.
pub const ROOT_FILE: &str = "root.crt";

/// Build a TLS configuration that presents the operator and verifies the fleet.
///
/// # Errors
///
/// Any of the three files missing or unreadable, naming the path and what the
/// file is for.
pub fn config(authority: &Path) -> Result<ureq::tls::TlsConfig, String> {
    let read = |name: &str| -> Result<Vec<u8>, String> {
        let path = authority.join(name);
        std::fs::read(&path).map_err(|e| {
            format!(
                "{}: {e}\n\x20 why  this is the fleet authority (SPEC-01 §4), which reads the \
                 fleet register and checks each machine's proxy — the same pair your browser is \
                 enrolled with.\n\
                 \x20 fix  point fleet.authority at the directory `rivet-provision ca` wrote.",
                path.display()
            )
        })
    };
    let chain = ureq::tls::Certificate::from_pem(&read(OPERATOR_CERTIFICATE)?)
        .map_err(|e| format!("{OPERATOR_CERTIFICATE} is not a certificate: {e}"))?;
    let key = ureq::tls::PrivateKey::from_pem(&read(OPERATOR_KEY)?)
        .map_err(|e| format!("{OPERATOR_KEY} is not a private key: {e}"))?;
    let root = ureq::tls::Certificate::from_pem(&read(ROOT_FILE)?)
        .map_err(|e| format!("{ROOT_FILE} is not a certificate: {e}"))?;

    Ok(ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::Specific(Arc::new(vec![root])))
        .client_cert(Some(ureq::tls::ClientCert::new_with_certs(&[chain], key)))
        .build())
}

/// An agent that sends every connection to one address whatever name the URL
/// carries.
///
/// **A fleet certificate carries a DNS name and the workstation may not
/// resolve it.** The request is made against the NAME so that the name is what
/// TLS verifies; the resolver puts the connection on the ADDRESS. This is the
/// arrangement `cargo rivet backup` uses, for the same reason.
pub fn agent_for(
    tls: ureq::tls::TlsConfig,
    address: std::net::SocketAddr,
    timeout: std::time::Duration,
) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .tls_config(tls)
        .build();
    ureq::Agent::with_parts(
        config,
        ureq::unversioned::transport::DefaultConnector::new(),
        resolver::Fixed(address),
    )
}

/// One socket address for `host:port`.
///
/// # Errors
///
/// A name that does not resolve, said here rather than inside a handshake.
pub fn resolve(host: &str, port: u16) -> Result<std::net::SocketAddr, String> {
    use std::net::ToSocketAddrs as _;
    (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("{host}:{port} does not resolve: {e}"))?
        .next()
        .ok_or_else(|| format!("{host}:{port} resolved to no address"))
}

mod resolver {
    use std::net::SocketAddr;

    use ureq::unversioned::resolver::{ArrayVec, ResolvedSocketAddrs, Resolver};

    /// The address, and nothing else.
    #[derive(Debug)]
    pub struct Fixed(pub SocketAddr);

    impl Resolver for Fixed {
        fn resolve(
            &self,
            _uri: &ureq::http::Uri,
            _config: &ureq::config::Config,
            _timeout: ureq::unversioned::transport::NextTimeout,
        ) -> Result<ResolvedSocketAddrs, ureq::Error> {
            // `from_fn` fills the backing array and leaves the length at zero,
            // so the address has to be pushed.
            let mut out: ResolvedSocketAddrs = ArrayVec::from_fn(|_| self.0);
            out.push(self.0);
            Ok(out)
        }
    }
}
