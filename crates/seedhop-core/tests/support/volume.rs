use std::io::Write;

/// Only the runner may supply this path: it creates a dedicated <=64 MiB mounted image.
/// Refuse ordinary directories and large devices before allocating any filler bytes.
pub fn fault_volume() -> std::path::PathBuf {
    use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
    let path = std::path::PathBuf::from(
        std::env::var_os("SEEDHOP_FAULT_VOLUME").expect("use scripts/faults.py"),
    );
    let parent = path.parent().unwrap();
    assert_ne!(
        std::fs::metadata(&path).unwrap().dev(),
        std::fs::metadata(parent).unwrap().dev(),
        "fault path must be a mount point"
    );
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: the path is a valid C string and statvfs initializes stats on success.
    assert_eq!(
        unsafe { libc::statvfs(name.as_ptr(), stats.as_mut_ptr()) },
        0
    );
    // SAFETY: statvfs succeeded above.
    let stats = unsafe { stats.assume_init() };
    let capacity = u128::from(stats.f_blocks) * u128::from(stats.f_frsize);
    assert!(
        capacity > 0 && capacity <= 64 * 1024 * 1024,
        "refusing to fill a device of {capacity} bytes"
    );
    path
}

pub fn fill_volume(path: &std::path::Path) {
    let mut file = std::fs::File::create(path).unwrap();
    let block = [0x5a; 4096];
    for _ in 0..=64 * 1024 * 1024 / block.len() {
        match file.write_all(&block) {
            Ok(()) => (),
            Err(error) => {
                assert_eq!(error.raw_os_error(), Some(libc::ENOSPC), "{error}");
                return;
            }
        }
    }
    panic!("bounded fault volume did not return ENOSPC");
}

pub fn free_volume(path: &std::path::Path) {
    // Explicitly truncate and flush: HFS+ can defer reclaiming an unlinked file's extents.
    let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_len(0).unwrap();
    file.sync_all().unwrap();
    drop(file);
    std::fs::remove_file(path).unwrap();
}
