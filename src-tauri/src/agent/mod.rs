pub mod app_sign;
pub mod approval;
pub mod eventbus;
pub mod install_id;
pub mod mcp;
pub mod permissions;
pub mod persist;
/// The phase-1 prova real. Test-only: it exercises the event bus and the approval
/// registry together, on one session, which is the thing neither's unit tests show.
#[cfg(test)]
mod prova_real_phase1;
pub mod provider;
pub mod run_state;
pub mod session;
pub mod skills;
pub mod subagent;
pub mod tools;
pub mod transition;
