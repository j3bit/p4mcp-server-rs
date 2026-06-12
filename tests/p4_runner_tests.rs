use std::{collections::BTreeMap, fs, sync::Arc};

use p4mcp_server_rs::p4::runner::{OutputMode, P4Executor, P4Invocation, TokioP4Executor};
use tempfile::tempdir;

#[tokio::test]
async fn json_invocation_adds_tagged_json_flags_before_command() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("p4");
    fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$P4_FAKE_ARGS\"\nprintf '{\"code\":\"info\",\"data\":\"ok\"}\\n'\n",
    )
    .unwrap();
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(&script)
        .status()
        .unwrap();

    let args_file = dir.path().join("args.txt");
    let executor: Arc<dyn P4Executor> = Arc::new(TokioP4Executor::new(script));
    let mut env = BTreeMap::new();
    env.insert(
        "P4_FAKE_ARGS".to_string(),
        args_file.to_string_lossy().to_string(),
    );
    let output = executor
        .run(
            P4Invocation {
                args: vec!["info".into()],
                stdin: None,
                mode: OutputMode::JsonLines,
            },
            env,
        )
        .await
        .unwrap();

    assert_eq!(
        fs::read_to_string(args_file).unwrap(),
        "-z\ntag\n-Mj\ninfo\n"
    );
    assert_eq!(output.records[0]["data"], "ok");
}

#[tokio::test]
async fn nonzero_exit_is_error() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("p4");
    fs::write(&script, "#!/bin/sh\necho 'bad auth' 1>&2\nexit 1\n").unwrap();
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(&script)
        .status()
        .unwrap();

    let executor = TokioP4Executor::new(script);
    let err = executor
        .run(
            P4Invocation {
                args: vec!["opened".into()],
                stdin: None,
                mode: OutputMode::Text,
            },
            BTreeMap::new(),
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("bad auth"));
}

#[tokio::test]
async fn stdin_reaches_p4_process() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("p4");
    fs::write(
        &script,
        "#!/bin/sh\ncat > \"$P4_FAKE_STDIN\"\nprintf 'ok\\n'\n",
    )
    .unwrap();
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(&script)
        .status()
        .unwrap();

    let stdin_file = dir.path().join("stdin.txt");
    let mut env = BTreeMap::new();
    env.insert(
        "P4_FAKE_STDIN".to_string(),
        stdin_file.to_string_lossy().to_string(),
    );

    let executor = TokioP4Executor::new(script);
    let output = executor
        .run(
            P4Invocation {
                args: vec!["submit".into(), "-i".into()],
                stdin: Some("Change: new\nDescription: form\n".into()),
                mode: OutputMode::Text,
            },
            env,
        )
        .await
        .unwrap();

    assert_eq!(
        fs::read_to_string(stdin_file).unwrap(),
        "Change: new\nDescription: form\n"
    );
    assert_eq!(output.text["stdout"], "ok\n");
}

#[tokio::test]
async fn nonzero_exit_preserves_status_stdout_and_stderr() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("p4");
    fs::write(
        &script,
        "#!/bin/sh\nprintf 'partial result\\n'\nprintf 'bad auth\\n' 1>&2\nexit 7\n",
    )
    .unwrap();
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(&script)
        .status()
        .unwrap();

    let executor = TokioP4Executor::new(script);
    let err = executor
        .run(
            P4Invocation {
                args: vec!["opened".into()],
                stdin: None,
                mode: OutputMode::Text,
            },
            BTreeMap::new(),
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("exit status: 7"));
    assert!(err.contains("partial result"));
    assert!(err.contains("bad auth"));
}
