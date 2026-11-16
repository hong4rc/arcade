//! Every snapshot that a released version wrote is kept under `tests/snapshots/`, and this
//! program must still build every session from each of them (the rehearsal does exactly that,
//! apart from taking real descriptors). A format change that cannot read an old file fails here.

use std::os::fd::AsRawFd;

#[test]
fn every_snapshot_a_release_wrote_can_be_read() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("the snapshots directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "snap"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no snapshot is kept under {}",
        dir.display()
    );
    for file in files {
        let work = std::path::PathBuf::from(format!(
            "/tmp/arc-fix-{}-{}",
            std::process::id(),
            file.file_stem().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&work).unwrap();
        let config = arcade_daemon::DaemonConfig::new(work.join("s"));
        let open = std::fs::File::open(&file).unwrap();
        let answer = arcade_daemon::rehearse(&config, open.as_raw_fd());
        let _ = std::fs::remove_dir_all(&work);
        assert!(
            answer.starts_with("ok "),
            "{} cannot be read: {answer}",
            file.display()
        );
        // The fixture has what the generator put in it: windows, a split and a full-screen pane.
        let fields: Vec<&str> = answer.split(' ').collect();
        assert_eq!(fields[2], "1", "sessions in {}", file.display());
        assert_eq!(fields[3], "2", "windows in {}", file.display());
        assert_eq!(fields[4], "3", "panes in {}", file.display());
        assert_eq!(fields[5], "0", "degraded panes in {}", file.display());
    }
}
