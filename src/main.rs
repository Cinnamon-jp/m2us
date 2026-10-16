pub mod state;

#[cfg(target_os = "macos")]
mod macos;

fn main() {
    #[cfg(target_os = "macos")]
    {
        if let Err(err) = macos::run() {
            eprintln!("m2us error: {err}");
            std::process::exit(1);
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("m2us is only supported on macOS (Apple Silicon).");
        std::process::exit(1);
    }
}
