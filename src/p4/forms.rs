pub fn change_form(description: &str, files: &[String]) -> String {
    let mut body = format!(
        "Change: new\n\nDescription:\n\t{}\n\nFiles:\n",
        description.replace('\n', "\n\t")
    );
    for file in files {
        body.push('\t');
        body.push_str(file);
        body.push('\n');
    }
    body
}
