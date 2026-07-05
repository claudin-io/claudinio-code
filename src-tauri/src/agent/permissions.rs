use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub enum PermissionLevel {
    Auto,
    RequiresApproval,
}

pub fn tool_permission(name: &str) -> PermissionLevel {
    match name {
        "edit_file" | "batch_edit" | "write_file" => PermissionLevel::RequiresApproval,
        _ => PermissionLevel::Auto,
    }
}
