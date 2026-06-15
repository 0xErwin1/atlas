use std::io::{self, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let spec = atlas_server::routes::openapi::openapi();
    let json = serde_json::to_string_pretty(&spec)?;

    let stdout = io::stdout();
    let mut handle = stdout.lock();
    writeln!(handle, "{json}")?;

    Ok(())
}
