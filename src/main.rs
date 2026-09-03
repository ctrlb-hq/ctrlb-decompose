#[cfg(feature = "cli")]
fn main() -> anyhow::Result<()> {
    use clap::Parser;
    use ctrlb_decompose::{Args, run};

    let args = Args::parse();
    run(args)
}

#[cfg(not(feature = "cli"))]
fn main() {
    // Binary requires the "cli" feature. Use as a library or via WASM instead.
    eprintln!("ctrlb-decompose: binary requires the 'cli' feature");
    std::process::exit(1);
}
