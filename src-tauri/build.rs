// The CLI definition is shared with the binary so the man page always matches `--help`.
#[allow(dead_code)]
#[path = "src/cli.rs"]
mod cli;

fn main() {
    write_man_page();
    tauri_build::build()
}

/// man/puru-nucleus.1 plus one page per subcommand (puru-nucleus-backup.1, …), installed by the
/// .deb into /usr/share/man/man1 (tauri.conf.json → deb.files).
fn write_man_page() {
    use clap::CommandFactory;
    println!("cargo:rerun-if-changed=src/cli.rs");

    let cmd = cli::Cli::command().name("puru-nucleus").bin_name("puru-nucleus");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("man");
    let _ = std::fs::remove_dir_all(&dir); // drop pages of removed subcommands
    std::fs::create_dir_all(&dir).expect("create man/");
    clap_mangen::generate_to(cmd, &dir).expect("write man pages");
}
