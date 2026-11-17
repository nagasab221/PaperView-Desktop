//! A tiny XML tree for S3 and WebDAV listings: elements by local name (the
//! namespace prefix is dropped — `d:href`, `D:href` and `href` all read as
//! `href`), with their text.

use quick_xml::events::Event;

#[derive(Debug, Default)]
pub struct Node {
    pub name: String,
    pub text: String,
    pub children: Vec<Node>,
}

impl Node {
    /// The first child called `name`.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Every child called `name`.
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// The text of the first child called `name` ("" when missing).
    pub fn text_of(&self, name: &str) -> &str {
        self.child(name).map(|c| c.text.trim()).unwrap_or("")
    }

    /// The first descendant called `name`, depth first.
    pub fn find(&self, name: &str) -> Option<&Node> {
        self.children
            .iter()
            .find_map(|c| if c.name == name { Some(c) } else { c.find(name) })
    }
}

fn local(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).into_owned()
}

/// Parses a document into its root element.
pub fn parse(body: &str) -> Result<Node, String> {
    let mut reader = quick_xml::Reader::from_str(body);
    let mut stack: Vec<Node> = vec![Node::default()];
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("unreadable XML: {e}"))?;
        match event {
            Event::Start(e) => stack.push(Node {
                name: local(e.local_name().as_ref()),
                ..Default::default()
            }),
            Event::Empty(e) => {
                let node = Node {
                    name: local(e.local_name().as_ref()),
                    ..Default::default()
                };
                if let Some(top) = stack.last_mut() {
                    top.children.push(node);
                }
            }
            Event::End(_) => {
                if stack.len() > 1 {
                    let node = stack.pop().unwrap_or_default();
                    if let Some(top) = stack.last_mut() {
                        top.children.push(node);
                    }
                }
            }
            Event::Text(t) => {
                if let (Some(top), Ok(s)) = (stack.last_mut(), t.decode()) {
                    top.text.push_str(&s);
                }
            }
            Event::CData(t) => {
                if let (Some(top), Ok(s)) = (stack.last_mut(), t.decode()) {
                    top.text.push_str(&s);
                }
            }
            Event::GeneralRef(r) => {
                let ch = match r.resolve_char_ref() {
                    Ok(Some(c)) => Some(c),
                    _ => match r.decode().as_deref() {
                        Ok("amp") => Some('&'),
                        Ok("lt") => Some('<'),
                        Ok("gt") => Some('>'),
                        Ok("quot") => Some('"'),
                        Ok("apos") => Some('\''),
                        _ => None,
                    },
                };
                if let (Some(top), Some(c)) = (stack.last_mut(), ch) {
                    top.text.push(c);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut doc = stack.into_iter().next().unwrap_or_default();
    doc.children
        .pop()
        .ok_or_else(|| "empty XML document".to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn namespaces_and_entities() {
        let doc = super::parse(
            r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:"><d:response>
            <d:href>/a/Tom%20&amp;%20Jerry.pdf</d:href><d:propstat><d:prop>
            <d:resourcetype/><d:getcontentlength>12</d:getcontentlength></d:prop>
            </d:propstat></d:response></d:multistatus>"#,
        )
        .unwrap();
        assert_eq!(doc.name, "multistatus");
        let r = doc.child("response").unwrap();
        assert_eq!(r.text_of("href"), "/a/Tom%20&%20Jerry.pdf");
        assert_eq!(r.find("getcontentlength").unwrap().text, "12");
        assert!(r.find("resourcetype").unwrap().children.is_empty());
    }
}
