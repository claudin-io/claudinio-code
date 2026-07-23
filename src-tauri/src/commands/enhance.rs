use crate::agent::provider::{self, AgentConfig};
use crate::code_intel::db::SemanticSearchResult;
use crate::state::{AppState, WorkspaceState};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageContext {
    pub role: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnhanceContext {
    pub messages: Vec<MessageContext>,
    pub mode: String,
    pub mentioned_files: Vec<String>,
    pub active_task_titles: Vec<String>,
    pub project_summary: String,
}

const ENHANCER_SYSTEM_PROMPT: &str = r#"You are a prompt REWRITER inside Claudinio Code — an AI coding agent for a native desktop app. The user writes prompts in a chat to give instructions to the agent, which can read files, edit code, run commands, and spawn subagents.

Your ONLY job is to rewrite the user's DRAFT PROMPT into a clearer prompt addressed TO the agent. You are NOT the agent: never answer the draft, never investigate, never trace code, never run or suggest commands, never begin solving the task. If your output would start with something like "Looking at the code..." or contain an answer, it is wrong — it must read as an instruction the user could send.

The agent has two modes:
- Brain mode: read-only exploration + planning. Cannot edit files or run commands.
- Builder mode: full tool access — edits, runs commands, spawns subagents.

GUIDELINES:
- Preserve the user's INTENT, @file mentions, <goal>, <skill>, <tag> markup verbatim.
- Write the rewritten prompt in the SAME LANGUAGE as the draft.
- Be concise: match the user's level of brevity. Do not expand a short question into a long plan. If the user asks a simple question, keep the prompt simple.
- Clarify ambiguity and fix spelling/grammar — but don't add steps, scripts, or structure the user didn't ask for.
- The RELEVANT CODE and GIT STATE sections are reference material retrieved automatically. When entries clearly match the draft's intent, you SHOULD anchor the rewrite with their file paths and symbol names — a draft that names no concrete code counts as improvable, not "already effective". Still do NOT expand scope, add steps, or grow the prompt much beyond its original length. If the retrieved code seems unrelated to the draft, ignore it entirely.
- Return the draft unchanged ONLY when it is already unambiguous AND there is no clearly matching RELEVANT CODE to anchor it with.
- Output ONLY the rewritten prompt text — no preamble, no markdown fences, no answer."#;

const QUERY_GEN_SYSTEM_PROMPT: &str = r#"You extract code-search queries. Given a draft prompt (in any language) that a user wrote for an AI coding agent, output 1-3 short ENGLISH code-search queries (behavior descriptions or identifiers) that would locate the code the draft is about.

Rules:
- One query per line. No numbering, no quotes, no commentary.
- Always in English (the search index is English-only), even if the draft is in another language.
- Keep identifiers from the draft verbatim (e.g. function or file names).
- If the draft is not about code in the project (greeting, general question), output exactly: NONE"#;

/// Cap the project context so it grounds the enhancer without dominating it.
const MAX_SEARCH_QUERIES: usize = 3;
const RESULTS_PER_QUERY: usize = 4;
const MAX_TOTAL_RESULTS: usize = 8;
const SNIPPET_MAX_LINES: usize = 15;
const SNIPPET_MAX_CHARS: usize = 1000;
const MAX_GIT_FILES: usize = 20;

#[tauri::command]
pub async fn enhance_prompt(
    workspace: String,
    prompt: String,
    context: EnhanceContext,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let ws = state.workspace(&workspace).await?;
    let config = {
        let c = state.config.lock().await;
        c.clone()
    };

    let model = config.model_for_mode("brain").to_string();

    // Best-effort project grounding: neither git nor the index may be
    // available (fresh workspace, embedder still loading) — the enhancer
    // must keep working exactly as before in that case.
    let git_section = build_git_section(&workspace).await;
    let code_section =
        build_code_section(&ws, &state, &config, &model, &prompt, &context.messages).await;

    // Build the user message with context
    let mut user_message = String::new();

    // Conversation history (last 10, truncated)
    if !context.messages.is_empty() {
        user_message.push_str("=== CONVERSATION HISTORY ===\n");
        let start = context.messages.len().saturating_sub(10);
        for msg in &context.messages[start..] {
            let truncated = if msg.text.len() > 500 {
                format!("{}...", &msg.text[..500])
            } else {
                msg.text.clone()
            };
            user_message.push_str(&format!("[{}]: {}\n", msg.role, truncated));
        }
        user_message.push('\n');
    }

    // Current mode
    user_message.push_str(&format!("=== CURRENT MODE: {} ===\n\n", context.mode));

    // Mentioned files
    if !context.mentioned_files.is_empty() {
        user_message.push_str("=== MENTIONED FILES ===\n");
        for file in &context.mentioned_files {
            user_message.push_str(&format!("- {}\n", file));
        }
        user_message.push('\n');
    }

    // Active tasks
    if !context.active_task_titles.is_empty() {
        user_message.push_str("=== ACTIVE TASKS ===\n");
        for task in &context.active_task_titles {
            user_message.push_str(&format!("- {}\n", task));
        }
        user_message.push('\n');
    }

    // Project summary
    if !context.project_summary.is_empty() {
        user_message.push_str(&format!("=== PROJECT: {} ===\n\n", context.project_summary));
    }

    if let Some(git) = git_section {
        user_message.push_str(&git);
        user_message.push('\n');
    }

    if let Some(code) = code_section {
        user_message.push_str(&code);
        user_message.push('\n');
    }

    // The user's draft prompt
    user_message.push_str("=== DRAFT PROMPT ===\n");
    user_message.push_str(&prompt);
    // Re-anchor the role after the (potentially long) context: without this,
    // rich RELEVANT CODE sections make the model answer the draft instead of
    // rewriting it.
    user_message.push_str(
        "\n\n=== YOUR TASK ===\nRewrite the DRAFT PROMPT above following your guidelines. \
         Do not answer or investigate it. Output only the rewritten prompt, in the same \
         language as the draft.",
    );

    let reply =
        provider::one_shot(&config, &model, ENHANCER_SYSTEM_PROMPT, &user_message, 4096).await?;

    // The draft is the only source of truth for <goal> tags (they create
    // mandatory golden tasks). If the enhancer hallucinates or "helpfully"
    // wraps inferred intent in <goal> that wasn't in the draft, strip the
    // tags from the rewrite so no golden task gets created behind the
    // user's back.
    let (_, draft_goals) = crate::agent::session::parse_goals(&prompt);
    let reply = if draft_goals.is_empty() {
        strip_goal_tags(&reply)
    } else {
        reply
    };

    Ok(reply)
}

/// Remove `<goal>`/`</goal>` markup while keeping the enclosed text, so an
/// enhancer rewrite can't manufacture a goal tag the user never wrote.
fn strip_goal_tags(text: &str) -> String {
    text.replace("<goal>", "").replace("</goal>", "")
}

async fn build_git_section(workspace: &str) -> Option<String> {
    let branch = super::git::git_branch(workspace.to_string()).await.ok();
    let status = super::git::git_status(workspace.to_string()).await.ok();

    let mut section = String::from("=== GIT STATE ===\n");
    let mut has_content = false;
    if let Some(branch) = branch {
        section.push_str(&format!("branch: {}\n", branch));
        has_content = true;
    }
    if let Some(status) = status {
        if status.has_changes {
            section.push_str("changed files:\n");
            for f in status.files.iter().take(MAX_GIT_FILES) {
                section.push_str(&format!("- [{}] {}\n", f.status, f.path));
            }
            if status.files.len() > MAX_GIT_FILES {
                section.push_str(&format!(
                    "... and {} more\n",
                    status.files.len() - MAX_GIT_FILES
                ));
            }
            has_content = true;
        }
    }
    has_content.then_some(section)
}

/// Two-step retrieval: a cheap LLM call turns the (possibly non-English)
/// draft into English search queries, then the local semantic index is
/// queried. Any failure along the way silently yields no section.
async fn build_code_section(
    ws: &WorkspaceState,
    state: &State<'_, AppState>,
    config: &AgentConfig,
    model: &str,
    prompt: &str,
    messages: &[MessageContext],
) -> Option<String> {
    let (_, symbols, _) = ws.index_db.index_stats().ok()?;
    if symbols == 0 {
        eprintln!("[enhance] index has no symbols — skipping code retrieval");
        return None;
    }

    let queries = match generate_search_queries(config, model, prompt, messages).await {
        Some(q) if !q.is_empty() => q,
        Some(_) => {
            eprintln!("[enhance] query generation returned no usable queries");
            return None;
        }
        None => {
            eprintln!("[enhance] query generation returned NONE or failed");
            return None;
        }
    };
    eprintln!("[enhance] search queries: {queries:?}");

    let embedder = state.embedding_model.lock().await.clone();
    if embedder.is_none() {
        eprintln!("[enhance] embedding model not loaded — using lexical fallback");
    }

    let mut results: Vec<SemanticSearchResult> = Vec::new();
    for query in &queries {
        // Hybrid search works with or without the embedder: BM25 alone
        // still returns real line ranges while the model loads.
        let query_vec: Option<Vec<f32>> = match &embedder {
            Some(embedder) => {
                let embedder = embedder.clone();
                let query_owned = query.clone();
                tokio::task::spawn_blocking(move || {
                    let mut model = embedder.lock().map_err(|e| format!("embedder lock: {e}"))?;
                    model.encode_query(&query_owned)
                })
                .await
                .ok()?
                .ok()
            }
            None => None,
        };
        let hits = ws
            .index_db
            .search_hybrid(query, query_vec.as_deref(), RESULTS_PER_QUERY)
            .ok()?;
        for hit in hits {
            if !results.iter().any(|r| r.symbol_id == hit.symbol_id) {
                results.push(hit);
            }
        }
    }
    results.truncate(MAX_TOTAL_RESULTS);
    eprintln!("[enhance] retrieved {} code results", results.len());
    if results.is_empty() {
        return None;
    }
    attach_snippets(&mut results);

    let root = ws.root.to_string_lossy();
    let mut section = String::from("=== RELEVANT CODE (from project index) ===\n");
    for r in &results {
        let path = r
            .file_path
            .strip_prefix(root.as_ref())
            .map(|p| p.trim_start_matches('/'))
            .unwrap_or(&r.file_path);
        section.push_str(&format!(
            "- {} `{}` ({}:{})",
            r.kind, r.name, path, r.start_line
        ));
        if let Some(sig) = &r.signature {
            section.push_str(&format!(" — {}", sig));
        }
        section.push('\n');
        if let Some(snippet) = &r.snippet {
            section.push_str("```\n");
            section.push_str(snippet);
            section.push_str("\n```\n");
        }
    }
    Some(section)
}

async fn generate_search_queries(
    config: &AgentConfig,
    model: &str,
    prompt: &str,
    messages: &[MessageContext],
) -> Option<Vec<String>> {
    let mut input = String::new();
    // A little recent context helps disambiguate short drafts like "fix it".
    let start = messages.len().saturating_sub(3);
    for msg in &messages[start..] {
        let truncated = if msg.text.len() > 300 {
            format!("{}...", &msg.text[..300])
        } else {
            msg.text.clone()
        };
        input.push_str(&format!("[{}]: {}\n", msg.role, truncated));
    }
    input.push_str("=== DRAFT ===\n");
    input.push_str(prompt);

    // Generous budget: the model may spend output tokens on a thinking
    // block before the query lines.
    let reply = provider::one_shot(config, model, QUERY_GEN_SYSTEM_PROMPT, &input, 500)
        .await
        .ok()?;

    let queries: Vec<String> = reply
        .lines()
        .map(|l| l.trim().trim_start_matches('-').trim().to_string())
        .filter(|l| !l.is_empty() && l.as_str() != "NONE")
        .take(MAX_SEARCH_QUERIES)
        .collect();
    if reply.trim() == "NONE" {
        return None;
    }
    Some(queries)
}

fn attach_snippets(results: &mut [SemanticSearchResult]) {
    for r in results.iter_mut() {
        if r.snippet.is_some() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&r.file_path) else {
            continue;
        };
        // start_line/end_line are 1-based (inclusive), lines() is 0-based.
        let start = r.start_line.max(1) as usize;
        let end = r.end_line.max(r.start_line) as usize;
        let mut snippet: String = content
            .lines()
            .skip(start.saturating_sub(1))
            .take((end - start + 1).min(SNIPPET_MAX_LINES))
            .collect::<Vec<_>>()
            .join("\n");
        if snippet.len() > SNIPPET_MAX_CHARS {
            let cut = snippet
                .char_indices()
                .take_while(|(i, _)| *i < SNIPPET_MAX_CHARS)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
            snippet.truncate(cut);
            snippet.push_str("\n… [truncated]");
        }
        if !snippet.is_empty() {
            r.snippet = Some(snippet);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_goal_tags_removes_markup_keeps_text() {
        let input = "Please <goal>ship the feature</goal> by Friday";
        assert_eq!(strip_goal_tags(input), "Please ship the feature by Friday");
    }

    #[test]
    fn strip_goal_tags_noop_without_tags() {
        let input = "Please ship the feature by Friday";
        assert_eq!(strip_goal_tags(input), input);
    }
}
