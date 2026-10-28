//! Reading Evernote export files (`.enex`): one `<note>` at a time, so a
//! notebook with hundreds of megabytes of attachments never has to fit in
//! memory at once.
//!
//! ```text
//! <en-export>
//!   <note>
//!     <title>…</title> <created>20230115T093000Z</created> <updated>…</updated>
//!     <tag>…</tag>…
//!     <content><![CDATA[<en-note>…ENML…</en-note>]]></content>
//!     <resource>
//!       <data encoding="base64">…</data> <mime>image/png</mime>
//!       <resource-attributes><file-name>photo.png</file-name></resource-attributes>
//!     </resource>…
//!   </note>…
//! </en-export>
//! ```

use std::io::BufRead;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use quick_xml::Reader;
use quick_xml::escape::resolve_html5_entity;
use quick_xml::events::Event;

use super::ImportError;

/// One note as Evernote exported it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EnexNote {
    pub title: String,
    /// Unix ms.
    pub created: Option<i64>,
    pub updated: Option<i64>,
    pub tags: Vec<String>,
    /// ENML (Evernote's XHTML), the `<en-note>` document.
    pub content: String,
    pub resources: Vec<EnexResource>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EnexResource {
    pub data: Vec<u8>,
    pub mime: String,
    pub file_name: Option<String>,
}

/// Calls `each` for every note in the file, in order.
pub fn read_notes(
    input: impl BufRead,
    mut each: impl FnMut(EnexNote) -> Result<(), ImportError>,
) -> Result<(), ImportError> {
    let mut reader = Reader::from_reader(input);
    let config = reader.config_mut();
    config.check_end_names = false;
    config.allow_unmatched_ends = true;

    let mut buf = Vec::new();
    // Element names from the root down to the current one.
    let mut path: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut note: Option<EnexNote> = None;
    let mut resource: Option<EnexResource> = None;
    let mut seen_root = false;

    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| ImportError::Format(format!("not a valid Evernote export: {e}")))?;
        match event {
            Event::Start(e) => {
                let name = e.name().as_ref().to_ascii_lowercase();
                seen_root |= name == "en-export";
                match name.as_str() {
                    "note" => note = Some(EnexNote::default()),
                    "resource" if note.is_some() => resource = Some(EnexResource::default()),
                    _ => {}
                }
                path.push(name);
                text.clear();
            }
            Event::End(_) => {
                let Some(name) = path.pop() else { continue };
                let value = std::mem::take(&mut text);
                let parent = path.last().map(String::as_str);
                match (name.as_str(), parent) {
                    ("note", _) => {
                        if let Some(done) = note.take() {
                            each(done)?;
                        }
                    }
                    ("resource", _) => {
                        if let (Some(n), Some(r)) = (note.as_mut(), resource.take()) {
                            n.resources.push(r);
                        }
                    }
                    ("title", Some("note")) => {
                        set(&mut note, |n| n.title = value.trim().to_owned())
                    }
                    ("created", Some("note")) => set(&mut note, |n| n.created = parse_date(&value)),
                    ("updated", Some("note")) => set(&mut note, |n| n.updated = parse_date(&value)),
                    ("tag", Some("note")) => {
                        let tag = value.trim().to_owned();
                        if !tag.is_empty() {
                            set(&mut note, |n| n.tags.push(tag));
                        }
                    }
                    ("content", Some("note")) => set(&mut note, |n| n.content = value),
                    ("data", Some("resource")) => {
                        if let Some(r) = resource.as_mut() {
                            let compact: String =
                                value.chars().filter(|c| !c.is_ascii_whitespace()).collect();
                            r.data = BASE64.decode(compact).map_err(|e| {
                                ImportError::Format(format!("an attachment's data is damaged: {e}"))
                            })?;
                        }
                    }
                    ("mime", Some("resource")) => {
                        if let Some(r) = resource.as_mut() {
                            r.mime = value.trim().to_ascii_lowercase();
                        }
                    }
                    ("file-name", Some("resource-attributes")) => {
                        if let Some(r) = resource.as_mut() {
                            let name = value.trim();
                            r.file_name = (!name.is_empty()).then(|| name.to_owned());
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(e) => text.push_str(&e.xml10_content()),
            Event::CData(e) => text.push_str(&e.xml10_content()),
            Event::GeneralRef(e) => {
                if let Ok(Some(c)) = e.resolve_char_ref() {
                    text.push(c);
                } else {
                    let name = e.into_inner();
                    match resolve_html5_entity(&name) {
                        Some(s) => text.push_str(s),
                        None => {
                            text.push('&');
                            text.push_str(&name);
                            text.push(';');
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    if !seen_root {
        return Err(ImportError::Format(
            "this isn't an Evernote export (.enex) file".into(),
        ));
    }
    Ok(())
}

fn set(note: &mut Option<EnexNote>, f: impl FnOnce(&mut EnexNote)) {
    if let Some(n) = note.as_mut() {
        f(n);
    }
}

/// `20230115T093000Z` → unix ms (UTC).
pub(crate) fn parse_date(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 15 || b[8] != b'T' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(4..6)?, num(6..8)?);
    let (h, mi, sec) = (num(9..11)?, num(11..13)?, num(13..15)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    Some(((days_from_civil(y, mo, d) * 24 + h) * 60 + mi) * 60_000 + sec * 1000)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(parse_date("19700101T000000Z"), Some(0));
        assert_eq!(parse_date("20000229T120000Z"), Some(951_825_600_000));
        assert_eq!(parse_date("20261025T200640Z"), Some(1_792_958_800_000));
        assert_eq!(parse_date("garbage"), None);
        assert_eq!(parse_date("20231340T000000Z"), None);
    }

    #[test]
    fn reads_notes_one_by_one() {
        let enex = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE en-export SYSTEM "http://xml.evernote.com/pub/evernote-export4.dtd">
<en-export export-date="20240101T000000Z" application="Evernote" version="10.0">
  <note>
    <title>Shopping &amp; errands</title>
    <created>20230115T093000Z</created>
    <updated>20230116T100000Z</updated>
    <tag>home</tag><tag>todo</tag>
    <note-attributes><author>me</author></note-attributes>
    <content><![CDATA[<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE en-note SYSTEM "http://xml.evernote.com/pub/enml2.dtd"><en-note><div>Milk</div></en-note>]]></content>
    <resource>
      <data encoding="base64">
aGVs
bG8=
      </data>
      <mime>text/plain</mime>
      <resource-attributes><file-name>hello.txt</file-name></resource-attributes>
    </resource>
  </note>
  <note><title>Second</title><content><![CDATA[<en-note/>]]></content></note>
</en-export>"#;
        let mut notes = Vec::new();
        read_notes(enex.as_bytes(), |n| {
            notes.push(n);
            Ok(())
        })
        .unwrap();
        assert_eq!(notes.len(), 2);
        let n = &notes[0];
        assert_eq!(n.title, "Shopping & errands");
        assert_eq!(n.created, parse_date("20230115T093000Z"));
        assert_eq!(n.tags, ["home", "todo"]);
        assert!(n.content.contains("<div>Milk</div>"));
        assert_eq!(n.resources.len(), 1);
        assert_eq!(n.resources[0].data, b"hello");
        assert_eq!(n.resources[0].mime, "text/plain");
        assert_eq!(n.resources[0].file_name.as_deref(), Some("hello.txt"));
        assert_eq!(notes[1].title, "Second");

        assert!(matches!(
            read_notes("<html><body>nope</body></html>".as_bytes(), |_| Ok(())),
            Err(ImportError::Format(_))
        ));
    }
}
