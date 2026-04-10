use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();

    // Read VERSION.txt
    let version_path = Path::new(&crate_dir).join("VERSION.txt");
    let version_str = fs::read_to_string(&version_path)
        .unwrap_or_else(|_| "0.1.0.100".to_string());
    let version_str = version_str.trim();

    // Parse version (format: 0.1.0.100)
    let parts: Vec<&str> = version_str.split('.').collect();
    let (major, minor, patch, build) = if parts.len() >= 4 {
        (
            parts[0].parse::<u64>().unwrap_or(0),
            parts[1].parse::<u64>().unwrap_or(1),
            parts[2].parse::<u64>().unwrap_or(0),
            parts[3].parse::<u64>().unwrap_or(100),
        )
    } else {
        (0, 1, 0, 100)
    };

    println!("cargo:rerun-if-changed=VERSION.txt");
    println!("cargo:rerun-if-changed=tail-f_plot.exe.manifest");
    println!("cargo:rustc-env=FULL_VERSION={}", version_str);

    // Windows resource info
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();

        res.set_version_info(winresource::VersionInfo::FILEVERSION,
            (major << 48) | (minor << 32) | (patch << 16) | build);
        res.set_version_info(winresource::VersionInfo::PRODUCTVERSION,
            (major << 48) | (minor << 32) | (patch << 16) | build);

        res.set("CompanyName", "HANA");
        res.set("FileDescription", "Graph rendering tool to track CSV/TSV files being updated in real-time, similar to the [tail -f] command");
        res.set("FileVersion", version_str);
        res.set("InternalName", "tail-f_plot");
        res.set("LegalCopyright", "\u{24d2}2026 HANA");
        res.set("OriginalFilename", "tail-f_plot.exe");
        res.set("ProductName", "HANA Cryogenics - tail-f Plot");
        let product_version = format!("{}.{}.{}", major, minor, patch);
        res.set("ProductVersion", &product_version);

        res.set_icon("hana.ico");

        // Read manifest and replace %%VERSION%% -> major.minor.patch.0
        let manifest_path = Path::new(&crate_dir).join("tail-f_plot.exe.manifest");
        let manifest = fs::read_to_string(&manifest_path)
            .expect("Failed to read tail-f_plot.exe.manifest");
        let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "x86_64".into());
        let proc_arch = match arch.as_str() {
            "aarch64" => "arm64",
            "x86" => "x86",
            _ => "amd64",
        };
        let manifest = manifest
            .replace("%%VERSION%%", &format!("{}.{}.{}.0", major, minor, patch))
            .replace("%%ARCH%%", proc_arch);
        res.set_manifest(&manifest);

        if let Err(e) = res.compile() {
            eprintln!("Warning: Failed to compile Windows resource: {}", e);
        }
    }
}
