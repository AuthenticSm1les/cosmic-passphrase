use zeroize::Zeroizing;

#[derive(Debug)]
pub struct DialogOutput {
    pub passphrase: Option<Zeroizing<String>>,
    pub confirmed: bool,
    pub cancelled: bool,
    pub remember: bool,
}

impl DialogOutput {
    pub fn ok(passphrase: Zeroizing<String>) -> Self {
        Self {
            passphrase: Some(passphrase),
            confirmed: true,
            cancelled: false,
            remember: false,
        }
    }

    pub fn ok_remember(passphrase: Zeroizing<String>, remember: bool) -> Self {
        Self {
            passphrase: Some(passphrase),
            confirmed: true,
            cancelled: false,
            remember,
        }
    }

    pub fn confirmed() -> Self {
        Self {
            passphrase: None,
            confirmed: true,
            cancelled: false,
            remember: false,
        }
    }

    pub fn not_confirmed() -> Self {
        Self {
            passphrase: None,
            confirmed: false,
            cancelled: false,
            remember: false,
        }
    }

    pub fn cancelled() -> Self {
        Self {
            passphrase: None,
            confirmed: false,
            cancelled: true,
            remember: false,
        }
    }
}

#[cfg(test)]
fn str_passphrase(passphrase: &Option<Zeroizing<String>>) -> Option<&str> {
    passphrase.as_ref().map(|z| z.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_ok() {
        let output = DialogOutput::ok(Zeroizing::new("secret".into()));
        assert_eq!(str_passphrase(&output.passphrase), Some("secret"));
        assert!(output.confirmed);
        assert!(!output.cancelled);
        assert!(!output.remember);
    }

    #[test]
    fn test_output_ok_empty() {
        let output = DialogOutput::ok(Zeroizing::new(String::new()));
        assert_eq!(str_passphrase(&output.passphrase), Some(""));
        assert!(output.confirmed);
    }

    #[test]
    fn test_output_ok_remember_true() {
        let output = DialogOutput::ok_remember(Zeroizing::new("p4ss".into()), true);
        assert_eq!(str_passphrase(&output.passphrase), Some("p4ss"));
        assert!(output.confirmed);
        assert!(output.remember);
    }

    #[test]
    fn test_output_ok_remember_false() {
        let output = DialogOutput::ok_remember(Zeroizing::new("p4ss".into()), false);
        assert!(!output.remember);
    }

    #[test]
    fn test_output_confirmed() {
        let output = DialogOutput::confirmed();
        assert!(output.passphrase.is_none());
        assert!(output.confirmed);
        assert!(!output.cancelled);
        assert!(!output.remember);
    }

    #[test]
    fn test_output_not_confirmed() {
        let output = DialogOutput::not_confirmed();
        assert!(output.passphrase.is_none());
        assert!(!output.confirmed);
        assert!(!output.cancelled);
    }

    #[test]
    fn test_output_cancelled() {
        let output = DialogOutput::cancelled();
        assert!(output.passphrase.is_none());
        assert!(!output.confirmed);
        assert!(output.cancelled);
    }

    #[test]
    fn test_output_remember_independent_of_passphrase() {
        let output = DialogOutput::ok_remember(Zeroizing::new("x".into()), true);
        assert!(output.remember);
        assert!(output.confirmed);
        assert_eq!(str_passphrase(&output.passphrase), Some("x"));
    }

    #[test]
    fn test_output_zeroize_on_drop() {
        let passphrase = Zeroizing::new(String::from("supersecret"));
        let output = DialogOutput::ok(passphrase);
        assert_eq!(str_passphrase(&output.passphrase), Some("supersecret"));
    }
}

