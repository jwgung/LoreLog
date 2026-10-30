//! What goes into the journal about a failure: its kind, never its text, which may carry a password, a URL's
//! credentials or what the user wrote.

/// The variant name of an error's `Debug` form (`Io(Custom { .. })` -> `Io`). Pass typed errors: for a bare string
/// the first word of the string itself would survive.
pub fn kind(e: &dyn std::fmt::Debug) -> String {
    let s = format!("{e:?}");
    let k = s.split(['(', ' ', '{']).next().unwrap_or_default();
    if k.is_empty() { "error".into() } else { k.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    #[allow(dead_code)]
    enum E {
        Auth(String),
        Tls { detail: String },
        Closed,
    }

    #[test]
    fn only_the_kind_is_kept() {
        assert_eq!(kind(&E::Auth("password=hunter2".into())), "Auth");
        assert_eq!(kind(&E::Tls { detail: "https://u:p@host".into() }), "Tls");
        assert_eq!(kind(&E::Closed), "Closed");
        assert_eq!(kind(&()), "error", "nothing before the first separator");
    }
}
