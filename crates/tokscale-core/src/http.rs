//! Shared construction of the workspace's HTTP clients.
//!
//! Every client except Cursor's goes through here, and every client here is
//! pinned to rustls with an explicit `use_rustls_tls()` rather than relying on
//! reqwest's default backend.
//!
//! The explicitness is load-bearing, not decoration. reqwest's `native-tls`
//! feature implies `default-tls`, and `impl Default for TlsBackend`
//! (reqwest `src/tls.rs`) resolves to the *native-tls* backend whenever
//! `default-tls` is enabled and `http3` is not. The Cursor client needs
//! native TLS to get past cursor.com's ClientHello fingerprinting (#1250), so
//! `default-tls` is compiled in on every target except Android — which means
//! an unqualified `reqwest::Client::new()` anywhere in this workspace would
//! silently be a *native-tls* client, not the rustls one its author intended.
//!
//! `clippy.toml` forbids `reqwest::Client::new` / `reqwest::Client::builder`
//! outside this module and `tokscale-cli`'s `cursor.rs` so a new call site
//! cannot re-open that hole by accident.

/// A `reqwest::ClientBuilder` pinned to the rustls backend.
///
/// Behaviourally identical to what an unqualified `reqwest::Client::builder()`
/// produced before native TLS entered the dependency graph: `use_rustls_tls()`
/// selects `TlsBackend::Rustls`, and `tls_built_in_certs_native` defaults to
/// `true`, so roots still come from the OS trust store via the workspace's
/// `rustls-tls-native-roots` feature.
#[allow(clippy::disallowed_methods)]
pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().use_rustls_tls()
}

/// A rustls-backed `reqwest::Client` with reqwest's default settings.
///
/// The drop-in replacement for `reqwest::Client::new()`, including its panic
/// shape: `Client::new()` is itself `builder().build().expect(..)`, so call
/// sites keep the error handling they already had.
pub fn client() -> reqwest::Client {
    client_builder()
        .build()
        .expect("failed to build the shared rustls HTTP client")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard for #1250's first attempt, which enabled reqwest's
    /// `native-tls` feature on the shared workspace dependency. That flipped
    /// `TlsBackend::default()` to native-tls for all ~28 clients in the
    /// workspace while the PR claimed only Cursor was affected. Nothing in the
    /// build caught it, because both backends compile fine either way.
    ///
    /// `ClientBuilder`'s `Debug` prints `tls_backend` only when both backends
    /// are compiled in, which is every target except Android.
    #[test]
    #[cfg(not(target_os = "android"))]
    fn shared_builder_is_pinned_to_rustls() {
        let rendered = format!("{:?}", client_builder());
        assert!(
            rendered.contains("tls_backend: Rustls"),
            "the shared client builder must select rustls explicitly, got: {rendered}"
        );
    }

    /// The reason the assertion above cannot be dropped: with `default-tls` in
    /// the graph, an unqualified builder is a *native-tls* builder. If this
    /// ever fails because reqwest stopped printing `Default` here, the pinning
    /// test above still stands on its own.
    #[test]
    #[cfg(not(target_os = "android"))]
    fn an_unqualified_builder_would_have_been_native_tls() {
        #[allow(clippy::disallowed_methods)]
        let rendered = format!("{:?}", reqwest::Client::builder());
        assert!(
            rendered.contains("tls_backend: Default"),
            "reqwest's unqualified default is expected to be the native-tls \
             backend while `default-tls` is enabled, got: {rendered}"
        );
    }

    /// Android is deliberately left without `default-tls`, so there is only
    /// one backend to choose and reqwest omits the field entirely.
    #[test]
    #[cfg(target_os = "android")]
    fn android_has_no_native_tls_backend_to_choose() {
        let rendered = format!("{:?}", client_builder());
        assert!(
            !rendered.contains("tls_backend"),
            "Android must compile with rustls only, got: {rendered}"
        );
    }

    #[test]
    fn client_builds() {
        let _ = client();
    }
}
