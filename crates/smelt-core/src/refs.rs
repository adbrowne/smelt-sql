use rowan::TextRange;
use smelt_parser::File as AstFile;

#[derive(Debug, Clone)]
pub struct RefInfo {
    pub model_name: String,
    pub has_named_params: bool,
    pub range: TextRange,
}

/// Extract all smelt.ref() calls from a parsed file
pub fn extract_refs(file: &AstFile) -> Vec<RefInfo> {
    file.refs()
        .filter_map(|ref_call| {
            let model_name = ref_call.model_name()?;
            let has_params = ref_call.named_params().count() > 0;
            let range = ref_call.range();

            Some(RefInfo {
                model_name,
                has_named_params: has_params,
                range,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_refs() {
        let sql = r#"
SELECT
    user_id,
    COUNT(*) as session_count
FROM smelt.ref('raw_events')
GROUP BY user_id
"#;

        let parse = smelt_parser::parse(sql);
        let file = AstFile::cast(parse.syntax()).unwrap();
        let refs = extract_refs(&file);

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].model_name, "raw_events");
        assert!(!refs[0].has_named_params);
    }

    #[test]
    fn test_extract_refs_with_named_params() {
        let sql = r#"
SELECT user_id
FROM smelt.ref('raw_events', filter => event_type = 'page_view')
"#;

        let parse = smelt_parser::parse(sql);
        let file = AstFile::cast(parse.syntax()).unwrap();
        let refs = extract_refs(&file);

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].model_name, "raw_events");
        assert!(refs[0].has_named_params);
    }

    #[test]
    fn test_multiple_refs() {
        let sql = r#"
SELECT
    a.user_id,
    b.session_id
FROM smelt.ref('model_a') a
INNER JOIN smelt.ref('model_b') b ON a.id = b.id
"#;

        let parse = smelt_parser::parse(sql);
        let file = AstFile::cast(parse.syntax()).unwrap();
        let refs = extract_refs(&file);

        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].model_name, "model_a");
        assert_eq!(refs[1].model_name, "model_b");
    }
}
