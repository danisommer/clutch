//! Just enough XML reading for the files that Windows tools and game launchers write: attributes and simple elements.

/// XML text without its comments, which often hold examples of the same elements.
pub fn without_comments(xml: &str) -> String {
    let mut text = String::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<!--") {
        text.push_str(&rest[..start]);
        rest = rest[start..].find("-->").map(|end| &rest[start + end + 3..]).unwrap_or("");
    }
    text.push_str(rest);
    text
}

pub fn unescape(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Value of an attribute of the first element with this name.
pub fn attribute(xml: &str, element: &str, attribute: &str) -> Option<String> {
    let open = format!("<{element}");
    let start = xml.match_indices(&open).map(|(at, _)| at + open.len()).find(|&at| xml[at..].starts_with(char::is_whitespace))?;
    let tag = &xml[start..start + xml[start..].find('>')?];
    let marker = format!("{attribute}=\"");
    let value = tag.match_indices(&marker).find(|(at, _)| tag[..*at].ends_with(char::is_whitespace)).map(|(at, _)| at + marker.len())?;
    tag[value..].split('"').next().map(unescape)
}

/// Text of every element with this name that has no attributes.
pub fn elements<'a>(xml: &'a str, element: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{element}>"), format!("</{element}>"));
    xml.split(open.as_str()).skip(1).filter_map(|rest| rest.split(close.as_str()).next()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_attributes_and_elements_and_skips_comments() {
        let xml = without_comments(
            r#"<Game configVersion="1">
                 <Identity Name="Microsoft.4297127D64EC6" Publisher="CN=Microsoft Corporation" />
                 <!-- <Identity Name="Example" /> OverrideDisplayName="Xbox Game Override" -->
                 <ShellVisuals OverrideDisplayName="Wrong" DefaultDisplayName="Ratchet &amp; Clank" />
                 <Path>first</Path><Path>second</Path>
               </Game>"#,
        );
        assert_eq!(attribute(&xml, "Identity", "Name").as_deref(), Some("Microsoft.4297127D64EC6"));
        assert_eq!(attribute(&xml, "ShellVisuals", "DefaultDisplayName").as_deref(), Some("Ratchet & Clank"));
        assert_eq!(attribute(&xml, "ShellVisuals", "DisplayName"), None);
        assert_eq!(attribute(&xml, "Missing", "Name"), None);
        assert_eq!(elements(&xml, "Path"), ["first", "second"]);
        assert!(elements(&xml, "Missing").is_empty());
    }
}
