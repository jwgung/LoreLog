//! Running the receiving server inside another program (the desktop app): its settings in ingest.env, a token made
//! on first use, which addresses count as the tailnet, and starting and stopping it in-process.
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::Arc;

use life_server::config::Paths;
use life_server::host::{self, IngestSettings, Models};

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn ingest_settings_have_the_old_defaults_and_refuse_a_short_token() {
    let s = IngestSettings::from_map(&map(&[("INGEST_TOKEN", "0123456789abcdef")])).unwrap();
    assert_eq!(s.bind, "127.0.0.1".parse::<IpAddr>().unwrap());
    assert_eq!((s.port, s.max_bytes, s.cls.as_str()), (8765, 20 * 1024 * 1024, "personal"));
    assert_eq!((s.cert.as_deref(), s.key.as_deref()), (None, None));

    let s = IngestSettings::from_map(&map(&[
        ("INGEST_TOKEN", "0123456789abcdef"),
        ("INGEST_BIND", "100.101.1.2"),
        ("INGEST_PORT", "9000"),
        ("INGEST_MAX_MB", "1.5"),
        ("INGEST_CLASS", "work"),
        ("INGEST_CERT", "/c.pem"),
        ("INGEST_KEY", ""),
    ]))
    .unwrap();
    assert_eq!((s.bind.to_string().as_str(), s.port, s.max_bytes), ("100.101.1.2", 9000, 1_572_864));
    assert_eq!((s.cls.as_str(), s.cert.as_deref(), s.key.as_deref()), ("work", Some("/c.pem"), None));

    let short = IngestSettings::from_map(&map(&[("INGEST_TOKEN", "short")])).unwrap_err();
    assert!(short.contains("INGEST_TOKEN"), "{short}");
    assert!(IngestSettings::from_map(&map(&[("INGEST_TOKEN", "0123456789abcdef"), ("INGEST_BIND", "x")])).is_err());
    assert!(IngestSettings::from_map(&map(&[("INGEST_TOKEN", "0123456789abcdef"), ("INGEST_PORT", "y")])).is_err());
}

#[test]
fn model_addresses_come_from_the_file_and_the_environment_wins() {
    let file = map(&[("LIFE_LLM_URL", "http://file/v1"), ("LIFE_LLM_MODEL", "m1")]);
    let env = map(&[("LIFE_LLM_MODEL", "m2"), ("LIFE_ASR_URL", ""), ("LIFE_LLM_URL", "")]);
    let m = Models::from_maps(&file, &env);
    assert_eq!(
        (m.chat_url.as_str(), m.chat_model.as_str()),
        ("http://file/v1", "m2"),
        "an empty variable hides nothing"
    );
    assert_eq!((m.asr_url.as_str(), m.asr_model.as_str()), ("", ""), "an empty variable is not a setting");
}

#[test]
fn a_token_is_made_once_and_other_settings_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let first = host::ensure_token(&paths).unwrap();
    assert!(first.len() >= 32 && first.chars().all(|c| c.is_ascii_hexdigit()), "{first}");
    assert_eq!(host::ensure_token(&paths).unwrap(), first, "kept, not made again");
    let file = host::ingest_env(&paths);
    std::fs::write(&file, "INGEST_PORT=9001\nINGEST_TOKEN=short\n").unwrap();
    let made = host::ensure_token(&paths).unwrap();
    assert_ne!(made, "short", "a token too short to use is replaced");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("INGEST_PORT=9001") && text.contains(&format!("INGEST_TOKEN={made}")), "{text}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    }
}

#[test]
fn tailnet_addresses_are_the_cgnat_range_and_the_tailscale_ula() {
    let ip = |s: &str| s.parse::<IpAddr>().unwrap();
    assert!(host::is_tailnet_ip(ip("100.101.171.95")));
    assert!(host::is_tailnet_ip(ip("100.64.0.1")));
    assert!(host::is_tailnet_ip(ip("100.127.255.254")));
    assert!(!host::is_tailnet_ip(ip("100.128.0.1")));
    assert!(!host::is_tailnet_ip(ip("192.168.0.2")));
    assert!(host::is_tailnet_ip(ip("fd7a:115c:a1e0::1")));
    assert!(!host::is_tailnet_ip(ip("fd00::1")));
    assert!(!host::is_tailnet_ip(ip("127.0.0.1")));
}

#[test]
fn the_server_starts_on_the_given_addresses_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let settings =
        IngestSettings::from_map(&map(&[("INGEST_TOKEN", "0123456789abcdef"), ("INGEST_PORT", "0")])).unwrap();
    let app = Arc::new(Models::default().app(paths, &settings));
    let running = host::start(rt.handle(), app, settings.port, &["127.0.0.1".parse().unwrap()]);
    let addr = running.listening()[0];
    let url = format!("http://{addr}/healthz");
    let body: String = ureq::get(&url).call().unwrap().body_mut().read_to_string().unwrap();
    assert_eq!(body, r#"{"ok":true}"#);
    let todos = format!("http://{addr}/todos");
    assert_eq!(ureq::get(&todos).call().unwrap_err().to_string(), "http status: 401");
    let ok = ureq::get(&todos).header("Authorization", "Bearer 0123456789abcdef").call().unwrap();
    assert_eq!(ok.status(), 200);
    running.stop();
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(ureq::get(&url).call().is_err(), "nothing listens after stop");
}

#[test]
fn an_address_that_cannot_be_bound_is_reported_and_the_others_still_serve() {
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let settings =
        IngestSettings::from_map(&map(&[("INGEST_TOKEN", "0123456789abcdef"), ("INGEST_PORT", "0")])).unwrap();
    let addrs: [IpAddr; 2] = ["127.0.0.1".parse().unwrap(), "192.0.2.1".parse().unwrap()]; // TEST-NET: not ours
    let app = Arc::new(Models::default().app(Paths::under(dir.path()), &settings));
    let running = host::start(rt.handle(), app, settings.port, &addrs);
    assert_eq!(running.listening().len(), 1);
    let failed = running.failed();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].0, "192.0.2.1".parse::<IpAddr>().unwrap());
    running.stop();
}
