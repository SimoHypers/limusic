use anyhow::Result;
use ort::session::Session;
use std::env;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: cargo run -p karaoke --example inspect -- <path-to-model.onnx>");
        std::process::exit(1);
    }
    let path = &args[1];

    // On Windows: ORT_DYLIB_PATH = absolute path of onnxruntime.dll. Must precede any ort call.
    let _rt = karaoke::runtime::from_env()?;
    let mut b = Session::builder()?;
    let session = b.commit_from_file(path)?;

    for (i, input) in session.inputs().iter().enumerate() {
        println!("input  {i}: {}  {:?}", input.name(), input.dtype());
    }
    for (i, output) in session.outputs().iter().enumerate() {
        println!("output {i}: {}  {:?}", output.name(), output.dtype());
    }

    Ok(())
}
