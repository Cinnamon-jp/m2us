pub mod state;

fn main() {
    #[cfg(target_os = "macos")]
    {
        // To be implemented in Step 2
    }

    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("m2us is only supported on macOS (Apple Silicon).");
        std::process::exit(1);
    }
}
