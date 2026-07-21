use std::fmt;

use super::model::{ConfigDocument, ConfigLine};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line_number: usize,
    pub line_text: String,
    pub reason: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "config parse error on line {}: {} ({})",
            self.line_number, self.reason, self.line_text
        )
    }
}

pub fn parse_config_text(text: &str) -> Result<ConfigDocument, ParseError> {
    let mut lines = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        lines.push(parse_line(line, line_number)?);
    }
    Ok(ConfigDocument { lines })
}

pub fn serialize_config(document: &ConfigDocument) -> String {
    if document.lines.is_empty() {
        return String::new();
    }
    let mut output = String::new();
    for line in &document.lines {
        output.push_str(&serialize_line(line));
        output.push('\n');
    }
    output
}

fn parse_line(line: &str, line_number: usize) -> Result<ConfigLine, ParseError> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(ConfigLine::Blank);
    }
    if trimmed.starts_with('#') {
        return Ok(ConfigLine::Comment(line.to_string()));
    }
    if trimmed.starts_with('[') {
        return parse_section_line(trimmed, line, line_number);
    }
    if trimmed.contains('=') {
        return parse_setting_line(trimmed, line, line_number);
    }
    Err(ParseError {
        line_number,
        line_text: line.to_string(),
        reason: "expected a [section], key = value, comment, or blank line".to_string(),
    })
}

fn parse_section_line(
    trimmed: &str,
    original: &str,
    line_number: usize,
) -> Result<ConfigLine, ParseError> {
    if !(trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() >= 2) {
        return Err(ParseError {
            line_number,
            line_text: original.to_string(),
            reason: "malformed section header; expected [sectionname]".to_string(),
        });
    }
    let name = &trimmed[1..trimmed.len() - 1];
    if name.is_empty() || name.contains(['[', ']']) {
        return Err(ParseError {
            line_number,
            line_text: original.to_string(),
            reason: "malformed section header; expected [sectionname]".to_string(),
        });
    }
    Ok(ConfigLine::Section(name.to_string()))
}

fn parse_setting_line(
    trimmed: &str,
    original: &str,
    line_number: usize,
) -> Result<ConfigLine, ParseError> {
    let Some((key_text, value_text)) = trimmed.split_once('=') else {
        return Err(ParseError {
            line_number,
            line_text: original.to_string(),
            reason: "malformed setting; expected key = value".to_string(),
        });
    };
    let key = key_text.trim();
    if key.is_empty() {
        return Err(ParseError {
            line_number,
            line_text: original.to_string(),
            reason: "malformed setting; key is empty".to_string(),
        });
    }
    if key.contains('[') || key.contains(']') {
        return Err(ParseError {
            line_number,
            line_text: original.to_string(),
            reason: "malformed setting; key must not contain brackets".to_string(),
        });
    }
    Ok(ConfigLine::Setting {
        key: key.to_string(),
        value: value_text.trim().to_string(),
    })
}

fn serialize_line(line: &ConfigLine) -> String {
    match line {
        ConfigLine::Blank => String::new(),
        ConfigLine::Comment(text) => text.clone(),
        ConfigLine::Section(name) => format!("[{name}]"),
        ConfigLine::Setting { key, value } => format!("{key} = {value}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::{
        join_flags_values, split_flags_values, SettingWidget, ValueConstraint,
    };

    const SAMPLE: &str = include_str!("sample.cfg");

    #[test]
    fn round_trips_sample_without_changes() {
        let document = parse_config_text(SAMPLE).expect("parse");
        let serialized = serialize_config(&document);
        assert_eq!(serialized, SAMPLE);
    }

    #[test]
    fn maps_all_five_widget_cases() {
        let document = parse_config_text(SAMPLE).expect("parse");
        let settings = document.editable_settings();
        assert_eq!(settings.len(), 6);

        let flags = &settings[0];
        assert_eq!(flags.key, "LogChannels");
        assert_eq!(flags.widget, SettingWidget::MultiCheckbox);
        assert!(flags.can_revert);
        assert!(matches!(
            &flags.metadata.constraint,
            ValueConstraint::Choices(choices) if choices.len() == 6
        ));
        assert_eq!(
            split_flags_values(&flags.value),
            vec!["Warn".to_string(), "Error".to_string()]
        );
        assert_eq!(
            join_flags_values(&["Warn".into(), "Error".into(), "Debug".into()]),
            "Warn, Error, Debug"
        );

        let unranged_int = &settings[1];
        assert_eq!(unranged_int.key, "MaxRetries");
        assert_eq!(unranged_int.widget, SettingWidget::NumberInput);
        assert_eq!(
            unranged_int.metadata.setting_type,
            Some(crate::config::SettingTypeHint::Integer)
        );

        let unranged_float = &settings[2];
        assert_eq!(unranged_float.key, "Volume");
        assert_eq!(unranged_float.widget, SettingWidget::NumberInput);
        assert_eq!(
            unranged_float.metadata.setting_type,
            Some(crate::config::SettingTypeHint::Decimal)
        );

        let text = &settings[3];
        assert_eq!(text.key, "PlayerName");
        assert_eq!(text.widget, SettingWidget::Text);

        let ranged = &settings[4];
        assert_eq!(ranged.key, "MoveSpeed");
        assert_eq!(ranged.widget, SettingWidget::Slider);

        let bare = &settings[5];
        assert_eq!(bare.key, "OrphanNote");
        assert_eq!(bare.widget, SettingWidget::Text);
        assert!(!bare.can_revert);
        assert!(!bare.metadata.has_hint_lines);
    }

    #[test]
    fn reverts_boolean_default_with_type_coercion() {
        let text = "\
[General]

## toggle
# Setting type: Boolean
# Default value: True
Flag = false
";
        let mut document = parse_config_text(text).expect("parse");
        let setting = document.editable_settings().remove(0);
        assert!(setting.can_revert);
        assert!(document
            .revert_setting_to_default(setting.line_index)
            .expect("coerce"));
        let updated = document.editable_settings().remove(0);
        assert_eq!(updated.value, "true");
    }

    #[test]
    fn rejects_malformed_lines() {
        let error = parse_config_text("[General]\nthis is garbage\n").expect_err("must fail");
        assert_eq!(error.line_number, 2);
        assert!(error.reason.contains("expected"));
    }

    #[test]
    fn bare_entry_cannot_revert() {
        let mut document = parse_config_text("[A]\nBare = 1\n").expect("parse");
        let setting = document.editable_settings().remove(0);
        assert!(!setting.can_revert);
        assert!(!document
            .revert_setting_to_default(setting.line_index)
            .expect("no coerce needed"));
    }
}
