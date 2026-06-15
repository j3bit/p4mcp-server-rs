use crate::error::{P4McpError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFormPatch {
    pub root: Option<String>,
    pub description: Option<String>,
    pub options: Option<String>,
    pub line_end: Option<String>,
    pub view: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamFormPatch {
    pub stream: Option<String>,
    pub stream_type: Option<String>,
    pub parent: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub options: Option<String>,
    pub parent_view: Option<String>,
    pub paths: Option<Vec<String>>,
    pub remapped: Option<Vec<String>>,
    pub ignored: Option<Vec<String>>,
}

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

pub fn patch_workspace_form(existing: &str, patch: &WorkspaceFormPatch) -> Result<String> {
    let mut patched = existing.to_string();
    if let Some(root) = &patch.root {
        patched = replace_single_line_field(&patched, "Root", root);
    }
    if let Some(description) = &patch.description {
        patched = upsert_indented_block(&patched, "Description", &indent_lines(description));
    }
    if let Some(options) = &patch.options {
        patched = replace_single_line_field(&patched, "Options", options);
    }
    if let Some(line_end) = &patch.line_end {
        patched = replace_single_line_field(&patched, "LineEnd", line_end);
    }
    if let Some(view) = &patch.view {
        patched = upsert_indented_block(&patched, "View", &indent_lines(&view.join("\n")));
    }
    Ok(patched)
}

pub fn patch_stream_form(existing: &str, patch: &StreamFormPatch) -> Result<String> {
    let mut patched = existing.to_string();
    if let Some(stream) = &patch.stream {
        patched = replace_single_line_field(&patched, "Stream", stream);
    }
    if let Some(stream_type) = &patch.stream_type {
        patched = replace_single_line_field(&patched, "Type", stream_type);
    }
    if let Some(parent) = &patch.parent {
        patched = replace_single_line_field(&patched, "Parent", parent);
    }
    if let Some(name) = &patch.name {
        patched = replace_single_line_field(&patched, "Name", name);
    }
    if let Some(description) = &patch.description {
        patched = upsert_indented_block(&patched, "Description", &indent_lines(description));
    }
    if let Some(options) = &patch.options {
        patched = replace_single_line_field(&patched, "Options", options);
    }
    if let Some(parent_view) = &patch.parent_view {
        patched = replace_single_line_field(&patched, "ParentView", parent_view);
    }
    if let Some(paths) = &patch.paths {
        patched = replace_list_block(&patched, "Paths", paths);
    }
    if let Some(remapped) = &patch.remapped {
        patched = replace_list_block(&patched, "Remapped", remapped);
    }
    if let Some(ignored) = &patch.ignored {
        patched = replace_list_block(&patched, "Ignored", ignored);
    }
    Ok(patched)
}

fn indent_description(description: &str) -> String {
    description
        .lines()
        .map(|line| format!("\t{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn replace_single_line_field(existing: &str, field: &str, value: &str) -> String {
    let prefix = format!("{field}:");
    let mut patched = String::new();
    let mut replaced = false;
    for line in existing.split_inclusive('\n') {
        let line_without_newline = line.strip_suffix('\n').unwrap_or(line);
        if line_without_newline.starts_with(&prefix) {
            patched.push_str(&format!("{field}: {value}"));
            if line.ends_with('\n') {
                patched.push('\n');
            }
            replaced = true;
        } else {
            patched.push_str(line);
        }
    }
    if !replaced {
        if !patched.is_empty() && !patched.ends_with('\n') {
            patched.push('\n');
        }
        patched.push_str(&format!("{field}: {value}\n"));
    }
    patched
}

fn upsert_indented_block(existing: &str, field: &str, body: &str) -> String {
    match indented_block_bounds(existing, field) {
        Some((block_start, block_end)) => {
            let mut patched = String::new();
            patched.push_str(&existing[..block_start]);
            patched.push_str(body);
            patched.push_str("\n\n");
            patched.push_str(&existing[block_end..]);
            patched
        }
        None => insert_indented_block(existing, field, body),
    }
}

fn insert_indented_block(existing: &str, field: &str, body: &str) -> String {
    let insert_at = existing
        .find('\n')
        .map(|offset| offset + 1)
        .unwrap_or(existing.len());
    let mut patched = String::new();
    patched.push_str(&existing[..insert_at]);
    patched.push_str(&format!("{field}:\n{body}\n"));
    patched.push_str(&existing[insert_at..]);
    patched
}

fn indented_block_bounds(existing: &str, field: &str) -> Option<(usize, usize)> {
    let header = format!("{field}:");
    let mut offset = 0;
    for line in existing.split_inclusive('\n') {
        let line_without_newline = line.trim_end_matches('\n').trim_end_matches('\r');
        let next_offset = offset + line.len();
        if line_without_newline == header {
            return Some((next_offset, find_indented_block_end(existing, next_offset)));
        }
        offset = next_offset;
    }
    None
}

fn find_indented_block_end(existing: &str, block_start: usize) -> usize {
    let mut offset = block_start;
    for line in existing[block_start..].split_inclusive('\n') {
        let line_without_newline = line.trim_end_matches('\n').trim_end_matches('\r');
        if !line_without_newline.is_empty()
            && !line_without_newline.starts_with('\t')
            && line_without_newline.contains(':')
        {
            return offset;
        }
        offset += line.len();
    }
    existing.len()
}

fn indent_lines(text: &str) -> String {
    if text.is_empty() {
        return "\t".to_string();
    }
    text.lines()
        .map(|line| format!("\t{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn replace_list_block(existing: &str, field: &str, values: &[String]) -> String {
    upsert_indented_block(existing, field, &indent_lines(&values.join("\n")))
}
