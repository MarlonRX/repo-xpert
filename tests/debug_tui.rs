//! Diagnóstico F2: qué cambios reporta el diff engine para src/tui.rs.
//! Correr con: $env:GADV_DEBUG_REPO="D:\projects\git-hero"; cargo test --test debug_tui -- --ignored --nocapture

#[test]
#[ignore = "diagnóstico manual sobre un repo real"]
fn dump_changes_for_tui_rs() {
    let path = option_env!("GADV_DEBUG_REPO").unwrap_or("D:\\projects\\git-hero");
    let repo = std::path::Path::new(path);
    let h = gadv::engine::scan_history(repo, 50_000).expect("scan");
    let tui = h
        .paths
        .iter()
        .position(|p| p.ends_with("tui.rs"))
        .expect("tui.rs");
    for c in &h.commits {
        if c.oid[0] == 0x56 && c.oid[1] == 0x2f {
            for f in &c.files {
                println!(
                    "INITIAL FILE {} adds={} dels={} bin={}",
                    h.paths[f.file.0 as usize], f.adds, f.dels, f.binary
                );
            }
        }
        for f in &c.files {
            if f.file.0 as usize == tui {
                println!(
                    "oid={:02x}{:02x}{:02x}.. adds={} dels={} binary={}",
                    c.oid[0],
                    c.oid[1],
                    c.oid[2],
                    f.adds,
                    f.dels,
                    f.binary
                );
            }
        }
    }
}
