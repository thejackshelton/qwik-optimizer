use crate::component::{SourceInfo, Target};
use crate::segment::Segment;
use base64::{engine, Engine};
use serde::Serialize;
use std::hash::{DefaultHasher, Hasher};

/// Unique component identifier with display name, symbol name, hash, and scope.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Id {
    pub display_name: String,
    pub symbol_name: String,
    pub local_file_name: String,
    pub hash: String,
    pub sort_order: u64,
    pub scope: Option<String>,
}

impl Id {
    fn sanitize(input: &str) -> String {
        input
            .chars()
            .fold((String::new(), false), |(mut acc, uscore), c| {
                if c.is_ascii_alphanumeric() {
                    acc.push(c);
                    (acc, false)
                } else if uscore {
                    (acc, true)
                } else {
                    acc.push('_');
                    (acc, true)
                }
            })
            .0
    }

    fn calculate_hash(
        local_file_name: &str,
        display_name: &str,
        scope: &Option<String>,
    ) -> (u64, String) {
        let mut hasher = DefaultHasher::new();
        if let Some(scope) = scope {
            hasher.write(scope.as_bytes());
        }
        hasher.write(local_file_name.as_bytes());
        hasher.write(display_name.as_bytes());
        let hash = hasher.finish();
        (
            hash,
            engine::general_purpose::URL_SAFE_NO_PAD
                .encode(hash.to_le_bytes())
                .replace(['-', '_'], "0"),
        )
    }

    fn update_display_name(display_name: &mut String, name_segment: String) {
        if display_name.is_empty()
            && name_segment
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
        {
            display_name.push('_');
            display_name.push_str(name_segment.as_str());
        } else if display_name.is_empty() {
            display_name.push_str(name_segment.as_str());
        } else {
            display_name.push('_');
            display_name.push_str(name_segment.as_str());
        }
    }

    pub(crate) fn new(
        source_info: &SourceInfo,
        segments: &Vec<Segment>,
        target: &Target,
        scope: &Option<String>,
    ) -> Id {
        let local_file_name = source_info.rel_path.to_string_lossy();

        let mut display_name = String::new();

        if let Some((tail, head)) = segments.split_last() {
            for s in head {
                match s {
                    Segment::Named(name) => {
                        Self::update_display_name(&mut display_name, name.into())
                    }
                    Segment::NamedQrl(name, 0) => {
                        Self::update_display_name(&mut display_name, name.into())
                    }
                    Segment::NamedQrl(name, index) => {
                        Self::update_display_name(&mut display_name, format!("{name}_{index}"))
                    }
                    Segment::IndexQrl(_) => {}
                }
            }

            match tail {
                Segment::Named(name) => Self::update_display_name(&mut display_name, name.into()),
                Segment::NamedQrl(name, 0) => {
                    Self::update_display_name(&mut display_name, name.into())
                }
                Segment::NamedQrl(name, index) => {
                    Self::update_display_name(&mut display_name, format!("{name}_{index}"))
                }
                Segment::IndexQrl(0) => {}
                Segment::IndexQrl(index) => {
                    Self::update_display_name(&mut display_name, index.to_string())
                }
            }

            display_name = Self::sanitize(&display_name);
        }

        let normalized_local_file_name = local_file_name
            .strip_prefix("./")
            .unwrap_or(&local_file_name);

        // display_name_without_file is the component name only (e.g., "test_component")
        // qwik-core hashes display_name WITHOUT file prefix, then adds file prefix AFTER for output
        // See qwik-core transform.rs:359 - hashes display_name.as_bytes() where display_name = "test_component"
        // See qwik-core transform.rs:368 - AFTER hashing: display_name = format!("{}_{}", file_name, display_name)
        let display_name_without_file = display_name;

        // Hash is calculated BEFORE adding file prefix (matches qwik-core order)
        let (sort_order, hash) =
            Self::calculate_hash(normalized_local_file_name, &display_name_without_file, scope);

        // Add file prefix AFTER hashing for output (display_name and local_file_name)
        let display_name = format!("{}_{}", source_info.file_name, display_name_without_file);

        // In dev mode, symbol_name is component_hash (without file prefix) - matches qwik-core
        // qwik-core: s_n = if dev { symbol_name.clone() } else { format!("s_{}", hash) }
        // where symbol_name is already component_hash format
        let symbol_name = match target {
            Target::Dev | Target::Test => format!("{}_{}", display_name_without_file, hash),
            Target::Lib | Target::Prod => format!("s_{}", hash),
        };

        // canonical_filename/local_file_name = file_displayName_hash (matches qwik-core)
        // qwik-core: get_canonical_filename(display_name, symbol_name) = format!("{}_{}", display_name, hash)
        // where display_name already includes file prefix, so result is file_component_hash
        let local_file_name = format!("{}_{}", display_name, hash);
        Id {
            display_name,
            symbol_name,
            local_file_name,
            hash,
            sort_order,
            scope: scope.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_a_name() {
        let name0 = Id::sanitize("a'b-c");
        let name1 = Id::sanitize("A123b_c-~45");
        assert_eq!(name0, "a_b_c");
        assert_eq!(name1, "A123b_c_45");
    }

    #[test]
    fn test_calculate_hash() {
        let (_, hash0) = Id::calculate_hash("./app.js", "a_b_c", &None);
        let (_, hash1) = Id::calculate_hash("./app.js", "a_b_c", &Some("scope".to_string()));
        assert_eq!(hash0, "0RVAWYCCxyk");
        assert_ne!(hash1, hash0);
    }

    #[test]
    fn creates_a_id() {
        let source_info0 = SourceInfo::new("app.js").unwrap();
        let id0 = Id::new(
            &source_info0,
            &vec![
                Segment::Named("a".to_string()),
                Segment::Named("b".to_string()),
                Segment::Named("c".to_string()),
            ],
            &Target::Dev,
            &Option::None,
        );
        // Hash is calculated from display_name WITHOUT file prefix (matches qwik-core order)
        // qwik-core hashes "a_b_c", then adds file prefix AFTER for output
        // symbol_name in dev mode uses component name only (without file prefix)
        // local_file_name is displayName_hash format
        let (sort_order, hash0) = Id::calculate_hash("app.js", "a_b_c", &None);

        let expected0 = Id {
            display_name: "app.js_a_b_c".to_string(),
            symbol_name: format!("a_b_c_{}", hash0),  // component + hash (no file prefix in dev)
            local_file_name: format!("app.js_a_b_c_{}", hash0),  // displayName + hash
            hash: hash0,
            sort_order,
            scope: None,
        };

        let scope1 = Some("scope".to_string());
        let id1 = Id::new(
            &source_info0,
            &vec![
                Segment::Named("1".to_string()),
                Segment::Named("b".to_string()),
                Segment::Named("c".to_string()),
            ],
            &Target::Prod,
            &scope1,
        );
        // With leading digit, display_name_without_file starts with _1 (before file prefix added)
        // Hash is calculated from "_1_b_c" (display_name WITHOUT file prefix)
        let (sort_order, hash1) = Id::calculate_hash("app.js", "_1_b_c", &scope1);
        let expected1 = Id {
            display_name: "app.js__1_b_c".to_string(),
            symbol_name: format!("s_{}", hash1),  // prod mode uses s_{hash}
            local_file_name: format!("app.js__1_b_c_{}", hash1),  // displayName + hash
            hash: hash1,
            sort_order,
            scope: Some("scope".to_string()),
        };

        assert_eq!(id0, expected0);
        assert_eq!(id1, expected1);
    }

    #[test]
    fn creates_a_id_with_indexes() {
        let source_info0 = SourceInfo::new("app.js").unwrap();
        let id1 = Id::new(
            &source_info0,
            &vec![
                Segment::Named("a".to_string()),
                Segment::Named("b".to_string()),
                Segment::IndexQrl(1),
                Segment::IndexQrl(2),
                Segment::IndexQrl(0),
            ],
            &Target::Dev,
            &None,
        );

        let id2 = Id::new(
            &source_info0,
            &vec![
                Segment::Named("a".to_string()),
                Segment::Named("b".to_string()),
                Segment::IndexQrl(1),
            ],
            &Target::Dev,
            &None,
        );

        let id3 = Id::new(
            &source_info0,
            &vec![
                Segment::Named("a".to_string()),
                Segment::Named("b".to_string()),
                Segment::NamedQrl("c".to_string(), 0),
            ],
            &Target::Dev,
            &None,
        );

        let id4 = Id::new(
            &source_info0,
            &vec![
                Segment::Named("a".to_string()),
                Segment::Named("b".to_string()),
                Segment::NamedQrl("c".to_string(), 1),
            ],
            &Target::Dev,
            &None,
        );

        let id5 = Id::new(
            &source_info0,
            &vec![
                Segment::Named("a".to_string()),
                Segment::NamedQrl("b".to_string(), 0),
                Segment::IndexQrl(1),
            ],
            &Target::Dev,
            &None,
        );

        let id6 = Id::new(
            &source_info0,
            &vec![
                Segment::Named("a".to_string()),
                Segment::NamedQrl("b".to_string(), 0),
                Segment::IndexQrl(0),
            ],
            &Target::Dev,
            &None,
        );

        // display_name now includes full filename (app.js) matching qwik-core
        assert_eq!(id1.display_name, "app.js_a_b");
        assert_eq!(id2.display_name, "app.js_a_b_1");
        assert_eq!(id3.display_name, "app.js_a_b_c");
        assert_eq!(id4.display_name, "app.js_a_b_c_1");
        assert_eq!(id5.display_name, "app.js_a_b_1");
        assert_eq!(id6.display_name, "app.js_a_b");
    }
}
