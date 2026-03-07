use serde_json::{Value, json};

const MOBENCH_COMMAND: &str = "/mobench";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualRunArgs {
    pub platform: String,
    pub device_profile: String,
    pub ios_device: String,
    pub ios_os_version: String,
    pub android_device: String,
    pub android_os_version: String,
    pub iterations: String,
    pub warmup: String,
}

impl Default for ManualRunArgs {
    fn default() -> Self {
        Self {
            platform: "both".to_string(),
            device_profile: "low-spec".to_string(),
            ios_device: String::new(),
            ios_os_version: String::new(),
            android_device: String::new(),
            android_os_version: String::new(),
            iterations: "30".to_string(),
            warmup: "5".to_string(),
        }
    }
}

impl ManualRunArgs {
    pub fn parse_manual_command(body: &str) -> Option<Self> {
        let command = body
            .lines()
            .find_map(|line| {
                let trimmed = line.trim();
                (!trimmed.is_empty()).then_some(trimmed)
            })?
            .strip_prefix(MOBENCH_COMMAND)?;
        if !command.is_empty() && !command.starts_with(char::is_whitespace) {
            return None;
        }

        let mut args = Self::default();
        let remainder = command.trim();
        if remainder.is_empty() {
            return Some(args);
        }

        let mut current_key: Option<&str> = None;
        let mut current_value = String::new();
        for token in remainder.split_whitespace() {
            if let Some((key, value)) = token.split_once('=') {
                args.apply_token(current_key.take(), &current_value)?;
                current_value.clear();

                if !Self::is_allowed_key(key) {
                    return None;
                }

                current_key = Some(key);
                current_value.push_str(value);
            } else if current_key.is_some() {
                if !current_value.is_empty() {
                    current_value.push(' ');
                }
                current_value.push_str(token);
            } else {
                return None;
            }
        }

        args.apply_token(current_key, &current_value)?;
        args.validate()?;
        Some(args)
    }

    pub fn workflow_inputs(&self, pr_number: i32, requested_by: &str) -> Value {
        json!({
            "platform": self.platform,
            "device_profile": self.device_profile,
            "ios_device": self.ios_device,
            "ios_os_version": self.ios_os_version,
            "android_device": self.android_device,
            "android_os_version": self.android_os_version,
            "iterations": self.iterations,
            "warmup": self.warmup,
            "pr_number": pr_number.to_string(),
            "requested_by": requested_by,
        })
    }

    fn is_allowed_key(key: &str) -> bool {
        matches!(
            key,
            "platform"
                | "device_profile"
                | "ios_device"
                | "ios_os_version"
                | "android_device"
                | "android_os_version"
                | "iterations"
                | "warmup"
        )
    }

    fn apply_token(&mut self, key: Option<&str>, value: &str) -> Option<()> {
        let key = match key {
            Some(key) => key,
            None => return Some(()),
        };

        match key {
            "platform" => self.platform = value.to_string(),
            "device_profile" => self.device_profile = value.to_string(),
            "ios_device" => self.ios_device = value.to_string(),
            "ios_os_version" => self.ios_os_version = value.to_string(),
            "android_device" => self.android_device = value.to_string(),
            "android_os_version" => self.android_os_version = value.to_string(),
            "iterations" => self.iterations = value.to_string(),
            "warmup" => self.warmup = value.to_string(),
            _ => return None,
        }

        Some(())
    }

    fn validate(&self) -> Option<()> {
        if !matches!(self.platform.as_str(), "ios" | "android" | "both") {
            return None;
        }

        Self::validate_positive_integer(&self.iterations)?;
        Self::validate_positive_integer(&self.warmup)?;
        Some(())
    }

    fn validate_positive_integer(value: &str) -> Option<()> {
        value
            .parse::<u32>()
            .ok()
            .filter(|parsed| *parsed > 0)
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::ManualRunArgs;

    #[test]
    fn parses_bare_command_defaults() {
        let args = ManualRunArgs::parse_manual_command("/mobench").unwrap();

        assert_eq!(args.platform, "both");
        assert_eq!(args.iterations, "30");
        assert_eq!(args.warmup, "5");
    }

    #[test]
    fn parses_values_with_spaces() {
        let args = ManualRunArgs::parse_manual_command(
            "/mobench platform=ios ios_device=iPhone 15 ios_os_version=17 iterations=50",
        )
        .unwrap();

        assert_eq!(args.platform, "ios");
        assert_eq!(args.ios_device, "iPhone 15");
        assert_eq!(args.ios_os_version, "17");
        assert_eq!(args.iterations, "50");
    }

    #[test]
    fn rejects_invalid_keys_and_values() {
        assert!(ManualRunArgs::parse_manual_command("/mobench foo=bar").is_none());
        assert!(ManualRunArgs::parse_manual_command("/mobench platform=web").is_none());
        assert!(ManualRunArgs::parse_manual_command("/mobench iterations=0").is_none());
    }
}
