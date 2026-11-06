fn main() {
    // `include_bytes!` cannot see a missing file, so the geoip module only
    // embeds GeoLite2 when this cfg is set. A later add or remove rebuilds.
    println!("cargo:rustc-check-cfg=cfg(geoip_embedded)");
    println!("cargo:rerun-if-changed=GeoLite2-City.mmdb");
    if std::path::Path::new("GeoLite2-City.mmdb").is_file() {
        println!("cargo:rustc-cfg=geoip_embedded");
    }

    #[cfg(feature = "self-hosted")]
    web_ui::compress_bundle();
}

/// Gzips the UI bundle into `$OUT_DIR/web-ui`, which `web_ui.rs` embeds. Text
/// files are stored only as `{name}.gz` and always served gzipped; formats
/// that are already compressed are copied as they are.
#[cfg(feature = "self-hosted")]
mod web_ui {
    use std::fs;
    use std::io::Write;
    use std::path::Path;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    const DIST: &str = "../web/app/dist-ui";
    const COMPRESSIBLE: &[&str] = &["html", "js", "mjs", "css", "json", "svg", "ico", "txt"];

    pub(super) fn compress_bundle() {
        // A directory here makes cargo rerun this script when any file under it changes.
        println!("cargo:rerun-if-changed={DIST}");
        let dist = Path::new(DIST);
        if !dist.join("dashboard/index.html").is_file() {
            panic!("the self-hosted feature needs {DIST}; run `pnpm build:bundle` in web/ first");
        }
        let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("web-ui");
        if out.exists() {
            fs::remove_dir_all(&out).expect("clear previous compressed bundle");
        }
        copy_dir(dist, &out);
    }

    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).expect("create bundle output dir");
        for entry in fs::read_dir(from).expect("read bundle dir") {
            let entry = entry.expect("read bundle entry");
            let path = entry.path();
            let name = entry.file_name();
            if path.is_dir() {
                copy_dir(&path, &to.join(&name));
                continue;
            }
            let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
            if COMPRESSIBLE.contains(&extension) {
                let contents = fs::read(&path).expect("read bundle file");
                let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
                encoder.write_all(&contents).expect("gzip bundle file");
                let mut gz_name = name.clone();
                gz_name.push(".gz");
                fs::write(
                    to.join(gz_name),
                    encoder.finish().expect("gzip bundle file"),
                )
                .expect("write gzipped bundle file");
            } else {
                fs::copy(&path, to.join(&name)).expect("copy bundle file");
            }
        }
    }
}
