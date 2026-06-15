use crate::error::{P4McpError, Result};

pub fn change_form(description: &str, files: &[String]) -> String {
    change_form_for("new", description, files)
}

pub fn change_form_for(change: &str, description: &str, files: &[String]) -> String {
    let mut body = format!(
        "Change: {}\n\nDescription:\n\t{}\n\nFiles:\n",
        change,
        description.replace('\n', "\n\t")
    );
    for file in files {
        body.push('\t');
        body.push_str(file);
        body.push('\n');
    }
    body
}

pub fn patch_change_description_form(existing: &str, description: &str) -> Result<String> {
    let description_start =
        existing
            .find("Description:\n")
            .ok_or_else(|| P4McpError::InvalidInput {
                message: "Description field is required".to_string(),
            })?;
    let block_start = description_start + "Description:\n".len();
    let block_end = existing[block_start..]
        .find("\n\n")
        .map(|offset| block_start + offset)
        .unwrap_or(existing.len());

    let mut patched = String::new();
    patched.push_str(&existing[..block_start]);
    patched.push_str(&indent_description(description));
    patched.push_str(&existing[block_end..]);
    Ok(patched)
}

fn indent_description(description: &str) -> String {
    description
        .lines()
        .map(|line| format!("\t{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}
