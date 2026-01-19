fn main() {
    // Embed Windows icon and metadata
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();

        // Set icon (relative to crate root)
        res.set_icon("../../assets/dendrite.ico");

        // Set Windows metadata
        res.set("ProductName", "Dendrite");
        res.set("FileDescription", "CogniPilot Hardware Discovery Daemon");
        res.set("LegalCopyright", "CogniPilot Foundation");

        if let Err(e) = res.compile() {
            eprintln!("Warning: Failed to set Windows resources: {}", e);
            // Don't fail the build - icon is nice-to-have
        }
    }
}
