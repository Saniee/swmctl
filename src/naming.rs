use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameMode {
    ModId,
    Name,
}

pub fn sanitize_name(name: &str) -> String {
    let mut result = String::new();
    let mut separator = false;

    for character in name.nfd() {
        if is_combining_mark(character) {
            continue;
        }
        if character.is_ascii_alphanumeric() {
            if separator && !result.is_empty() {
                result.push('_');
            }
            result.push(character.to_ascii_lowercase());
            separator = false;
        } else if !result.is_empty() {
            separator = true;
        }
    }

    result
}

pub fn directory_name(mode: NameMode, mod_id: &str, name: &str) -> String {
    match mode {
        NameMode::ModId => mod_id.to_string(),
        NameMode::Name => {
            let sanitized = sanitize_name(name);
            if sanitized.is_empty() {
                mod_id.to_string()
            } else {
                sanitized
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_diacritics_and_replaces_separators() {
        assert_eq!(sanitize_name("Möd: A Great Mod!"), "mod_a_great_mod");
    }

    #[test]
    fn falls_back_to_id_for_empty_names() {
        assert_eq!(directory_name(NameMode::Name, "123", "!!!"), "123");
    }
}
