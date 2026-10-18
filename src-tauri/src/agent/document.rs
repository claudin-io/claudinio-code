//! The living document's format (lean-harness plan, Phase 4b): parse, render,
//! and the edits the harness makes to one `.md` file a session owns.
//!
//! Pure: no file system, no session. `doc_session` owns the file, the lock and
//! what the agent last saw; this module only turns text into sections and back.
//!
//! The format is plain Markdown any editor opens. A *placeholder* is a line
//! whose trimmed content is `[topic]` — square brackets and nothing else, so a
//! link (`[x](y)`) or a task-list item (`- [ ] x`) is never one. The harness
//! owns one HTML comment per section, invisible when the file is rendered:
//!
//! ```md
//! # Pudim de leite
//!
//! ## origem do pudim de leite
//! <!-- claudinio: id=s1 status=done by=main -->
//! O pudim chegou ao Brasil com os portugueses…
//!
//! [ingredientes originais] <!-- claudinio: id=s2 status=doing by=sub:ingredientes -->
//! [Modo de preparo clássico] <!-- claudinio: id=s3 status=todo -->
//! ```
//!
//! An unwritten placeholder keeps its line and gains the comment; a written
//! section becomes a `## topic` heading, the comment on the next line, and its
//! body up to the next section, placeholder or level-1/2 heading. Everything
//! else — the title, prose the user wrote, extra headings — is carried as it
//! was, byte for byte, which is what lets a person edit the file in any editor
//! and the harness parse it back without losing a character
//! (`parse(md)` then `render` returns `md`).

/// Who last moved a section, and where it stands. `Human` is set by the
/// harness when the user edited the section's text (in the app or any
/// editor): their edit is the section's content until an agent re-reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Todo,
    Doing,
    Done,
    Human,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::Doing => "doing",
            Status::Done => "done",
            Status::Human => "human",
        }
    }

    /// An unknown word reads as `todo`: a person who mistyped a status in the
    /// comment loses nothing, and the next harness write corrects it.
    pub fn parse(s: &str) -> Status {
        match s {
            "doing" => Status::Doing,
            "done" => Status::Done,
            "human" => Status::Human,
            _ => Status::Todo,
        }
    }
}

/// One section: a placeholder, or a written heading with its body.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// `s<N>`, or empty for a placeholder a person added that has no comment
    /// yet ([`Document::assign_ids`] gives it one).
    pub id: String,
    pub topic: String,
    pub status: Status,
    /// `main`, or `sub:<name>` for a subagent.
    pub owner: Option<String>,
    /// The text under the heading, as it is in the file (newlines included).
    /// Always empty for a placeholder.
    pub body: String,
    /// A `## topic` heading (true) or a `[topic]` placeholder line (false).
    pub written: bool,
    /// The header line(s) exactly as read, and the fields they parsed to:
    /// rendered verbatim while the fields still match, so an untouched
    /// section keeps the user's spacing.
    header_raw: String,
    header_fields: Option<HeaderFields>,
}

type HeaderFields = (String, String, Status, Option<String>, bool);

impl Section {
    fn fields(&self) -> HeaderFields {
        (
            self.id.clone(),
            self.topic.clone(),
            self.status,
            self.owner.clone(),
            self.written,
        )
    }

    fn comment(&self) -> String {
        let mut c = format!(
            "<!-- claudinio: id={} status={}",
            self.id,
            self.status.as_str()
        );
        if let Some(owner) = &self.owner {
            c.push_str(&format!(" by={owner}"));
        }
        c.push_str(" -->");
        c
    }

    fn header(&self) -> String {
        if self.header_fields.as_ref() == Some(&self.fields()) {
            return self.header_raw.clone();
        }
        if self.written {
            format!("## {}\n{}\n", self.topic, self.comment())
        } else if self.id.is_empty() {
            format!("[{}]\n", self.topic)
        } else {
            format!("[{}] {}\n", self.topic, self.comment())
        }
    }

    /// The body with surrounding blank space removed: what counts when the
    /// harness asks whether a section's text changed.
    pub fn text(&self) -> &str {
        self.body.trim()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// Anything outside a section, verbatim.
    Prose(String),
    Section(Section),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// The first `# ` line's text, or empty.
    pub title: String,
    pub blocks: Vec<Block>,
    /// The lowest number a new id may take ([`Document::reserve_ids_of`]).
    /// Not part of the file.
    floor: u32,
}

/// Why a write was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteError {
    /// No section has this id; the ids that exist, in order.
    NoSuchSection { ids: Vec<String> },
    /// Another agent is writing it (`status=doing` under a different owner).
    Claimed { owner: String },
    /// `done` with nothing written: a finished section has a body.
    EmptyBody,
}

/// How a section differs between two readings of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Removed,
    Edited,
}

impl ChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeKind::Added => "added",
            ChangeKind::Removed => "removed",
            ChangeKind::Edited => "edited",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Change {
    pub id: String,
    pub topic: String,
    #[serde(with = "change_kind_serde")]
    pub kind: ChangeKind,
}

mod change_kind_serde {
    use super::ChangeKind;
    pub fn serialize<S: serde::Serializer>(k: &ChangeKind, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(k.as_str())
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<ChangeKind, D::Error> {
        let s: String = serde::Deserialize::deserialize(d)?;
        Ok(match s.as_str() {
            "added" => ChangeKind::Added,
            "removed" => ChangeKind::Removed,
            _ => ChangeKind::Edited,
        })
    }
}

/// A line's text without its line ending.
fn bare(line: &str) -> &str {
    line.trim_end_matches(['\n', '\r'])
}

fn is_fence(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

/// `# x` or `## x`: the headings that end a section's body. `###` and deeper
/// belong to the section they are in.
fn is_top_heading(line: &str) -> bool {
    let t = bare(line);
    t.starts_with("# ") || t.starts_with("## ")
}

/// `<!-- claudinio: id=s1 status=done by=main -->`, read into its fields.
fn parse_comment(text: &str) -> Option<(String, Status, Option<String>)> {
    let inner = text
        .trim()
        .strip_prefix("<!--")?
        .strip_suffix("-->")?
        .trim()
        .strip_prefix("claudinio:")?;
    let (mut id, mut status, mut owner) = (String::new(), Status::Todo, None);
    for pair in inner.split_whitespace() {
        match pair.split_once('=') {
            Some(("id", v)) => id = v.to_string(),
            Some(("status", v)) => status = Status::parse(v),
            Some(("by", v)) if !v.is_empty() => owner = Some(v.to_string()),
            _ => {}
        }
    }
    Some((id, status, owner))
}

/// A placeholder line: `[topic]`, optionally followed by the harness's
/// comment. The topic holds no bracket, so `[a] [b]` and `[x](y)` are prose.
fn parse_placeholder(line: &str) -> Option<(String, Option<(String, Status, Option<String>)>)> {
    let t = bare(line).trim();
    let rest = t.strip_prefix('[')?;
    let close = rest.find(']')?;
    let topic = &rest[..close];
    let after = rest[close + 1..].trim();
    if topic.trim().is_empty() || topic.contains('[') {
        return None;
    }
    let comment = if after.is_empty() {
        None
    } else {
        Some(parse_comment(after)?)
    };
    Some((topic.trim().to_string(), comment))
}

fn section(
    header_raw: String,
    topic: String,
    comment: Option<(String, Status, Option<String>)>,
    written: bool,
) -> Section {
    let (id, status, owner) = comment.unwrap_or((String::new(), Status::Todo, None));
    let mut s = Section {
        id,
        topic,
        status,
        owner,
        body: String::new(),
        written,
        header_raw,
        header_fields: None,
    };
    s.header_fields = Some(s.fields());
    s
}

pub fn parse(md: &str) -> Document {
    let lines: Vec<&str> = md.split_inclusive('\n').collect();
    let mut blocks: Vec<Block> = Vec::new();
    let mut prose = String::new();
    // The written section whose body is being collected.
    let mut open: Option<Section> = None;
    let mut in_fence = false;
    let mut i = 0;
    let flush = |blocks: &mut Vec<Block>, prose: &mut String, open: &mut Option<Section>| {
        if let Some(s) = open.take() {
            blocks.push(Block::Section(s));
        }
        if !prose.is_empty() {
            blocks.push(Block::Prose(std::mem::take(prose)));
        }
    };
    while i < lines.len() {
        let line = lines[i];
        if !in_fence {
            let written_header = bare(line)
                .strip_prefix("## ")
                .zip(lines.get(i + 1))
                .and_then(|(topic, next)| Some((topic, next, parse_comment(bare(next))?)));
            if let Some((topic, next, comment)) = written_header {
                flush(&mut blocks, &mut prose, &mut open);
                open = Some(section(
                    format!("{line}{next}"),
                    topic.trim().to_string(),
                    Some(comment),
                    true,
                ));
                i += 2;
                continue;
            }
            if let Some((topic, comment)) = parse_placeholder(line) {
                flush(&mut blocks, &mut prose, &mut open);
                blocks.push(Block::Section(section(
                    line.to_string(),
                    topic,
                    comment,
                    false,
                )));
                i += 1;
                continue;
            }
            if open.is_some() && is_top_heading(line) {
                flush(&mut blocks, &mut prose, &mut open);
            }
        }
        if is_fence(line) {
            in_fence = !in_fence;
        }
        match open.as_mut() {
            Some(s) => s.body.push_str(line),
            None => prose.push_str(line),
        }
        i += 1;
    }
    flush(&mut blocks, &mut prose, &mut open);
    let title = blocks
        .iter()
        .filter_map(|b| match b {
            Block::Prose(p) => Some(p),
            Block::Section(_) => None,
        })
        .flat_map(|p| p.lines())
        .find_map(|l| l.strip_prefix("# "))
        .map(|t| t.trim().to_string())
        .unwrap_or_default();
    Document {
        title,
        blocks,
        floor: 0,
    }
}

pub fn render(doc: &Document) -> String {
    let mut out = String::new();
    for b in &doc.blocks {
        match b {
            Block::Prose(p) => out.push_str(p),
            Block::Section(s) => {
                out.push_str(&s.header());
                out.push_str(&s.body);
            }
        }
    }
    out
}

/// A new placeholder section (no header read from a file).
fn placeholder(id: String, topic: &str) -> Section {
    Section {
        id,
        topic: topic.split_whitespace().collect::<Vec<_>>().join(" "),
        status: Status::Todo,
        owner: None,
        body: String::new(),
        written: false,
        header_raw: String::new(),
        header_fields: None,
    }
}

/// A topic as a placeholder line can hold it: one line, no brackets.
fn clean_topic(topic: &str) -> String {
    topic
        .replace(['[', ']'], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A model-written body made safe to sit inside a section: a level-1/2
/// heading would end the section early and a `[x]` line would open a new
/// one, so outside code fences the first becomes `###` and the second is
/// escaped (`\[x]` renders as `[x]`).
fn normalize_body(body: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in body.trim_end().lines() {
        if is_fence(line) {
            in_fence = !in_fence;
            out.push_str(line);
        } else if in_fence {
            out.push_str(line);
        } else if let Some(rest) = line.strip_prefix("## ").or_else(|| line.strip_prefix("# ")) {
            out.push_str("### ");
            out.push_str(rest);
        } else if parse_placeholder(line).is_some() || parse_comment(line).is_some() {
            let indent = &line[..line.len() - line.trim_start().len()];
            out.push_str(indent);
            out.push('\\');
            out.push_str(line.trim_start());
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out.push('\n');
    out
}

/// A new document: the title, then one tagged placeholder per topic.
pub fn scaffold(title: &str, topics: &[String]) -> Document {
    let mut blocks = vec![Block::Prose(format!("# {}\n\n", title.trim()))];
    let mut n = 0;
    for t in topics {
        let topic = clean_topic(t);
        if topic.is_empty() {
            continue;
        }
        n += 1;
        blocks.push(Block::Section(placeholder(format!("s{n}"), &topic)));
    }
    Document {
        title: title.trim().to_string(),
        blocks,
        floor: 0,
    }
}

/// The sections whose topic or text differ between `old` and `new`, by id,
/// in `new`'s order and then the removed ones in `old`'s. A change to a
/// status or an owner is the harness's own bookkeeping, not an edit.
pub fn diff(old: &Document, new: &Document) -> Vec<Change> {
    let mut out = Vec::new();
    for s in new.sections() {
        let kind = match old.section(&s.id) {
            None => ChangeKind::Added,
            Some(o) if o.topic != s.topic || o.text() != s.text() => ChangeKind::Edited,
            Some(_) => continue,
        };
        out.push(Change {
            id: s.id.clone(),
            topic: s.topic.clone(),
            kind,
        });
    }
    for o in old.sections() {
        if new.section(&o.id).is_none() {
            out.push(Change {
                id: o.id.clone(),
                topic: o.topic.clone(),
                kind: ChangeKind::Removed,
            });
        }
    }
    out
}

/// What a person saved from the app, merged with what the agents wrote while
/// they had the file open. `base` is the text the editor loaded, `mine` what
/// they saved, `theirs` the file as it is now. A section the person changed
/// is theirs to keep (status `human`, no owner) — the human wins; one they
/// did not touch takes the file's current version, so a section a subagent
/// finished meanwhile is not rolled back; a section an agent added meanwhile
/// is kept after the section it followed. Prose comes from `mine`.
pub fn merge_human_save(base: &Document, mine: &Document, theirs: &Document) -> Document {
    let mut out = mine.clone();
    for s in out.sections_mut() {
        let (Some(b), Some(t)) = (base.section(&s.id), theirs.section(&s.id)) else {
            continue;
        };
        if s.id.is_empty() {
            continue;
        }
        if b.topic == s.topic && b.text() == s.text() {
            *s = t.clone();
        } else {
            s.status = Status::Human;
            s.owner = None;
        }
    }
    let mut prev: Option<String> = None;
    for t in theirs.sections() {
        if base.section(&t.id).is_none() && out.section(&t.id).is_none() {
            let at = prev
                .as_deref()
                .and_then(|p| {
                    out.blocks
                        .iter()
                        .position(|b| matches!(b, Block::Section(s) if s.id == p))
                })
                .map_or(out.blocks.len(), |i| i + 1);
            out.blocks.insert(at, Block::Section(t.clone()));
        }
        prev = Some(t.id.clone());
    }
    out
}

impl Document {
    pub fn sections(&self) -> impl Iterator<Item = &Section> {
        self.blocks.iter().filter_map(|b| match b {
            Block::Section(s) => Some(s),
            Block::Prose(_) => None,
        })
    }

    pub fn sections_mut(&mut self) -> impl Iterator<Item = &mut Section> {
        self.blocks.iter_mut().filter_map(|b| match b {
            Block::Section(s) => Some(s),
            Block::Prose(_) => None,
        })
    }

    pub fn section(&self, id: &str) -> Option<&Section> {
        self.sections().find(|s| s.id == id)
    }

    pub fn section_mut(&mut self, id: &str) -> Option<&mut Section> {
        self.sections_mut().find(|s| s.id == id)
    }

    pub fn ids(&self) -> Vec<String> {
        self.sections().map(|s| s.id.clone()).collect()
    }

    /// Give every section without an id (a placeholder a person typed), and
    /// every repeated id (a tagged line copied), the next free `s<N>`.
    /// Returns the ids it handed out.
    pub fn assign_ids(&mut self) -> Vec<String> {
        let mut next = self.next_number();
        let mut seen: Vec<String> = Vec::new();
        let mut assigned = Vec::new();
        for s in self.sections_mut() {
            if s.id.is_empty() || seen.contains(&s.id) {
                s.id = format!("s{next}");
                next += 1;
                assigned.push(s.id.clone());
            }
            seen.push(s.id.clone());
        }
        assigned
    }

    /// One more than the highest `s<N>` in the document, and never below
    /// the floor an earlier reading set.
    fn next_number(&self) -> u32 {
        let highest = self
            .sections()
            .filter_map(|s| s.id.strip_prefix('s')?.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        (highest + 1).max(self.floor)
    }

    /// New ids continue after every id `earlier` had. A section a person
    /// deleted keeps its number: the model may still know `s3` as the topic
    /// it was, and a new placeholder answering to it would read as an edit.
    pub fn reserve_ids_of(&mut self, earlier: &Document) {
        self.floor = self.floor.max(earlier.next_number());
    }

    /// Claim and write section `id` for `owner`. `body` empty or absent with
    /// `Doing` claims it without touching its text.
    pub fn apply_write(
        &mut self,
        id: &str,
        body: Option<&str>,
        owner: &str,
        status: Status,
    ) -> Result<(), WriteError> {
        let ids = self.ids();
        let s = self
            .section_mut(id)
            .ok_or(WriteError::NoSuchSection { ids })?;
        if s.status == Status::Doing
            && let Some(holder) = s.owner.as_deref().filter(|o| *o != owner)
        {
            return Err(WriteError::Claimed {
                owner: holder.to_string(),
            });
        }
        let body = body.map(str::trim).filter(|b| !b.is_empty());
        if body.is_none() && status == Status::Done && s.text().is_empty() {
            return Err(WriteError::EmptyBody);
        }
        if let Some(body) = body {
            s.body = normalize_body(body);
            s.written = true;
        }
        s.status = status;
        s.owner = Some(owner.to_string());
        Ok(())
    }

    /// Insert a placeholder for `topic` after section `after` (at the end
    /// when `None`), and return its id.
    pub fn add_placeholder(&mut self, topic: &str, after: Option<&str>) -> Result<String, String> {
        let topic = clean_topic(topic);
        if topic.is_empty() {
            return Err("the topic is empty".into());
        }
        let at = match after {
            Some(after) => {
                self.blocks
                    .iter()
                    .position(|b| matches!(b, Block::Section(s) if s.id == after))
                    .ok_or_else(|| {
                        format!(
                            "no section has id '{after}'; the ids are: {}",
                            self.ids().join(", ")
                        )
                    })?
                    + 1
            }
            None => self.blocks.len(),
        };
        // After a section that ends without a newline (the file's last line),
        // the new line must start on a line of its own.
        if let Some(Block::Section(prev)) = at.checked_sub(1).and_then(|i| self.blocks.get_mut(i))
            && prev.written
            && !prev.body.is_empty()
            && !prev.body.ends_with('\n')
        {
            prev.body.push('\n');
        }
        if let Some(Block::Prose(prev)) = at.checked_sub(1).and_then(|i| self.blocks.get_mut(i))
            && !prev.is_empty()
            && !prev.ends_with('\n')
        {
            prev.push('\n');
        }
        let id = format!("s{}", self.next_number());
        self.blocks
            .insert(at, Block::Section(placeholder(id.clone(), &topic)));
        Ok(id)
    }

    /// The title, then one line per section: `id · status · owner · topic`.
    /// What the model carries instead of the bodies.
    pub fn outline(&self) -> String {
        let mut out = format!("# {}", self.title);
        for s in self.sections() {
            out.push_str(&format!(
                "\n{} \u{b7} {} \u{b7} {} \u{b7} {}",
                s.id,
                s.status.as_str(),
                s.owner.as_deref().unwrap_or("-"),
                s.topic
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUDIM: &str = "# Pudim de leite\n\n[origem do pudim de leite]\n[ingredientes originais]\n[Modo de preparo clássico]\n[Como servir o pudim]\n[Como armazenar o pudim]\n";

    fn topics(doc: &Document) -> Vec<&str> {
        doc.sections().map(|s| s.topic.as_str()).collect()
    }

    #[test]
    fn the_plans_pudim_example_parses_into_five_placeholders() {
        let doc = parse(PUDIM);
        assert_eq!(doc.title, "Pudim de leite");
        assert_eq!(
            topics(&doc),
            [
                "origem do pudim de leite",
                "ingredientes originais",
                "Modo de preparo clássico",
                "Como servir o pudim",
                "Como armazenar o pudim"
            ]
        );
        assert!(doc.sections().all(|s| s.id.is_empty() && !s.written));
        assert!(doc.sections().all(|s| s.status == Status::Todo));
    }

    #[test]
    fn a_link_a_task_item_or_an_empty_bracket_is_not_a_placeholder() {
        let doc = parse(
            "# T\n\n[docs](https://x.y)\n- [ ] item\n[ ]\n[]\n[a] [b]\nsee [here]\n  [indented topic]  \n",
        );
        assert_eq!(topics(&doc), ["indented topic"]);
    }

    #[test]
    fn parse_then_render_returns_the_file_byte_for_byte() {
        let files = [
            PUDIM,
            "",
            "no title, no newline at the end",
            "# T\r\n\r\n[a]\r\nprose\r\n",
            "# T\n\n## written\n<!-- claudinio: id=s1 status=done by=main -->\nbody line\n\n### sub heading\nmore\n\n[next]   <!--  claudinio: id=s2 status=todo  -->\n\n## Notes by me\nfree prose\n",
            "# T\n\n## code\n<!-- claudinio: id=s1 status=done by=sub:x -->\n```md\n## not a heading\n[not a placeholder]\n```\nafter\n",
        ];
        for md in files {
            assert_eq!(render(&parse(md)), md, "{md:?}");
        }
    }

    #[test]
    fn parse_render_parse_is_the_identity_on_sections() {
        let mut doc = parse(PUDIM);
        doc.assign_ids();
        doc.apply_write("s2", Some("- leite\n- ovos"), "main", Status::Done)
            .unwrap();
        let once = parse(&render(&doc));
        let twice = parse(&render(&once));
        assert_eq!(once, twice);
        assert_eq!(render(&once), render(&doc));
        let s2 = once.section("s2").unwrap();
        assert!(s2.written);
        assert_eq!(s2.text(), "- leite\n- ovos");
        assert_eq!(s2.owner.as_deref(), Some("main"));
        assert_eq!(s2.status, Status::Done);
    }

    #[test]
    fn a_scaffold_is_the_title_and_one_tagged_placeholder_per_topic() {
        let doc = scaffold(
            "Pudim de leite",
            &["origem".to_string(), "ingredientes".to_string()],
        );
        assert_eq!(
            render(&doc),
            "# Pudim de leite\n\n[origem] <!-- claudinio: id=s1 status=todo -->\n\
             [ingredientes] <!-- claudinio: id=s2 status=todo -->\n"
        );
    }

    #[test]
    fn a_written_section_is_a_heading_a_comment_and_its_body() {
        let mut doc = scaffold("T", &["a".to_string(), "b".to_string()]);
        doc.apply_write("s1", Some("first\nsecond\n"), "sub:pesquisa", Status::Done)
            .unwrap();
        assert_eq!(
            render(&doc),
            "# T\n\n## a\n<!-- claudinio: id=s1 status=done by=sub:pesquisa -->\nfirst\nsecond\n\n\
             [b] <!-- claudinio: id=s2 status=todo -->\n"
        );
    }

    #[test]
    fn a_hand_edited_file_round_trips_and_keeps_its_ids() {
        let mut doc = scaffold(
            "Pudim",
            &[
                "origem".to_string(),
                "ingredientes".to_string(),
                "servir".to_string(),
            ],
        );
        doc.apply_write("s1", Some("Veio de Portugal."), "main", Status::Done)
            .unwrap();
        doc.apply_write("s3", Some("Gelado."), "main", Status::Done)
            .unwrap();
        let agent = render(&doc);
        // The user, in any editor: rewrites s1's body, adds free prose and a
        // placeholder between s2 and s3, and a note at the end.
        let edited = agent
            .replace("Veio de Portugal.", "Veio de Portugal, no século XVIII.")
            .replace(
                "## servir",
                "Uma nota minha sobre o caramelo.\n[caramelo]\n## servir",
            )
            + "\n## Minhas notas\nfazer no domingo\n";
        let mut back = parse(&edited);
        assert_eq!(render(&back), edited, "the hand edit itself survives");
        assert_eq!(
            topics(&back),
            ["origem", "ingredientes", "caramelo", "servir"]
        );
        assert_eq!(back.assign_ids(), ["s4"]);
        assert_eq!(
            back.ids(),
            ["s1", "s2", "s4", "s3"],
            "existing ids are stable"
        );
        let out = render(&back);
        assert!(out.contains(
            "Uma nota minha sobre o caramelo.\n[caramelo] <!-- claudinio: id=s4 status=todo -->\n"
        ));
        assert!(out.ends_with("\n## Minhas notas\nfazer no domingo\n"));
        assert_eq!(
            back.section("s1").unwrap().text(),
            "Veio de Portugal, no século XVIII."
        );
        assert_eq!(
            back.section("s3").unwrap().text(),
            "Gelado.",
            "a level-2 heading ends the last section's body"
        );
    }

    #[test]
    fn a_copied_tagged_line_gets_a_fresh_id() {
        let mut doc = parse(
            "# T\n[a] <!-- claudinio: id=s1 status=todo -->\n[b] <!-- claudinio: id=s1 status=todo -->\n",
        );
        assert_eq!(doc.assign_ids(), ["s2"]);
        assert_eq!(doc.ids(), ["s1", "s2"]);
    }

    #[test]
    fn a_section_another_agent_is_writing_cannot_be_claimed() {
        let mut doc = scaffold("T", &["a".to_string()]);
        doc.apply_write("s1", None, "sub:one", Status::Doing)
            .unwrap();
        assert_eq!(
            doc.apply_write("s1", Some("mine"), "sub:two", Status::Done),
            Err(WriteError::Claimed {
                owner: "sub:one".into()
            })
        );
        assert_eq!(
            doc.apply_write("s1", None, "main", Status::Doing),
            Err(WriteError::Claimed {
                owner: "sub:one".into()
            })
        );
        // The holder itself writes and releases it.
        doc.apply_write("s1", Some("done"), "sub:one", Status::Done)
            .unwrap();
        // A finished section can be taken over.
        doc.apply_write("s1", None, "main", Status::Doing).unwrap();
        assert_eq!(doc.section("s1").unwrap().owner.as_deref(), Some("main"));
        assert_eq!(
            doc.section("s1").unwrap().text(),
            "done",
            "a claim keeps the text"
        );
    }

    #[test]
    fn writing_an_unknown_section_lists_the_ids_and_done_needs_a_body() {
        let mut doc = scaffold("T", &["a".to_string(), "b".to_string()]);
        assert_eq!(
            doc.apply_write("s9", Some("x"), "main", Status::Done),
            Err(WriteError::NoSuchSection {
                ids: vec!["s1".into(), "s2".into()]
            })
        );
        assert_eq!(
            doc.apply_write("s1", Some("  \n"), "main", Status::Done),
            Err(WriteError::EmptyBody)
        );
    }

    #[test]
    fn a_body_cannot_open_a_section_of_its_own() {
        let mut doc = scaffold("T", &["a".to_string(), "b".to_string()]);
        doc.apply_write(
            "s1",
            Some("## Big heading\n[1]\n# Top\n```\n## kept in code\n```"),
            "main",
            Status::Done,
        )
        .unwrap();
        let back = parse(&render(&doc));
        assert_eq!(back.ids(), ["s1", "s2"]);
        assert_eq!(
            back.section("s1").unwrap().text(),
            "### Big heading\n\\[1]\n### Top\n```\n## kept in code\n```"
        );
    }

    #[test]
    fn a_placeholder_is_added_after_a_section_or_at_the_end() {
        let mut doc = scaffold("T", &["a".to_string(), "b".to_string()]);
        assert_eq!(doc.add_placeholder("mid", Some("s1")).unwrap(), "s3");
        assert_eq!(doc.add_placeholder("last", None).unwrap(), "s4");
        assert_eq!(topics(&doc), ["a", "mid", "b", "last"]);
        assert!(doc.add_placeholder("x", Some("s9")).is_err());
        assert!(doc.add_placeholder("  ", None).is_err());
        let back = parse(&render(&doc));
        assert_eq!(back.ids(), ["s1", "s3", "s2", "s4"]);
    }

    #[test]
    fn the_outline_is_one_line_per_section() {
        let mut doc = scaffold("Pudim", &["origem".to_string(), "servir".to_string()]);
        doc.apply_write("s1", Some("x"), "main", Status::Done)
            .unwrap();
        doc.apply_write("s2", None, "sub:servir", Status::Doing)
            .unwrap();
        assert_eq!(
            doc.outline(),
            "# Pudim\ns1 \u{b7} done \u{b7} main \u{b7} origem\ns2 \u{b7} doing \u{b7} sub:servir \u{b7} servir"
        );
        let empty = scaffold("T", &["a".to_string()]);
        assert_eq!(empty.outline(), "# T\ns1 \u{b7} todo \u{b7} - \u{b7} a");
    }

    #[test]
    fn diff_names_edited_added_and_removed_sections_but_not_bookkeeping() {
        let mut old = scaffold("T", &["a".to_string(), "b".to_string(), "c".to_string()]);
        old.apply_write("s1", Some("one"), "main", Status::Done)
            .unwrap();
        let mut new = old.clone();
        new.section_mut("s1").unwrap().body = "one, edited\n".into();
        new.section_mut("s2").unwrap().status = Status::Doing;
        new.section_mut("s2").unwrap().owner = Some("main".into());
        new.blocks
            .retain(|b| !matches!(b, Block::Section(s) if s.id == "s3"));
        new.reserve_ids_of(&old);
        new.add_placeholder("d", None).unwrap();
        assert_eq!(
            diff(&old, &new),
            vec![
                Change {
                    id: "s1".into(),
                    topic: "a".into(),
                    kind: ChangeKind::Edited
                },
                Change {
                    id: "s4".into(),
                    topic: "d".into(),
                    kind: ChangeKind::Added
                },
                Change {
                    id: "s3".into(),
                    topic: "c".into(),
                    kind: ChangeKind::Removed
                },
            ]
        );
        assert!(diff(&old, &old).is_empty());
        let mut renamed = old.clone();
        renamed.section_mut("s2").unwrap().topic = "b, renamed".into();
        assert_eq!(diff(&old, &renamed)[0].kind, ChangeKind::Edited);
    }

    #[test]
    fn a_save_from_the_app_keeps_the_persons_edits_and_the_agents_new_work() {
        let mut base = scaffold("T", &["a".to_string(), "b".to_string(), "c".to_string()]);
        base.apply_write("s1", Some("agent one"), "main", Status::Done)
            .unwrap();
        // While the person edits, a subagent finishes s2 and the agent adds s4.
        let mut theirs = base.clone();
        theirs
            .apply_write("s2", Some("sub wrote b"), "sub:b", Status::Done)
            .unwrap();
        theirs.add_placeholder("d", Some("s2")).unwrap();
        // The person rewrote s1 and added prose, from the text they loaded.
        let mine = parse(
            &(render(&base).replace("agent one", "the person's one") + "\nnotes at the end\n"),
        );
        let merged = merge_human_save(&base, &mine, &theirs);
        let s1 = merged.section("s1").unwrap();
        assert_eq!(s1.text(), "the person's one");
        assert_eq!(s1.status, Status::Human);
        assert_eq!(s1.owner, None);
        let s2 = merged.section("s2").unwrap();
        assert_eq!(
            s2.text(),
            "sub wrote b",
            "untouched sections take the file's version"
        );
        assert_eq!(s2.status, Status::Done);
        assert_eq!(merged.ids(), ["s1", "s2", "s4", "s3"]);
        assert!(render(&merged).ends_with("notes at the end\n"));
    }
}
