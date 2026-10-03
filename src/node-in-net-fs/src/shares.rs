use serde_json::{json, Value};

pub const SHARES_KEY: &str = "shares";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Share {
    pub name: String,
    pub path: String,
}

pub fn suggested_name(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or("share")
        .to_string()
}

pub fn parse(source: &str) -> Vec<Share> {
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(source) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|held| {
            let name = held["name"].as_str().unwrap_or_default().trim();
            let path = held["path"].as_str().unwrap_or_default().trim();
            (!name.is_empty() && !path.is_empty()).then(|| Share {
                name: name.to_string(),
                path: path.to_string(),
            })
        })
        .collect()
}

pub fn encode(list: &[Share]) -> String {
    Value::Array(
        list.iter()
            .map(|share| json!({ "name": share.name, "path": share.path }))
            .collect(),
    )
    .to_string()
}

pub fn add(list: &[Share], name: &str, path: &str) -> Result<Vec<Share>, String> {
    let path = path.trim().trim_end_matches(['/', '\\']);
    if path.is_empty() {
        return Err(crate::i18n::tr("nodeinnet.select_folder_to_share"));
    }
    let name = match name.trim() {
        "" => suggested_name(path),
        given => given.to_string(),
    };
    if list
        .iter()
        .any(|held| held.name.eq_ignore_ascii_case(&name))
    {
        return Err(crate::i18n::trf("nodeinnet.name_taken", &[("name", &name)]));
    }
    if list.iter().any(|held| held.path == path) {
        return Err(crate::i18n::trf(
            "nodeinnet.already_shared",
            &[("path", path)],
        ));
    }
    let mut next = list.to_vec();
    next.push(Share {
        name,
        path: path.to_string(),
    });
    Ok(next)
}

pub fn remove(list: &[Share], name: &str) -> Vec<Share> {
    list.iter()
        .filter(|held| !held.name.eq_ignore_ascii_case(name.trim()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two() -> Vec<Share> {
        vec![
            Share {
                name: "docs".to_string(),
                path: "/home/user/docs".to_string(),
            },
            Share {
                name: "music".to_string(),
                path: "/home/user/music".to_string(),
            },
        ]
    }

    #[test]
    fn a_folder_lends_its_last_segment_as_the_name() {
        assert_eq!(suggested_name("/home/user/docs"), "docs");
        assert_eq!(suggested_name("/home/user/docs/"), "docs");
        assert_eq!(suggested_name("C:\\Users\\user\\Music"), "Music");
        assert_eq!(suggested_name("/"), "share");
    }

    #[test]
    fn adding_keeps_what_was_there_and_names_what_was_not_named() {
        let next = add(&two(), "", "/home/user/photos").expect("it is added");
        assert_eq!(next.len(), 3);
        assert_eq!(next[2].name, "photos");
        assert_eq!(next[0], two()[0], "the ones already there are untouched");
    }

    #[test]
    fn the_same_name_twice_is_refused_whatever_its_case() {
        let why = add(&two(), "DOCS", "/somewhere/else").expect_err("refused");
        assert_eq!(
            why,
            crate::i18n::trf("nodeinnet.name_taken", &[("name", "DOCS")])
        );
    }

    #[test]
    fn the_same_folder_under_another_name_is_refused_too() {
        assert_eq!(
            add(&two(), "papers", "/home/user/docs"),
            Err(crate::i18n::trf(
                "nodeinnet.already_shared",
                &[("path", "/home/user/docs")]
            ))
        );
        assert!(
            add(&two(), "papers", "/home/user/docs/").is_err(),
            "a trailing separator is the same folder"
        );
    }

    #[test]
    fn a_share_without_a_folder_is_refused_rather_than_stored_empty() {
        assert!(add(&two(), "empty", "   ").is_err());
        assert!(add(&[], "empty", "").is_err());
    }

    #[test]
    fn removing_takes_one_and_leaves_the_rest() {
        let left = remove(&two(), "docs");
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].name, "music");
        assert_eq!(remove(&two(), "nothing-of-that-name").len(), 2);
    }

    #[test]
    fn what_is_written_is_what_is_read_back() {
        let list = two();
        assert_eq!(parse(&encode(&list)), list);
        assert!(parse("").is_empty());
        assert!(parse("not json").is_empty());
        assert!(parse("{}").is_empty());
    }

    #[test]
    fn a_stored_entry_missing_its_path_is_dropped_rather_than_half_read() {
        let held = r#"[{"name":"docs"},{"name":"","path":"/x"},{"name":"ok","path":"/y"}]"#;
        let list = parse(held);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "ok");
    }
}
