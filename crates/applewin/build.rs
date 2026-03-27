fn main() {
    // Embed the application icon into the Windows .exe so it appears in
    // Explorer and the taskbar without requiring a separate resource file.
    // winres is only a dependency (and this block only compiles) on Windows.
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("icons/Applewin.ico");
        res.compile().expect("failed to compile Windows resources");
    }
}
