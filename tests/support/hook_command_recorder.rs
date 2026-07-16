use serde::Serialize;
use std::env;
use std::fs;
use std::io::{self, Read};

#[derive(Serialize)]
struct HookCommandRecord {
    args: Vec<String>,
    cwd: String,
    stdin: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output_path = env::var_os("GIT_AI_HOOK_RECORDER_OUTPUT")
        .ok_or("GIT_AI_HOOK_RECORDER_OUTPUT is required")?;
    let args = env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let cwd = env::current_dir()?.to_string_lossy().into_owned();
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let record = HookCommandRecord { args, cwd, stdin };

    fs::write(output_path, serde_json::to_vec(&record)?)?;
    Ok(())
}
