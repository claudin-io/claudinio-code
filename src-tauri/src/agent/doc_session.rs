//! The session's living document on disk (lean-harness plan, Phase 4b): which
//! file a session owns, what the agents last saw of it, and the reads and
//! writes the `doc_*` tools, the run loop and the app make to it.
//!
//! The format is `agent::document`'s; this module adds the file, a lock and a
//! memory. The memory is a small JSON file beside the session
//! (`<session>.doc.json`): the text the harness last wrote or read back
//! (`known`), the sections a person changed that no agent has re-read since
//! (`unread`), and the changes not yet told to the model (`pending`). It is
//! not in the session JSONL because that file is replayed into every request
//! and the document can be long; it is not in memory because a person edits
//! the file between runs, and across restarts.
//!
//! **The human wins.** Every read of the file first compares it with
//! `known`. A section whose text changed was edited by a person: it is marked
//! `human`, its id joins `unread`, and an agent's next `doc_write` to it is
//! refused with the section's current text, so the model merges instead of
//! overwriting. A `doc_read` of the section, or that refusal, counts as
//! having seen it. The change is also queued in `pending`, and the run loop
//! tells the model about it once, as a `<doc-changed>` note (ids and topics,
//! never bodies) in the next user message it persists — never as a
//! per-request injection, which would rewrite the cached prefix.

use crate::agent::document::{self, Change, ChangeKind, Document, Status};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// What a context may do to the session's document. `Main` by default:
/// every context the app builds is a session's own; a subagent's is set
/// where it is spawned (`subagent::run_subagent_recorded`).
#[derive(Debug, Clone, Default, PartialEq)]
pub enum DocAccess {
    /// The session itself: any section, owner `main`.
    #[default]
    Main,
    /// A subagent spawned with `section`: that one section, under `owner`
    /// (`sub:<name>`).
    Section { id: String, owner: String },
    /// A subagent without a section.
    None,
}

/// Where documents live, relative to the workspace.
pub const DOCS_DIR: &str = ".claudinio/docs";

/// What a section looks like to the panel.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionView {
    pub id: String,
    pub topic: String,
    pub status: String,
    pub owner: Option<String>,
    /// 1-based line of the section's first line in the file, so the editor
    /// can open at it.
    pub line: usize,
}

/// The document as the panel shows it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocView {
    /// Relative to the workspace.
    pub path: String,
    pub title: String,
    pub sections: Vec<SectionView>,
}

/// The tool result for a context with no linked document.
pub const NO_DOCUMENT: &str =
    "this session has no document; create one with doc_create {title, sections}";

/// One read-modify-write of a document at a time, process-wide. Parallel
/// subagents write sections of the same file, and the app saves a person's
/// edit while a run is going; the file and its memory are read and written
/// together under this lock. Never held across an `await`.
static DOC_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What the harness remembers of the document between reads.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct DocState {
    /// The document this memory is about; a different linked path starts a
    /// fresh memory.
    path: String,
    /// The file as the harness last wrote or read it.
    known: String,
    /// Sections a person changed that no agent has read since.
    unread: BTreeSet<String>,
    /// Changes not yet told to the model, oldest first, one per section.
    pending: Vec<Change>,
    /// The lowest number a new section id may take: ids of sections a
    /// person deleted are never handed out again.
    #[serde(default)]
    next_id: u32,
    /// For each section a subagent holds: how it stood before the claim, so
    /// a subagent that writes nothing gives it back as it found it.
    #[serde(default)]
    claimed: BTreeMap<String, BeforeClaim>,
}

/// A section as it stood when a subagent claimed it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct BeforeClaim {
    status: String,
    owner: Option<String>,
    text: String,
}

/// Free a section a subagent still holds as `doing`. Untouched since the
/// claim, it goes back to how it stood then: a section `main` finished or a
/// person edited keeps that status and owner (Phase 4b final review — it
/// used to become `todo` under the subagent's name). Written to but never
/// finished, or claimed before this memory existed, it goes to `todo`, and
/// keeps the subagent as owner only when there is text it wrote.
fn give_back(s: &mut document::Section, before: Option<BeforeClaim>) {
    match before {
        Some(b) if b.text == s.text() => {
            s.status = Status::parse(&b.status);
            s.owner = b.owner;
        }
        _ => {
            s.status = Status::Todo;
            if s.text().is_empty() {
                s.owner = None;
            }
        }
    }
}

/// A session's document: where the file is and where its memory is.
struct Linked {
    rel: String,
    abs: PathBuf,
    state: PathBuf,
}

/// `<session>.doc.json`, beside the session file.
fn state_path(store_path: &Path) -> PathBuf {
    store_path.with_extension("doc.json")
}

/// Where `rel` points, if it is a file the harness may own: a `.md` file
/// under `<workspace>/.claudinio/docs/`, named with plain components (no
/// `..`, no root), that does not resolve outside that folder through a
/// link. The path comes from the session file, which is workspace data like
/// any other — a crafted or corrupted `document_linked` record must not make
/// the harness rewrite `Cargo.toml` at the next run start (every `[word]`
/// line of an INI or TOML file reads as a placeholder), so anything else
/// leaves the session with no document. A file that does not exist (yet)
/// passes: reading it fails, as before, and a dangling link is refused,
/// since writing through it would create its target.
fn document_path(workspace: &Path, rel: &str) -> Option<PathBuf> {
    use std::path::Component;
    let path = Path::new(rel);
    let plain = path.components().all(|c| matches!(c, Component::Normal(_)));
    let md = path.extension().is_some_and(|e| e == "md");
    if !(plain && path.starts_with(DOCS_DIR) && md) {
        return None;
    }
    let abs = workspace.join(path);
    if std::fs::symlink_metadata(&abs).is_ok() {
        let real = std::fs::canonicalize(&abs).ok()?;
        // One component at a time: inside a Windows verbatim path
        // (`\\?\C:\...`, what canonicalize returns) `/` is no separator.
        let docs = DOCS_DIR
            .split('/')
            .fold(std::fs::canonicalize(workspace).ok()?, |p, c| p.join(c));
        if !real.starts_with(docs) {
            return None;
        }
    }
    Some(abs)
}

fn linked_from(
    workspace: &Path,
    store_path: &Path,
    records: &[crate::agent::persist::SessionRecord],
) -> Option<Linked> {
    linked_rel(
        workspace,
        store_path,
        crate::agent::persist::linked_document(records)?,
    )
}

fn linked_rel(workspace: &Path, store_path: &Path, rel: String) -> Option<Linked> {
    Some(Linked {
        abs: document_path(workspace, &rel)?,
        rel,
        state: state_path(store_path),
    })
}

/// For the app's commands, which have no run's records cache: the link is
/// read straight from the session file (`persist::linked_document_at`).
fn linked_at(workspace: &Path, store_path: &Path) -> Option<Linked> {
    linked_rel(
        workspace,
        store_path,
        crate::agent::persist::linked_document_at(store_path)?,
    )
}

/// Through the run's records cache: the loop asks after every round, and
/// re-parsing the whole session file each time would be the cost of a
/// feature most sessions do not use.
fn linked(ctx: &crate::agent::tools::ToolContext) -> Option<Linked> {
    let store_path = Path::new(ctx.session_store_path.as_deref()?);
    let records =
        crate::agent::persist::load_records_cached(store_path, &ctx.records_cache).ok()?;
    linked_from(
        Path::new(ctx.workspace_root.as_deref()?),
        store_path,
        &records,
    )
}

/// Whether `path` (absolute, or relative to the workspace) is the session's
/// living document.
pub fn is_linked_document(ctx: &crate::agent::tools::ToolContext, path: &str) -> bool {
    let (Some(l), Some(ws)) = (linked(ctx), ctx.workspace_root.as_deref()) else {
        return false;
    };
    let given = Path::new(ws).join(path);
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(&given) == canon(&l.abs)
}

/// Whether the session `ctx` writes to has a document.
pub fn has_document(ctx: &crate::agent::tools::ToolContext) -> bool {
    linked(ctx).is_some()
}

fn load_state(l: &Linked, content: &str) -> DocState {
    std::fs::read_to_string(&l.state)
        .ok()
        .and_then(|t| serde_json::from_str::<DocState>(&t).ok())
        .filter(|s| s.path == l.rel)
        // No memory yet (or of another document): what is there now is
        // what the agents know; nothing has changed under them.
        .unwrap_or_else(|| DocState {
            path: l.rel.clone(),
            known: content.to_string(),
            ..DocState::default()
        })
}

fn save_state(l: &Linked, state: &DocState) {
    if let Ok(text) = serde_json::to_string(state) {
        let _ = std::fs::write(&l.state, text);
    }
}

/// `content` read against what the harness last knew: ids for placeholders a
/// person typed, `human` on sections whose text they changed, and each change
/// queued for the model.
fn sync(content: &str, state: &mut DocState) -> Document {
    let known = document::parse(&state.known);
    let mut cur = document::parse(content);
    cur.reserve_ids_of(&known);
    cur.reserve_from(state.next_id);
    cur.assign_ids();
    if content == state.known {
        return cur;
    }
    for change in document::diff(&known, &cur) {
        match change.kind {
            ChangeKind::Edited => {
                let old_text = known.section(&change.id).map(|s| s.text().to_string());
                if let Some(s) = cur.section_mut(&change.id)
                    && old_text.as_deref() != Some(s.text())
                {
                    s.status = Status::Human;
                    s.owner = None;
                    state.unread.insert(change.id.clone());
                }
            }
            ChangeKind::Removed => {
                state.unread.remove(&change.id);
            }
            ChangeKind::Added => {}
        }
        state.pending.retain(|p| p.id != change.id);
        state.pending.push(change);
    }
    cur
}

/// Read the file under the lock, bring it in line with the harness's memory
/// ([`sync`]), let `f` work on it, and write back whatever changed — the
/// file, and the memory. Persisted even when `f` refuses: what the read
/// found (a person's edit) is true either way.
fn with_doc<T>(
    l: &Linked,
    f: impl FnOnce(&mut Document, &mut DocState) -> Result<T, String>,
) -> Result<T, String> {
    let _guard = DOC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let content = std::fs::read_to_string(&l.abs)
        .map_err(|e| format!("cannot read the document {}: {e}", l.rel))?;
    let mut state = load_state(l, &content);
    let mut doc = sync(&content, &mut state);
    let out = f(&mut doc, &mut state);
    let text = document::render(&doc);
    if text != content {
        std::fs::write(&l.abs, &text)
            .map_err(|e| format!("cannot write the document {}: {e}", l.rel))?;
    }
    state.next_id = state.next_id.max(doc.next_number());
    state.known = text;
    save_state(l, &state);
    out
}

/// A file name for `title`: lowercase letters and digits joined by `-`,
/// at most 60 characters, `document` when nothing is left.
pub fn slug(title: &str) -> String {
    let words: Vec<String> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    let joined: String = words.join("-").chars().take(60).collect();
    let joined = joined.trim_end_matches('-').to_string();
    if joined.is_empty() {
        "document".into()
    } else {
        joined
    }
}

/// `<workspace>/.claudinio/docs`, made one folder at a time, refusing a
/// component that is a link or a file. `doc_create` runs without approval
/// in a workspace that may be a cloned repo: a `.claudinio` that links out
/// would otherwise take the scaffold (and `create_dir_all` the folders)
/// wherever it points — the same escape [`document_path`] refuses on the
/// read and rewrite path.
fn docs_dir(workspace: &Path) -> Result<PathBuf, String> {
    let mut dir = workspace.to_path_buf();
    for component in DOCS_DIR.split('/') {
        dir.push(component);
        match std::fs::symlink_metadata(&dir) {
            // A link's own metadata is never `is_dir`.
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(format!(
                    "{DOCS_DIR} must be a plain folder inside the workspace; {} is a link or a file",
                    dir.display()
                ));
            }
            Err(_) => {
                std::fs::create_dir(&dir).map_err(|e| format!("cannot create {DOCS_DIR}: {e}"))?
            }
        }
    }
    Ok(dir)
}

/// Write a new document for `title` with one placeholder per topic under
/// `<workspace>/.claudinio/docs/`, never over an existing file, and start its
/// memory beside `memory_of` (a session file) when given. Returns the
/// relative path and the document.
///
/// `memory_of` is `None` for a new top-level topic: the session calling
/// `doc_create` keeps its own document, and its `<session>.doc.json` — the
/// unread edits of a person, the pending notes, the id floor — is about that
/// one. Overwriting it with the new document's would make the old session
/// forget an edit no agent has read, and the next `doc_write` there would
/// overwrite the person. The successor that owns the new document starts its
/// memory at its first read instead (`load_state`).
pub fn create(
    workspace: &Path,
    memory_of: Option<&Path>,
    title: &str,
    topics: &[String],
) -> Result<(String, Document), String> {
    if title.trim().is_empty() {
        return Err("doc_create needs a title".into());
    }
    let doc = document::scaffold(title, topics);
    if doc.sections().next().is_none() {
        return Err(
            "doc_create needs at least one section topic: {\"title\", \"sections\": [\"...\"]}"
                .into(),
        );
    }
    let dir = docs_dir(workspace)?;
    let base = slug(title);
    // `symlink_metadata`, not `exists`: `exists` follows a link and calls a
    // dangling one free, and writing through it would create its target.
    let name = (1..)
        .map(|n| match n {
            1 => format!("{base}.md"),
            n => format!("{base}-{n}.md"),
        })
        .find(|name| std::fs::symlink_metadata(dir.join(name)).is_err())
        .unwrap_or_default();
    let text = document::render(&doc);
    // `create_new` refuses any entry already there, a link included, so a
    // link planted between the check above and this write is not followed.
    {
        use std::io::Write as _;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(&name))
            .and_then(|mut f| f.write_all(text.as_bytes()))
            .map_err(|e| format!("cannot write {name}: {e}"))?;
    }
    let rel = format!("{DOCS_DIR}/{name}");
    let Some(store_path) = memory_of else {
        return Ok((rel, doc));
    };
    let _guard = DOC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    save_state(
        &Linked {
            rel: rel.clone(),
            abs: dir.join(&name),
            state: state_path(store_path),
        },
        &DocState {
            path: rel.clone(),
            known: text,
            ..DocState::default()
        },
    );
    Ok((rel, doc))
}

/// Link the document at `rel` to the session in `store`.
pub fn link(
    store: &crate::agent::persist::SessionStore,
    records_cache: &crate::agent::persist::RecordsCache,
    rel: &str,
) {
    store.try_append(&crate::agent::persist::SessionRecord::DocumentLinked {
        path: rel.to_string(),
        ts: crate::agent::persist::now_ms(),
    });
    crate::agent::persist::invalidate_cache(&store.path, records_cache);
}

/// Who writes, and whether `id` is theirs to touch.
fn owner_for(access: &DocAccess, id: Option<&str>) -> Result<String, String> {
    match access {
        DocAccess::Main => Ok("main".into()),
        DocAccess::None => Err("this agent has no access to the session's document".into()),
        DocAccess::Section { id: granted, owner } => match id {
            Some(id) if id != granted => Err(format!(
                "you were given section {granted} only; {id} belongs to another agent"
            )),
            _ => Ok(owner.clone()),
        },
    }
}

/// `doc_write`: claim and write one section, as the context's owner.
pub fn write(
    ctx: &crate::agent::tools::ToolContext,
    id: &str,
    body: Option<&str>,
    status: Status,
) -> Result<String, String> {
    let owner = owner_for(&ctx.doc_access, Some(id))?;
    let l = linked(ctx).ok_or(NO_DOCUMENT)?;
    with_doc(&l, |doc, state| {
        if state.unread.remove(id) {
            let s = doc.section(id).map_or("", |s| s.text());
            return Err(format!(
                "{id} was edited by the user since you last read it, and their edit wins: \
                 nothing was written. Its text now:\n---\n{s}\n---\nMerge what still applies \
                 into it and call doc_write again."
            ));
        }
        doc.apply_write(id, body, &owner, status).map_err(|e| match e {
            document::WriteError::NoSuchSection { ids } => {
                format!("no section has id '{id}'; the ids are: {}", ids.join(", "))
            }
            document::WriteError::Claimed { owner } => format!(
                "{id} is being written by {owner} (status doing); pick another section"
            ),
            document::WriteError::EmptyBody => format!(
                "the body is empty: a 'done' section needs its text ('doing' claims {id} without it)"
            ),
        })?;
        Ok(format!(
            "Wrote {id} ({}).\n{}",
            status.as_str(),
            doc.outline()
        ))
    })
}

/// The owner a subagent writes as: `sub:<name>`, spaces as dashes so the
/// harness's comment stays one token per field, and only letters, digits,
/// `_` and `-` kept: the name is the model's, and a `>` in it (`a-->`) would
/// end the comment early in any Markdown renderer, showing the rest of it in
/// the document the person reads. `sub:agent` when nothing is left. The
/// panel matches owners to subagents with the same rule
/// (`subagentForOwner` in `lib/documentStore.ts`).
pub fn subagent_owner(name: &str) -> String {
    let words: Vec<String> = name
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return "sub:agent".into();
    }
    format!("sub:{}", words.join("-"))
}

/// Claim section `id` for a subagent before it starts (`doing`, under
/// `owner`), so two agents never hold it and the panel shows who has it.
/// Returns the section's topic.
pub fn claim(
    ctx: &crate::agent::tools::ToolContext,
    id: &str,
    owner: &str,
) -> Result<String, String> {
    let l = linked(ctx).ok_or(NO_DOCUMENT)?;
    with_doc(&l, |doc, state| {
        let before = doc.section(id).map(|s| BeforeClaim {
            status: s.status.as_str().into(),
            owner: s.owner.clone(),
            text: s.text().into(),
        });
        doc.apply_write(id, None, owner, Status::Doing)
            .map_err(|e| match e {
                document::WriteError::Claimed { owner } => {
                    format!("{id} is being written by {owner}")
                }
                _ => format!(
                    "the document has no section '{id}'; its ids are: {}",
                    doc.ids().join(", ")
                ),
            })?;
        if let Some(before) = before {
            state.claimed.insert(id.to_string(), before);
        }
        Ok(doc.section(id).map(|s| s.topic.clone()).unwrap_or_default())
    })
}

/// After the subagent ends: a section it still holds as `doing` (it never
/// finished it) is freed ([`give_back`]), so nobody waits on an agent that
/// is gone.
pub fn release(ctx: &crate::agent::tools::ToolContext, id: &str, owner: &str) {
    let Some(l) = linked(ctx) else {
        return;
    };
    let _ = with_doc(&l, |doc, state| {
        let before = state.claimed.remove(id);
        if let Some(s) = doc.section_mut(id)
            && s.status == Status::Doing
            && s.owner.as_deref() == Some(owner)
        {
            give_back(s, before);
        }
        Ok(())
    });
}

/// `doc_read`: one section's text, or the outline when `id` is `None`.
pub fn read(ctx: &crate::agent::tools::ToolContext, id: Option<&str>) -> Result<String, String> {
    owner_for(&ctx.doc_access, id)?;
    let l = linked(ctx).ok_or(NO_DOCUMENT)?;
    with_doc(&l, |doc, state| {
        let Some(id) = id else {
            return Ok(doc.outline());
        };
        let s = doc.section(id).ok_or_else(|| {
            format!(
                "no section has id '{id}'; the ids are: {}",
                doc.ids().join(", ")
            )
        })?;
        state.unread.remove(id);
        let text = if s.text().is_empty() {
            "(not written yet)"
        } else {
            s.text()
        };
        Ok(format!(
            "{id} \u{b7} {} \u{b7} {} \u{b7} {}\n\n{text}",
            s.status.as_str(),
            s.owner.as_deref().unwrap_or("-"),
            s.topic
        ))
    })
}

/// `doc_add`: a new placeholder.
pub fn add(
    ctx: &crate::agent::tools::ToolContext,
    topic: &str,
    after: Option<&str>,
) -> Result<String, String> {
    if !matches!(ctx.doc_access, DocAccess::Main) {
        return Err("only the main session adds sections".into());
    }
    let l = linked(ctx).ok_or(NO_DOCUMENT)?;
    with_doc(&l, |doc, _| {
        let id = doc.add_placeholder(topic, after)?;
        Ok(format!("Added {id}.\n{}", doc.outline()))
    })
}

/// `pending` as one note, emptied.
fn drain_note(state: &mut DocState) -> Option<String> {
    if state.pending.is_empty() {
        return None;
    }
    let parts: Vec<String> = state
        .pending
        .drain(..)
        .map(|c| format!("{} ({}) {}", c.id, c.topic, c.kind.as_str()))
        .collect();
    Some(format!(
        "{DOC_CHANGED_OPEN}\nThe user changed the document: {}. Their edits win: doc_read an \
         edited section before writing it.\n</doc-changed>",
        parts.join("; ")
    ))
}

/// The changes a person made that the model has not been told about, as one
/// `<doc-changed>` note, or `None`. Each is told once.
pub fn take_change_note(ctx: &crate::agent::tools::ToolContext) -> Option<String> {
    let l = linked(ctx)?;
    with_doc(&l, |_, state| Ok(drain_note(state)))
        .ok()
        .flatten()
}

/// How a `<doc-changed>` note opens (`task_recital` looks past one when it
/// matches a run turn's recital).
pub const DOC_CHANGED_OPEN: &str = "<doc-changed>";
/// How the `<document>` outline block opens (`task_recital` tells a recited
/// outline by it).
pub const DOCUMENT_OPEN: &str = "<document path=";

fn document_block(rel: &str, doc: &Document) -> String {
    format!("{DOCUMENT_OPEN}\"{rel}\">\n{}\n</document>", doc.outline())
}

/// The `<document>` outline block as the file stands now, for a compaction
/// to record and restate after its summary (lean-harness Phase 4f: Phase 4b
/// left the outline out of the compaction recital). A read like any other —
/// a person's edit found here is recorded — but it frees no claim (a
/// compaction runs mid-run, while subagents may hold sections) and tells no
/// note (that belongs to the round's tool results).
pub fn outline_block(ctx: &crate::agent::tools::ToolContext) -> Option<String> {
    let l = linked(ctx)?;
    with_doc(&l, |doc, _| Ok(document_block(&l.rel, doc))).ok()
}

/// What a run's user turn ends with when the session has a document: the
/// `<document>` outline block, then a `<doc-changed>` note when a person
/// edited it since the model last heard.
pub fn run_start_blocks(ctx: &crate::agent::tools::ToolContext) -> Vec<String> {
    let Some(l) = linked(ctx) else {
        return Vec::new();
    };
    with_doc(&l, |doc, state| {
        // No subagent of this session outlives the run that spawned it, so
        // a claim one still holds here is left over (the app was closed
        // mid-call) and would refuse every other writer.
        for s in doc.sections_mut() {
            if s.status == Status::Doing
                && s.owner.as_deref().is_some_and(|o| o.starts_with("sub:"))
            {
                give_back(s, state.claimed.remove(&s.id));
            }
        }
        state.claimed.clear();
        let mut blocks = vec![document_block(&l.rel, doc)];
        blocks.extend(drain_note(state));
        Ok(blocks)
    })
    .unwrap_or_default()
}

fn view_of(rel: &str, doc: &Document) -> DocView {
    let mut sections = Vec::new();
    let mut line = 1;
    for b in &doc.blocks {
        let text = match b {
            document::Block::Prose(p) => p.clone(),
            document::Block::Section(s) => {
                sections.push(SectionView {
                    id: s.id.clone(),
                    topic: s.topic.clone(),
                    status: s.status.as_str().into(),
                    owner: s.owner.clone(),
                    line,
                });
                document::render_section(s)
            }
        };
        line += text.matches('\n').count();
    }
    DocView {
        path: rel.to_string(),
        title: doc.title.clone(),
        sections,
    }
}

/// The document as it stands, for the panel — read only: nothing is written
/// and nothing the model is owed a note about is consumed. A section a
/// person changed since the harness last looked shows as `human` already.
pub fn view(workspace: &Path, store_path: &Path) -> Option<DocView> {
    let l = linked_at(workspace, store_path)?;
    let _guard = DOC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let content = std::fs::read_to_string(&l.abs).ok()?;
    let mut state = load_state(&l, &content);
    let doc = sync(&content, &mut state);
    Some(view_of(&l.rel, &doc))
}

/// The session file of `session_id` in `workspace`, for the app's
/// document commands. `None` unless the id is a plain name (letters, digits,
/// `-`, `_`) and the file exists: the id arrives over IPC, and the webview
/// is not trusted to name a path.
pub fn session_file(workspace: &Path, session_id: &str) -> Option<PathBuf> {
    let plain = !session_id.is_empty()
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !plain {
        return None;
    }
    let path = workspace
        .join(".claudinio")
        .join("sessions")
        .join(format!("{session_id}.jsonl"));
    path.is_file().then_some(path)
}

/// The current view for a context, after a write (`DocumentUpdated`).
pub fn view_for(ctx: &crate::agent::tools::ToolContext) -> Option<DocView> {
    let ws = ctx.workspace_root.as_deref()?;
    let store = ctx.session_store_path.as_deref()?;
    view(Path::new(ws), Path::new(store))
}

/// A save from the app: `content` is what the person wrote, `base` the text
/// the editor loaded. Merged with what agents wrote meanwhile
/// (`document::merge_human_save`); the sections the person changed are
/// marked `human` and the model is told.
pub fn save_human(
    workspace: &Path,
    store_path: &Path,
    base: &str,
    content: &str,
) -> Result<DocView, String> {
    let l = linked_at(workspace, store_path).ok_or(NO_DOCUMENT)?;
    with_doc(&l, |doc, state| {
        let base = document::parse(base);
        let mut mine = document::parse(content);
        mine.reserve_ids_of(doc);
        mine.reserve_from(state.next_id);
        mine.assign_ids();
        let merged = document::merge_human_save(&base, &mine, doc);
        for change in document::diff(doc, &merged) {
            if change.kind == ChangeKind::Edited
                && merged.section(&change.id).map(|s| s.status) == Some(Status::Human)
            {
                state.unread.insert(change.id.clone());
            }
            if change.kind == ChangeKind::Removed {
                state.unread.remove(&change.id);
            }
            state.pending.retain(|p| p.id != change.id);
            state.pending.push(change);
        }
        *doc = merged;
        Ok(view_of(&l.rel, doc))
    })
}

// ── The run loop's side ─────────────────────────────────────────────────

pub(crate) fn send_update(
    ctx: &crate::agent::tools::ToolContext,
    event_tx: &tauri::ipc::Channel<crate::agent::events::AgentEvent>,
    session_id: &str,
) {
    if let Some(document) = view_for(ctx) {
        let _ = event_tx.send(crate::agent::events::AgentEvent::DocumentUpdated {
            session_id: session_id.to_string(),
            document,
        });
    }
}

/// After a `doc_*` tool wrote the document: tell the panel.
pub fn after_tool(
    tool_name: &str,
    ctx: &crate::agent::tools::ToolContext,
    event_tx: &tauri::ipc::Channel<crate::agent::events::AgentEvent>,
    session_id: &str,
) {
    if matches!(tool_name, "doc_write" | "doc_add") {
        send_update(ctx, event_tx, session_id);
    }
}

/// A run of a session with a document carries the tools that work on it
/// from its first request, as if `doc_create` had loaded them — which is
/// what happened in this session or the one it continues. A session that
/// gets its document another way (a successor the app linked without the
/// seed) loads them here, once, on the record.
pub fn seed_tools(
    ctx: &crate::agent::tools::ToolContext,
    store: &crate::agent::persist::SessionStore,
    loaded: &mut Vec<String>,
) {
    let from = loaded.len();
    join_tools(ctx, loaded);
    crate::agent::deferred::record(store, ctx, &loaded[from..]);
}

/// After a `doc_create`: the document tools join `loaded` when the session
/// now has a document, in the same round (one cache break, recorded with the
/// round's other loads). A `doc_create` that created nothing — no topics, no
/// title, no workspace — loads nothing: three definitions that could only
/// answer NO_DOCUMENT would ride on every later request, and on resume
/// (Phase 4b final review; they used to join before the call ran).
pub fn join_tools(ctx: &crate::agent::tools::ToolContext, loaded: &mut Vec<String>) {
    if !has_document(ctx) {
        return;
    }
    for name in crate::agent::deferred::DOCUMENT_TOOLS {
        if !loaded.iter().any(|l| l == name) {
            loaded.push(name.to_string());
        }
    }
}

/// Before the round's tool results become the next user message: a change
/// a person made to the document joins that message as a `<doc-changed>`
/// note, so it is persisted with the results and a reload sends the same
/// bytes, and the panel is told.
pub fn note_round(
    results: &mut Vec<crate::agent::provider::ContentBlock>,
    ctx: &crate::agent::tools::ToolContext,
    event_tx: &tauri::ipc::Channel<crate::agent::events::AgentEvent>,
    session_id: &str,
) {
    if let Some(note) = take_change_note(ctx) {
        results.push(crate::agent::provider::ContentBlock::text(note));
        send_update(ctx, event_tx, session_id);
    }
}

/// How the successor's first message for a new top-level topic begins.
pub(crate) const NEW_DOCUMENT_KICKOFF: &str = "[system] A new top-level topic:";

/// The successor's first message for a new top-level topic. The outline
/// follows it as the run's `<document>` block.
fn new_document_kickoff(user_message: &str, rel: &str) -> String {
    format!(
        "{NEW_DOCUMENT_KICKOFF} this fresh session owns the document {rel} (the \
         previous session keeps its own). Its outline follows as a `<document>` block; fill \
         its sections. The user's request was:\n\n{user_message}"
    )
}

/// Whether the run whose message is `user_message` is part of a user turn
/// a new-document handoff already spent: the run that handoff started, or
/// the first run of a session a context handoff opened from such a run —
/// a context handoff continues its predecessor's run, so it is still the
/// same user turn (Phase 4b final review: the kickoff check alone let a
/// kickoff run that filled its window open a second document in its
/// successor). Followed back through `context_handoff` links only, and
/// only while each session is in its first run; any run the user typed
/// ends the walk. Bounded, in case a session file links in a cycle.
fn new_document_turn(
    workspace: Option<&Path>,
    user_message: &str,
    records: &[crate::agent::persist::SessionRecord],
) -> bool {
    use crate::agent::persist::SessionRecord;
    let mut message = user_message.to_string();
    let mut records = records.to_vec();
    for _ in 0..32 {
        if message.starts_with(NEW_DOCUMENT_KICKOFF) {
            return true;
        }
        let runs = records
            .iter()
            .filter(|r| matches!(r, SessionRecord::User { .. }))
            .count();
        let continued = crate::agent::persist::linked_from(&records)
            .filter(|l| runs <= 1 && l.reason == "context_handoff");
        let Some(prev) = continued
            .zip(workspace)
            .and_then(|(l, ws)| session_file(ws, &l.prev_session_id))
            .and_then(|p| crate::agent::persist::load_records(&p).ok())
        else {
            return false;
        };
        // The predecessor's last run is the one that handed off.
        let Some(last) = prev.iter().rev().find_map(|r| match r {
            SessionRecord::User { text, .. } => Some(text.clone()),
            _ => None,
        }) else {
            return false;
        };
        message = last;
        records = prev;
    }
    false
}

/// `doc_create`, answered by the main loop. A session without a document
/// gets the scaffold linked to it. A session that has one is starting a new
/// top-level topic: the new document is written and the run hands off to a
/// linked session that owns it (`HandoffReason::NewDocument`), this session
/// keeping its own.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_call(
    tool_use_id: &str,
    input: &serde_json::Value,
    user_message: &str,
    mode_ctl: &crate::agent::session::ModeCtl,
    store: &crate::agent::persist::SessionStore,
    ctx: &crate::agent::tools::ToolContext,
    event_tx: &tauri::ipc::Channel<crate::agent::events::AgentEvent>,
    session_id: &str,
    pending_handoff: &mut Option<crate::agent::handoff::HandoffSpec>,
) -> crate::agent::provider::ContentBlock {
    use crate::agent::events::AgentEvent;
    let _ = event_tx.send(AgentEvent::ToolCall {
        session_id: session_id.to_string(),
        tool_id: tool_use_id.to_string(),
        tool_name: "doc_create".into(),
        args: input.clone(),
        permission: "auto".into(),
        edit_proposal: None,
    });
    let title = input.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let topics: Vec<String> = input
        .get("sections")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let existing = linked(ctx);
    // The run a new document's handoff starts exists to fill that document.
    // Its first message repeats the user's request, and the app follows
    // every handoff, so a second `doc_create` here would open session after
    // session with nothing to stop the chain; a new topic from this session
    // waits for a turn the user types.
    let kickoff_run = existing.is_some()
        && new_document_turn(
            ctx.workspace_root.as_deref().map(Path::new),
            user_message,
            &crate::agent::persist::load_records_cached(&store.path, &ctx.records_cache)
                .unwrap_or_default(),
        );
    let result = match (ctx.workspace_root.as_deref(), &existing) {
        _ if pending_handoff.is_some() => Err(crate::agent::handoff::HANDOFF_QUEUED.to_string()),
        (Some(_), Some(own)) if kickoff_run => Err(format!(
            "this session was just opened for {}; fill its sections (doc_add adds one) \
             instead of creating another document. A further new topic waits for the \
             user's next message.",
            own.rel
        )),
        (None, _) => Err("doc_create needs an open workspace".to_string()),
        (Some(ws), _) => create(
            Path::new(ws),
            existing.is_none().then_some(store.path.as_path()),
            title,
            &topics,
        ),
    }
    .map(|(rel, doc)| match existing {
        None => {
            link(store, &ctx.records_cache, &rel);
            format!(
                "Created {rel}, linked to this session:\n{}\nFill sections with doc_write \
                 {{id, body}}; give independent sections to explore subagents (spawn_agents \
                 with \"section\": \"<id>\") — each writes only its own. The user may edit the \
                 file at any time, and their edits win.",
                doc.outline()
            )
        }
        Some(old) => {
            let (mode, origin) = mode_ctl.get();
            // The golden loop's state carries over, as on a context handoff,
            // so its caps never reset; the golden tasks go along too
            // (`transition::tasks_for_successor`).
            let records =
                crate::agent::persist::load_records_cached(&store.path, &ctx.records_cache)
                    .unwrap_or_default();
            let (linked_cycle, golden_stalls, golden_last_pending) =
                crate::agent::persist::linked_golden_state(&records);
            *pending_handoff = Some(crate::agent::handoff::HandoffSpec {
                reason: crate::agent::handoff::HandoffReason::NewDocument,
                next_mode: mode,
                next_origin: origin,
                first_message: new_document_kickoff(user_message, &rel),
                golden_cycle: crate::agent::persist::golden_cycle_count(&records).max(linked_cycle),
                golden_stalls,
                golden_last_pending,
                document: Some(rel.clone()),
            });
            format!(
                "This session's document is {}. A new topic is a new session: {rel} was \
                 created and a linked session opens with it. End your turn now.",
                old.rel
            )
        }
    });
    let (text, error) = match result {
        Ok(text) => (text, None),
        Err(e) => (format!("Error: {e}"), Some(e)),
    };
    let _ = event_tx.send(AgentEvent::ToolResult {
        tool_id: tool_use_id.to_string(),
        tool_name: "doc_create".into(),
        output: if error.is_some() {
            String::new()
        } else {
            text.clone()
        },
        error,
    });
    if pending_handoff.is_none() {
        send_update(ctx, event_tx, session_id);
    }
    crate::agent::provider::ContentBlock::tool_result(tool_use_id, text)
}

/// Link a successor session to its document: the new one for a new
/// top-level topic (`new_document`), else the predecessor's, whose memory is
/// carried along so an unread edit stays unread. The document tools are
/// seeded as loaded, so the successor's first request carries them.
pub fn carry_into(
    old_store: &Path,
    old_records: &[crate::agent::persist::SessionRecord],
    new_store: &crate::agent::persist::SessionStore,
    new_document: Option<&str>,
) {
    let rel = match new_document {
        Some(rel) => rel.to_string(),
        None => {
            let Some(rel) = crate::agent::persist::linked_document(old_records) else {
                return;
            };
            let _guard = DOC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let _ = std::fs::copy(state_path(old_store), state_path(&new_store.path));
            rel
        }
    };
    let now = crate::agent::persist::now_ms();
    new_store
        .try_append(&crate::agent::persist::SessionRecord::DocumentLinked { path: rel, ts: now });
    new_store.try_append(&crate::agent::persist::SessionRecord::ToolsLoaded {
        names: crate::agent::deferred::DOCUMENT_TOOLS
            .iter()
            .map(|t| t.to_string())
            .collect(),
        ts: now,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::persist::{SessionRecord, SessionStore, now_ms};
    use crate::agent::tools::ToolContext;

    struct Ws {
        root: PathBuf,
        ctx: ToolContext,
    }

    impl Drop for Ws {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    impl Ws {
        fn new(tag: &str) -> Ws {
            let root = std::env::temp_dir().join(format!(
                "claudinio-doc-{tag}-{}-{}",
                std::process::id(),
                now_ms()
            ));
            std::fs::create_dir_all(&root).unwrap();
            let store = SessionStore::create("s", Some(root.to_str().unwrap())).unwrap();
            let ctx = ToolContext {
                workspace_root: Some(root.to_string_lossy().into()),
                session_store_path: Some(store.path.to_string_lossy().into()),
                ..crate::agent::tools::tests_support::ctx()
            };
            Ws { root, ctx }
        }

        fn store(&self) -> PathBuf {
            PathBuf::from(self.ctx.session_store_path.as_deref().unwrap())
        }

        /// A document with these topics, linked to the session.
        fn with_doc(tag: &str, topics: &[&str]) -> (Ws, PathBuf) {
            let ws = Ws::new(tag);
            let topics: Vec<String> = topics.iter().map(|t| t.to_string()).collect();
            let (rel, _) = create(&ws.root, Some(&ws.store()), "Pudim de leite", &topics).unwrap();
            SessionStore { path: ws.store() }
                .append(&SessionRecord::DocumentLinked {
                    path: rel.clone(),
                    ts: now_ms(),
                })
                .unwrap();
            let abs = ws.root.join(rel);
            (ws, abs)
        }

        fn sub(&self, id: &str, owner: &str) -> ToolContext {
            ToolContext {
                doc_access: DocAccess::Section {
                    id: id.into(),
                    owner: owner.into(),
                },
                ..self.ctx.clone()
            }
        }
    }

    fn edit(path: &Path, from: &str, to: &str) {
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains(from), "{from:?} not in {text}");
        std::fs::write(path, text.replace(from, to)).unwrap();
    }

    /// The owner sits inside the harness's HTML comment: a model-chosen name
    /// must not be able to end that comment early in a Markdown renderer.
    #[test]
    fn a_subagent_owner_is_one_token_that_cannot_close_the_comment() {
        assert_eq!(subagent_owner("pesquisa origem"), "sub:pesquisa-origem");
        assert_eq!(subagent_owner("a-->"), "sub:a--");
        assert_eq!(subagent_owner("x=1 <b>"), "sub:x1-b");
        assert_eq!(subagent_owner("análise_2"), "sub:análise_2");
        assert_eq!(subagent_owner("-->"), "sub:--");
        assert_eq!(subagent_owner("<>"), "sub:agent");
        for name in ["a-->", "a--!>", "x=1 <b>"] {
            let owner = subagent_owner(name);
            assert!(!owner.contains('>') && !owner.contains('='), "{owner}");
        }
    }

    #[test]
    fn a_slug_is_lowercase_words_joined_by_dashes() {
        assert_eq!(slug("Pudim de Leite!"), "pudim-de-leite");
        assert_eq!(slug("Modo de preparo clássico"), "modo-de-preparo-clássico");
        assert_eq!(slug("  ?? "), "document");
        assert_eq!(slug(&"a".repeat(100)).chars().count(), 60);
    }

    #[test]
    fn create_writes_the_scaffold_under_claudinio_docs_and_never_over_a_file() {
        let ws = Ws::new("create");
        let topics = vec!["origem".to_string(), "ingredientes".to_string()];
        let (rel, doc) = create(&ws.root, Some(&ws.store()), "Pudim de leite", &topics).unwrap();
        assert_eq!(rel, ".claudinio/docs/pudim-de-leite.md");
        assert_eq!(
            std::fs::read_to_string(ws.root.join(&rel)).unwrap(),
            document::render(&doc)
        );
        let (again, _) = create(&ws.root, Some(&ws.store()), "Pudim de leite", &topics).unwrap();
        assert_eq!(again, ".claudinio/docs/pudim-de-leite-2.md");
        assert!(create(&ws.root, Some(&ws.store()), " ", &topics).is_err());
        assert!(create(&ws.root, Some(&ws.store()), "T", &[]).is_err());
    }

    /// `doc_create` runs without approval and its file name comes from the
    /// model's title, so a link a cloned repo planted in its path must not
    /// make the harness write outside the workspace: a dangling link where
    /// the scaffold would go is a taken name, never followed (following it
    /// would create its target), and a `.claudinio` or `.claudinio/docs`
    /// that is a link is refused before anything is written.
    #[cfg(unix)]
    #[test]
    fn create_never_writes_through_a_planted_symlink() {
        let outside = std::env::temp_dir().join(format!(
            "claudinio-doc-planted-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&outside).unwrap();
        let topics = vec!["origem".to_string()];

        // A dangling link at the slug's name.
        let ws = Ws::new("planted-file");
        std::fs::create_dir_all(ws.root.join(DOCS_DIR)).unwrap();
        let target = outside.join("created.md");
        std::os::unix::fs::symlink(&target, ws.root.join(DOCS_DIR).join("pudim.md")).unwrap();
        let made = create(&ws.root, Some(&ws.store()), "Pudim", &topics);
        assert!(!target.exists(), "the link's target was created");
        if let Ok((rel, _)) = made {
            assert_eq!(rel, ".claudinio/docs/pudim-2.md");
        }

        // The docs folder itself, and `.claudinio`, as links out (a bare
        // workspace with no session folder, so `.claudinio` is free).
        for linked in [DOCS_DIR, ".claudinio"] {
            let root = outside.join(format!("ws-{}", linked.replace('/', "-")));
            let away = outside.join(format!("away-{}", linked.replace('/', "-")));
            std::fs::create_dir_all(&root).unwrap();
            std::fs::create_dir_all(&away).unwrap();
            if linked == DOCS_DIR {
                std::fs::create_dir_all(root.join(".claudinio")).unwrap();
            }
            std::os::unix::fs::symlink(&away, root.join(linked)).unwrap();
            assert!(create(&root, None, "Pudim", &topics).is_err(), "{linked}");
            let written: Vec<_> = walk(&away);
            assert!(written.is_empty(), "{linked}: wrote {written:?}");
        }
        std::fs::remove_dir_all(&outside).ok();
    }

    #[cfg(unix)]
    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
        out
    }

    /// The session file is workspace data, and the harness rewrites the
    /// linked document at every run start without asking: a
    /// `document_linked` record naming anything but a `.md` file under
    /// `.claudinio/docs/` (a crafted or corrupted session) must leave the
    /// session with no document, and the file it names untouched.
    #[test]
    fn a_document_link_outside_claudinio_docs_is_ignored_and_never_written() {
        let outside = std::env::temp_dir().join(format!(
            "claudinio-doc-outside-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&outside).unwrap();
        let toml = "[package]\nname = \"x\"\n\n[dependencies]\n";
        let outside_md = outside.join("notes.md");
        let outside_name = outside.file_name().unwrap().to_string_lossy().to_string();
        let mut hostile: Vec<(String, &str)> = vec![
            ("Cargo.toml".into(), "Cargo.toml"),
            (".claudinio/docs/../../Cargo.toml".into(), "Cargo.toml"),
            (".claudinio/docs/../x.md".into(), ".claudinio/x.md"),
            (
                ".claudinio/docs/notes.txt".into(),
                ".claudinio/docs/notes.txt",
            ),
            (format!("../{outside_name}/notes.md"), ""),
            (outside_md.to_string_lossy().into(), ""),
        ];
        if cfg!(unix) {
            hostile.push((".claudinio/docs/link.md".into(), "Cargo.toml"));
        }
        for (rel, target) in hostile {
            let ws = Ws::new("hostile");
            let file = if target.is_empty() {
                outside_md.clone()
            } else {
                ws.root.join(target)
            };
            std::fs::create_dir_all(ws.root.join(DOCS_DIR)).unwrap();
            std::fs::write(&file, toml).unwrap();
            #[cfg(unix)]
            if rel.ends_with("link.md") {
                std::os::unix::fs::symlink(&file, ws.root.join(&rel)).unwrap();
            }
            SessionStore { path: ws.store() }
                .append(&SessionRecord::DocumentLinked {
                    path: rel.clone(),
                    ts: now_ms(),
                })
                .unwrap();
            assert!(!has_document(&ws.ctx), "{rel}");
            assert!(run_start_blocks(&ws.ctx).is_empty(), "{rel}");
            assert_eq!(
                write(&ws.ctx, "s1", Some("x"), Status::Done),
                Err(NO_DOCUMENT.to_string()),
                "{rel}"
            );
            assert_eq!(view(&ws.root, &ws.store()), None, "{rel}");
            assert!(
                save_human(&ws.root, &ws.store(), toml, "[a]\n").is_err(),
                "{rel}"
            );
            assert_eq!(std::fs::read_to_string(&file).unwrap(), toml, "{rel}");
        }
        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn a_session_without_a_document_is_told_to_create_one() {
        let ws = Ws::new("nodoc");
        assert_eq!(read(&ws.ctx, None), Err(NO_DOCUMENT.to_string()));
        assert!(run_start_blocks(&ws.ctx).is_empty());
        assert_eq!(take_change_note(&ws.ctx), None);
        assert_eq!(view(&ws.root, &ws.store()), None);
    }

    #[test]
    fn a_write_fills_the_section_and_answers_with_the_outline() {
        let (ws, path) = Ws::with_doc("write", &["origem", "ingredientes"]);
        let out = write(&ws.ctx, "s1", Some("Veio de Portugal."), Status::Done).unwrap();
        assert!(
            out.contains("s1 \u{b7} done \u{b7} main \u{b7} origem"),
            "{out}"
        );
        assert!(out.contains("s2 \u{b7} todo"), "{out}");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(
            "## origem\n<!-- claudinio: id=s1 status=done by=main -->\nVeio de Portugal.\n"
        ));
        // The agent's own write is not a change to tell it about.
        assert_eq!(take_change_note(&ws.ctx), None);
    }

    #[test]
    fn a_section_a_person_edited_is_not_overwritten_until_the_agent_has_seen_it() {
        let (ws, path) = Ws::with_doc("human", &["origem", "ingredientes"]);
        write(&ws.ctx, "s1", Some("Veio de Portugal."), Status::Done).unwrap();
        edit(
            &path,
            "Veio de Portugal.",
            "Veio de Portugal, no século XVIII.",
        );

        let refused = write(&ws.ctx, "s1", Some("Outra coisa."), Status::Done).unwrap_err();
        assert!(refused.contains("edited by the user"), "{refused}");
        assert!(
            refused.contains("Veio de Portugal, no século XVIII."),
            "the refusal carries the current text: {refused}"
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("no século XVIII."), "nothing was written");
        assert!(text.contains("id=s1 status=human"), "{text}");
        // Another section is not held up by it.
        write(&ws.ctx, "s2", Some("leite, ovos"), Status::Done).unwrap();
        // Having been shown the text, the agent may now merge and write.
        write(
            &ws.ctx,
            "s1",
            Some("Veio de Portugal, no século XVIII, com as freiras."),
            Status::Done,
        )
        .unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("com as freiras")
        );
    }

    #[test]
    fn reading_the_section_counts_as_seeing_the_persons_edit() {
        let (ws, path) = Ws::with_doc("read", &["origem"]);
        write(&ws.ctx, "s1", Some("v1"), Status::Done).unwrap();
        edit(&path, "v1", "v2 by hand");
        let body = read(&ws.ctx, Some("s1")).unwrap();
        assert!(body.contains("v2 by hand"), "{body}");
        assert!(body.contains("origem"), "{body}");
        write(&ws.ctx, "s1", Some("v3"), Status::Done).unwrap();
        assert!(
            read(&ws.ctx, None)
                .unwrap()
                .starts_with("# Pudim de leite\ns1")
        );
    }

    #[test]
    fn the_model_is_told_once_which_sections_changed_and_never_their_text() {
        let (ws, path) = Ws::with_doc("note", &["origem", "ingredientes", "servir"]);
        write(&ws.ctx, "s1", Some("segredo do corpo"), Status::Done).unwrap();
        edit(&path, "segredo do corpo", "texto novo da pessoa");
        edit(&path, "[servir]", "[caramelo]\n[servir]");
        let note = take_change_note(&ws.ctx).expect("a note");
        assert!(note.starts_with("<doc-changed>") && note.ends_with("</doc-changed>"));
        assert!(note.contains("s1 (origem) edited"), "{note}");
        assert!(note.contains("s4 (caramelo) added"), "{note}");
        assert!(!note.contains("texto novo"), "never a body: {note}");
        assert_eq!(take_change_note(&ws.ctx), None, "told once");
        // The placeholder the person typed got its id in the file.
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("[caramelo] <!-- claudinio: id=s4 status=todo -->")
        );
    }

    #[test]
    fn a_removed_section_is_reported_and_its_id_is_not_reused() {
        let (ws, path) = Ws::with_doc("removed", &["a", "b"]);
        edit(&path, "[b] <!-- claudinio: id=s2 status=todo -->\n", "");
        let note = take_change_note(&ws.ctx).unwrap();
        assert!(note.contains("s2 (b) removed"), "{note}");
        let out = add(&ws.ctx, "c", None).unwrap();
        assert!(out.contains("s3 \u{b7} todo \u{b7} - \u{b7} c"), "{out}");
    }

    #[test]
    fn a_subagent_with_a_section_writes_and_reads_only_that_section() {
        let (ws, _) = Ws::with_doc("grant", &["origem", "ingredientes"]);
        let sub = ws.sub("s2", "sub:ingredientes");
        write(&sub, "s2", Some("leite condensado"), Status::Done).unwrap();
        assert!(
            read(&ws.ctx, None)
                .unwrap()
                .contains("s2 \u{b7} done \u{b7} sub:ingredientes")
        );
        assert!(
            write(&sub, "s1", Some("x"), Status::Done)
                .unwrap_err()
                .contains("s2")
        );
        assert!(read(&sub, Some("s1")).is_err());
        assert!(read(&sub, Some("s2")).is_ok());
        assert!(add(&sub, "extra", None).is_err());
        let none = ToolContext {
            doc_access: DocAccess::None,
            ..ws.ctx.clone()
        };
        assert!(write(&none, "s1", Some("x"), Status::Done).is_err());
        assert!(read(&none, None).is_err());
    }

    #[test]
    fn two_agents_cannot_hold_the_same_section() {
        let (ws, _) = Ws::with_doc("claim", &["origem"]);
        let one = ws.sub("s1", "sub:one");
        let two = ws.sub("s1", "sub:two");
        write(&one, "s1", None, Status::Doing).unwrap();
        let refused = write(&two, "s1", Some("mine"), Status::Done).unwrap_err();
        assert!(refused.contains("sub:one"), "{refused}");
        assert!(write(&ws.ctx, "s1", Some("main's"), Status::Done).is_err());
    }

    #[test]
    fn a_run_starts_with_the_outline_and_any_note_it_is_owed() {
        let (ws, path) = Ws::with_doc("start", &["origem"]);
        let blocks = run_start_blocks(&ws.ctx);
        assert_eq!(
            blocks,
            vec![
                "<document path=\".claudinio/docs/pudim-de-leite.md\">\n# Pudim de leite\n\
                 s1 \u{b7} todo \u{b7} - \u{b7} origem\n</document>"
                    .to_string()
            ]
        );
        write(&ws.ctx, "s1", Some("v1"), Status::Done).unwrap();
        edit(&path, "v1", "v2");
        let blocks = run_start_blocks(&ws.ctx);
        assert_eq!(blocks.len(), 2);
        assert!(
            blocks[0].contains("s1 \u{b7} human \u{b7} - \u{b7} origem"),
            "{blocks:?}"
        );
        assert!(blocks[1].starts_with("<doc-changed>"));
        assert_eq!(take_change_note(&ws.ctx), None, "the run start told it");
    }

    #[test]
    fn the_panels_view_reads_without_writing_or_consuming() {
        let (ws, path) = Ws::with_doc("view", &["origem", "servir"]);
        write(&ws.ctx, "s1", Some("v1"), Status::Done).unwrap();
        edit(&path, "v1", "v2");
        let before = std::fs::read_to_string(&path).unwrap();
        let v = view(&ws.root, &ws.store()).unwrap();
        assert_eq!(v.path, ".claudinio/docs/pudim-de-leite.md");
        assert_eq!(v.title, "Pudim de leite");
        assert_eq!(v.sections[0].status, "human");
        assert_eq!(v.sections[0].owner, None);
        assert_eq!(v.sections[0].line, 3);
        assert_eq!(v.sections[1].id, "s2");
        // `# Pudim`, blank, `## origem`, its comment, `v2`, the end marker,
        // blank: s2's line is the eighth.
        assert_eq!(v.sections[1].line, 8);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        assert!(
            take_change_note(&ws.ctx).is_some(),
            "the model is still owed the note"
        );
    }

    #[test]
    fn a_save_from_the_app_marks_the_persons_sections_and_keeps_the_agents() {
        let (ws, path) = Ws::with_doc("save", &["origem", "servir"]);
        let base = std::fs::read_to_string(&path).unwrap();
        // While the person edits, an agent writes s2.
        write(&ws.ctx, "s2", Some("gelado"), Status::Done).unwrap();
        let mine = base.replace(
            "[origem] <!-- claudinio: id=s1 status=todo -->",
            "## origem\n<!-- claudinio: id=s1 status=todo -->\nescrito à mão",
        );
        let v = save_human(&ws.root, &ws.store(), &base, &mine).unwrap();
        assert_eq!(v.sections[0].status, "human");
        assert_eq!(v.sections[1].status, "done");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("escrito à mão") && text.contains("gelado"),
            "{text}"
        );
        assert!(
            write(&ws.ctx, "s1", Some("over it"), Status::Done).is_err(),
            "the agent sees the person's text before writing over it"
        );
        assert!(
            take_change_note(&ws.ctx)
                .unwrap()
                .contains("s1 (origem) edited")
        );
    }

    #[test]
    fn a_successor_keeps_the_document_its_memory_and_its_tools_or_takes_the_new_one() {
        let (ws, path) = Ws::with_doc("carry", &["origem"]);
        write(&ws.ctx, "s1", Some("v1"), Status::Done).unwrap();
        edit(&path, "v1", "v2 by hand");
        assert!(
            take_change_note(&ws.ctx).is_some(),
            "the edit is found before the handoff"
        );
        let old_records = crate::agent::persist::load_records(&ws.store()).unwrap();

        let next = SessionStore::create("next", Some(ws.root.to_str().unwrap())).unwrap();
        carry_into(&ws.store(), &old_records, &next, None);
        let records = crate::agent::persist::load_records(&next.path).unwrap();
        assert_eq!(
            crate::agent::persist::linked_document(&records).as_deref(),
            Some(".claudinio/docs/pudim-de-leite.md")
        );
        assert_eq!(
            crate::agent::deferred::loaded_names(&records),
            crate::agent::deferred::DOCUMENT_TOOLS
        );
        let next_ctx = ToolContext {
            session_store_path: Some(next.path.to_string_lossy().into()),
            ..ws.ctx.clone()
        };
        assert!(
            write(&next_ctx, "s1", Some("over it"), Status::Done).is_err(),
            "an edit the predecessor never read is still unread"
        );

        let other = SessionStore::create("other", Some(ws.root.to_str().unwrap())).unwrap();
        carry_into(
            &ws.store(),
            &old_records,
            &other,
            Some(".claudinio/docs/bolo.md"),
        );
        assert_eq!(
            crate::agent::persist::linked_document(
                &crate::agent::persist::load_records(&other.path).unwrap()
            )
            .as_deref(),
            Some(".claudinio/docs/bolo.md")
        );

        let plain = Ws::new("carry-none");
        let none = SessionStore::create("none", Some(plain.root.to_str().unwrap())).unwrap();
        carry_into(&plain.store(), &[], &none, None);
        assert_eq!(
            crate::agent::persist::load_records(&none.path)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn a_session_id_from_the_app_names_a_session_file_and_nothing_else() {
        let ws = Ws::new("session-file");
        assert_eq!(session_file(&ws.root, "s"), Some(ws.store()));
        assert_eq!(session_file(&ws.root, "missing"), None);
        for bad in ["", "../s", "s/../../etc", "a b", "s.jsonl"] {
            assert_eq!(session_file(&ws.root, bad), None, "{bad:?}");
        }
    }

    /// A subagent runs inside one `spawn_agents` call of a run, so when a
    /// run starts none of this session's subagents is alive: a section one
    /// still holds (the app was closed mid-call) would otherwise stay
    /// `doing` forever and refuse every other writer.
    #[test]
    fn a_run_frees_the_sections_subagents_left_claimed() {
        let (ws, path) = Ws::with_doc("stale", &["origem", "servir"]);
        write(&ws.sub("s1", "sub:gone"), "s1", None, Status::Doing).unwrap();
        write(&ws.ctx, "s2", None, Status::Doing).unwrap();
        run_start_blocks(&ws.ctx);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("[origem] <!-- claudinio: id=s1 status=todo -->"),
            "{text}"
        );
        assert!(
            text.contains("id=s2 status=doing by=main"),
            "the main session's own stays"
        );
        write(&ws.ctx, "s1", Some("now mine"), Status::Done).unwrap();
    }

    /// A subagent handed a section it never writes leaves the section as it
    /// found it: a section `main` finished stays `done by=main`, and one a
    /// person edited stays "Edited by you" — whether the subagent ends
    /// (`release`) or never ends and the next run frees its claim. Moving
    /// them to `todo` under the subagent's name would show written text as
    /// unwritten, owned by an agent that wrote nothing.
    #[test]
    fn a_section_a_subagent_gives_up_on_returns_to_how_it_was_before_the_claim() {
        let status_of = |ws: &Ws, id: &str| {
            let v = view(&ws.root, &ws.store()).unwrap();
            let s = v.sections.into_iter().find(|s| s.id == id).unwrap();
            (s.status, s.owner)
        };
        for stale in [false, true] {
            let (ws, path) = Ws::with_doc("give-up", &["origem", "servir"]);
            write(&ws.ctx, "s1", Some("veio de Portugal"), Status::Done).unwrap();
            write(&ws.ctx, "s2", Some("gelado"), Status::Done).unwrap();
            edit(&path, "gelado", "bem gelado");
            read(&ws.ctx, Some("s2")).unwrap();
            assert_eq!(status_of(&ws, "s2"), ("human".into(), None));

            claim(&ws.ctx, "s1", "sub:a").unwrap();
            claim(&ws.ctx, "s2", "sub:b").unwrap();
            if stale {
                run_start_blocks(&ws.ctx);
            } else {
                release(&ws.ctx, "s1", "sub:a");
                release(&ws.ctx, "s2", "sub:b");
            }
            assert_eq!(
                status_of(&ws, "s1"),
                ("done".into(), Some("main".into())),
                "stale={stale}"
            );
            assert_eq!(
                status_of(&ws, "s2"),
                ("human".into(), None),
                "stale={stale}"
            );
        }
    }

    /// A memory written before a field existed still loads: losing it would
    /// make the next read take the file as "what the agents know" and drop
    /// a person's unread edit.
    #[test]
    fn a_document_memory_from_before_the_claim_field_still_loads() {
        let old = r#"{"path":"p","known":"k","unread":["s1"],"pending":[],"next_id":3}"#;
        let state: DocState = serde_json::from_str(old).unwrap();
        assert!(state.unread.contains("s1"));
        assert!(state.claimed.is_empty());
    }

    /// The document is written through `doc_write`, never `edit_file`: an
    /// agent's edit to the file would read as the person's and lock the
    /// agent out of its own section.
    #[tokio::test]
    async fn edit_file_on_the_document_is_refused_and_points_at_doc_write() {
        let (ws, path) = Ws::with_doc("edit-file", &["origem"]);
        let Err(err) = crate::agent::tools::execute(
            "edit_file",
            serde_json::json!({"path": path.to_string_lossy(), "old_string": "[origem]", "new_string": "x"}),
            &ws.ctx,
        )
        .await
        else {
            panic!("edit_file on the document was not refused");
        };
        assert!(err.contains("doc_write"), "{err}");
        assert!(is_linked_document(&ws.ctx, &path.to_string_lossy()));
        assert!(!is_linked_document(
            &ws.ctx,
            &ws.root.join("other.md").to_string_lossy()
        ));
    }

    #[test]
    fn a_deleted_document_is_an_error_not_a_panic() {
        let (ws, path) = Ws::with_doc("gone", &["a"]);
        std::fs::remove_file(&path).unwrap();
        assert!(write(&ws.ctx, "s1", Some("x"), Status::Done).is_err());
        assert!(run_start_blocks(&ws.ctx).is_empty());
        assert_eq!(view(&ws.root, &ws.store()), None);
    }
}
