use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigLine {
    Blank,
    Comment(String),
    Section(String),
    Setting { key: String, value: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDocument {
    pub lines: Vec<ConfigLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingTypeHint {
    Boolean,
    Integer,
    Decimal,
    String,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ValueConstraint {
    None,
    Range { minimum: f64, maximum: f64 },
    Choices(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingWidget {
    Checkbox,
    MultiCheckbox,
    Slider,
    NumberInput,
    Dropdown,
    Text,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingMetadata {
    pub description_lines: Vec<String>,
    pub setting_type: Option<SettingTypeHint>,
    pub setting_type_label: Option<String>,
    pub default_value: Option<String>,
    pub constraint: ValueConstraint,
    pub allows_multiple_values: bool,
    pub has_hint_lines: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EditableSetting {
    pub section: String,
    pub key: String,
    pub value: String,
    pub line_index: usize,
    pub metadata: SettingMetadata,
    pub widget: SettingWidget,
    pub can_revert: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueCoerceError {
    InvalidBoolean { raw: String },
    InvalidInteger { raw: String },
    InvalidDecimal { raw: String },
}

impl fmt::Display for ValueCoerceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBoolean { raw } => {
                write!(formatter, "expected true or false, got '{raw}'")
            }
            Self::InvalidInteger { raw } => {
                write!(formatter, "expected an integer, got '{raw}'")
            }
            Self::InvalidDecimal { raw } => {
                write!(formatter, "expected a number, got '{raw}'")
            }
        }
    }
}

impl ConfigDocument {
    pub fn editable_settings(&self) -> Vec<EditableSetting> {
        let mut settings = Vec::new();
        let mut section = String::new();
        let mut preceding_comments: Vec<&str> = Vec::new();

        for (line_index, line) in self.lines.iter().enumerate() {
            match line {
                ConfigLine::Blank => {
                    preceding_comments.clear();
                }
                ConfigLine::Comment(text) => {
                    preceding_comments.push(text);
                }
                ConfigLine::Section(name) => {
                    section = name.clone();
                    preceding_comments.clear();
                }
                ConfigLine::Setting { key, value } => {
                    let metadata = SettingMetadata::from_comment_lines(&preceding_comments);
                    let widget = SettingWidget::from_metadata(&metadata);
                    let can_revert = metadata.default_value.is_some() && metadata.has_hint_lines;
                    settings.push(EditableSetting {
                        section: section.clone(),
                        key: key.clone(),
                        value: value.clone(),
                        line_index,
                        metadata,
                        widget,
                        can_revert,
                    });
                    preceding_comments.clear();
                }
            }
        }

        settings
    }

    pub fn set_value(&mut self, line_index: usize, value: impl Into<String>) -> bool {
        match self.lines.get_mut(line_index) {
            Some(ConfigLine::Setting {
                value: stored, ..
            }) => {
                *stored = value.into();
                true
            }
            _ => false,
        }
    }

    pub fn set_flags_values(
        &mut self,
        line_index: usize,
        selected: &[String],
    ) -> bool {
        self.set_value(line_index, join_flags_values(selected))
    }

    pub fn revert_setting_to_default(
        &mut self,
        line_index: usize,
    ) -> Result<bool, ValueCoerceError> {
        let setting = self
            .editable_settings()
            .into_iter()
            .find(|entry| entry.line_index == line_index);
        let Some(setting) = setting else {
            return Ok(false);
        };
        if !setting.can_revert {
            return Ok(false);
        }
        let Some(raw_default) = setting.metadata.default_value.as_deref() else {
            return Ok(false);
        };
        let coerced = coerce_value_for_type(raw_default, setting.metadata.setting_type)?;
        Ok(self.set_value(line_index, coerced))
    }
}

impl SettingMetadata {
    pub fn from_comment_lines(comment_lines: &[&str]) -> Self {
        let has_hint_lines = !comment_lines.is_empty();
        let mut description_lines = Vec::new();
        let mut setting_type_label = None;
        let mut default_value = None;
        let mut constraint = ValueConstraint::None;
        let mut allows_multiple_values = false;

        for raw in comment_lines {
            let body = strip_comment_prefix(raw);
            if let Some(label) = strip_prefix_ignore_case(body, "Setting type:") {
                setting_type_label = Some(label.trim().to_string());
            } else if let Some(value) = strip_prefix_ignore_case(body, "Default value:") {
                default_value = Some(value.trim().to_string());
            } else if let Some(values) = strip_prefix_ignore_case(body, "Acceptable values:") {
                let choices = split_comma_list(values);
                if !choices.is_empty() {
                    constraint = ValueConstraint::Choices(choices);
                }
            } else if let Some(range_text) =
                strip_prefix_ignore_case(body, "Acceptable value range:")
            {
                if let Some(range) = parse_range_hint(range_text.trim()) {
                    constraint = ValueConstraint::Range {
                        minimum: range.0,
                        maximum: range.1,
                    };
                }
            } else if starts_with_ignore_case(body, "Multiple values can be set") {
                allows_multiple_values = true;
            } else if !body.is_empty() {
                description_lines.push(body.to_string());
            }
        }

        let setting_type = setting_type_label
            .as_deref()
            .map(SettingTypeHint::from_label);

        Self {
            description_lines,
            setting_type,
            setting_type_label,
            default_value,
            constraint,
            allows_multiple_values,
            has_hint_lines,
        }
    }
}

impl SettingTypeHint {
    pub fn from_label(label: &str) -> Self {
        let normalized = label.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "boolean" | "bool" => Self::Boolean,
            "int32" | "int64" | "int16" | "int" | "uint32" | "uint64" | "byte" | "sbyte" => {
                Self::Integer
            }
            "single" | "double" | "float" | "decimal" => Self::Decimal,
            "string" => Self::String,
            _ if is_integer_type_name(&normalized) => Self::Integer,
            _ if is_decimal_type_name(&normalized) => Self::Decimal,
            _ => Self::Other,
        }
    }

    pub fn is_numeric(self) -> bool {
        matches!(self, Self::Integer | Self::Decimal)
    }
}

impl SettingWidget {
    pub fn from_metadata(metadata: &SettingMetadata) -> Self {
        if let ValueConstraint::Choices(_) = &metadata.constraint {
            if metadata.allows_multiple_values {
                return Self::MultiCheckbox;
            }
            return Self::Dropdown;
        }

        if let ValueConstraint::Range { .. } = metadata.constraint {
            if matches!(
                metadata.setting_type,
                Some(SettingTypeHint::Integer | SettingTypeHint::Decimal) | None
            ) {
                return Self::Slider;
            }
        }

        match metadata.setting_type {
            Some(SettingTypeHint::Boolean) => Self::Checkbox,
            Some(SettingTypeHint::Integer | SettingTypeHint::Decimal) => Self::NumberInput,
            Some(SettingTypeHint::String) | Some(SettingTypeHint::Other) | None => Self::Text,
        }
    }
}

pub fn join_flags_values(selected: &[String]) -> String {
    selected.join(", ")
}

pub fn split_flags_values(value: &str) -> Vec<String> {
    split_comma_list(value)
}

pub fn coerce_value_for_type(
    raw: &str,
    setting_type: Option<SettingTypeHint>,
) -> Result<String, ValueCoerceError> {
    let trimmed = raw.trim();
    match setting_type {
        Some(SettingTypeHint::Boolean) => coerce_boolean(trimmed),
        Some(SettingTypeHint::Integer) => coerce_integer(trimmed),
        Some(SettingTypeHint::Decimal) => coerce_decimal(trimmed),
        Some(SettingTypeHint::String) | Some(SettingTypeHint::Other) | None => {
            Ok(trimmed.to_string())
        }
    }
}

pub fn validate_numeric_input(
    raw: &str,
    setting_type: SettingTypeHint,
) -> Result<String, ValueCoerceError> {
    match setting_type {
        SettingTypeHint::Integer => coerce_integer(raw.trim()),
        SettingTypeHint::Decimal => coerce_decimal(raw.trim()),
        _ => Ok(raw.trim().to_string()),
    }
}

fn coerce_boolean(raw: &str) -> Result<String, ValueCoerceError> {
    match raw.to_ascii_lowercase().as_str() {
        "true" => Ok("true".to_string()),
        "false" => Ok("false".to_string()),
        _ => Err(ValueCoerceError::InvalidBoolean {
            raw: raw.to_string(),
        }),
    }
}

fn coerce_integer(raw: &str) -> Result<String, ValueCoerceError> {
    raw.parse::<i64>()
        .map(|number| number.to_string())
        .map_err(|_| ValueCoerceError::InvalidInteger {
            raw: raw.to_string(),
        })
}

fn coerce_decimal(raw: &str) -> Result<String, ValueCoerceError> {
    let number = raw.parse::<f64>().map_err(|_| ValueCoerceError::InvalidDecimal {
        raw: raw.to_string(),
    })?;
    if !number.is_finite() {
        return Err(ValueCoerceError::InvalidDecimal {
            raw: raw.to_string(),
        });
    }
    Ok(format_decimal(number, raw))
}

fn format_decimal(number: f64, original: &str) -> String {
    if original.contains(['.', 'e', 'E']) {
        let rendered = number.to_string();
        if rendered.contains('.') || rendered.contains(['e', 'E']) {
            rendered
        } else {
            format!("{number:.1}")
        }
    } else if (number - number.trunc()).abs() < f64::EPSILON {
        format!("{number:.0}")
    } else {
        number.to_string()
    }
}

fn split_comma_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(|piece| piece.trim().to_string())
        .filter(|piece| !piece.is_empty())
        .collect()
}

fn strip_comment_prefix(line: &str) -> &str {
    let trimmed = line.trim_start();
    let without_hashes = trimmed.trim_start_matches('#');
    without_hashes.strip_prefix(' ').unwrap_or(without_hashes)
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    if text.len() < prefix.len() {
        return None;
    }
    if text[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&text[prefix.len()..])
    } else {
        None
    }
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len() && text[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn parse_range_hint(text: &str) -> Option<(f64, f64)> {
    let trimmed = text.trim();
    let without_from = strip_prefix_ignore_case(trimmed, "From ")?.trim();
    let (minimum_text, maximum_text) = split_once_ignore_case(without_from, " to ")?;
    let minimum = minimum_text.trim().parse::<f64>().ok()?;
    let maximum = maximum_text.trim().parse::<f64>().ok()?;
    Some((minimum, maximum))
}

fn split_once_ignore_case<'a>(text: &'a str, separator: &str) -> Option<(&'a str, &'a str)> {
    let lower_text = text.to_ascii_lowercase();
    let lower_separator = separator.to_ascii_lowercase();
    let index = lower_text.find(&lower_separator)?;
    let end = index + separator.len();
    Some((&text[..index], &text[end..]))
}

fn is_integer_type_name(normalized: &str) -> bool {
    normalized.contains("int")
        && !normalized.contains("point")
        && !normalized.contains("interface")
}

fn is_decimal_type_name(normalized: &str) -> bool {
    normalized.contains("float")
        || normalized.contains("double")
        || normalized.contains("single")
        || normalized.contains("decimal")
}
