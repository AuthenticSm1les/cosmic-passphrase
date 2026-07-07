use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogMode {
    Passphrase,
    Confirm,
    Message,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtraContent {
    None,
    Repeat,
    Remember,
}

#[derive(Debug, Clone)]
pub struct DialogConfig {
    pub title: String,
    pub description: Option<String>,
    pub error: Option<String>,
    pub prompt: String,
    pub ok_label: String,
    pub cancel_label: String,
    pub notok_label: Option<String>,
    pub mode: DialogMode,
    pub extra: ExtraContent,
    pub timeout: Option<Duration>,
}

impl Default for DialogConfig {
    fn default() -> Self {
        Self {
            title: String::from("Passphrase Required"),
            description: None,
            error: None,
            prompt: String::from("Passphrase:"),
            ok_label: String::from("OK"),
            cancel_label: String::from("Cancel"),
            notok_label: None,
            mode: DialogMode::Passphrase,
            extra: ExtraContent::None,
            timeout: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_title() {
        let config = DialogConfig::default();
        assert_eq!(config.title, "Passphrase Required");
    }

    #[test]
    fn test_default_config_mode() {
        let config = DialogConfig::default();
        assert_eq!(config.mode, DialogMode::Passphrase);
    }

    #[test]
    fn test_default_config_extra() {
        let config = DialogConfig::default();
        assert_eq!(config.extra, ExtraContent::None);
    }

    #[test]
    fn test_default_config_no_timeout() {
        let config = DialogConfig::default();
        assert!(config.timeout.is_none());
    }

    #[test]
    fn test_default_config_labels() {
        let config = DialogConfig::default();
        assert_eq!(config.ok_label, "OK");
        assert_eq!(config.cancel_label, "Cancel");
        assert!(config.notok_label.is_none());
    }

    #[test]
    fn test_default_config_optional_fields() {
        let config = DialogConfig::default();
        assert!(config.description.is_none());
        assert!(config.error.is_none());
    }

    #[test]
    fn test_dialog_mode_equality() {
        assert_eq!(DialogMode::Passphrase, DialogMode::Passphrase);
        assert_ne!(DialogMode::Passphrase, DialogMode::Confirm);
        assert_ne!(DialogMode::Passphrase, DialogMode::Message);
    }

    #[test]
    fn test_extra_content_equality() {
        assert_eq!(ExtraContent::None, ExtraContent::None);
        assert_ne!(ExtraContent::None, ExtraContent::Repeat);
        assert_ne!(ExtraContent::None, ExtraContent::Remember);
    }

    #[test]
    fn test_config_custom_values() {
        let config = DialogConfig {
            title: String::from("Custom Title"),
            prompt: String::from("Enter PIN:"),
            ok_label: String::from("Yes"),
            cancel_label: String::from("No"),
            mode: DialogMode::Confirm,
            extra: ExtraContent::Remember,
            timeout: Some(Duration::from_secs(30)),
            ..Default::default()
        };
        assert_eq!(config.title, "Custom Title");
        assert_eq!(config.prompt, "Enter PIN:");
        assert_eq!(config.ok_label, "Yes");
        assert_eq!(config.cancel_label, "No");
        assert_eq!(config.mode, DialogMode::Confirm);
        assert_eq!(config.extra, ExtraContent::Remember);
        assert_eq!(config.timeout, Some(Duration::from_secs(30)));
    }
}
