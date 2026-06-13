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
