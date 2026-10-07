//! Readiness contract of the background mount constructors.
//!
//! `mount_background_configured` used to sleep for a fixed 100 ms and return
//! `Ok` unconditionally: the worker's validation failure was only logged, and
//! a mount that had not appeared yet looked the same as a serving one. Callers
//! then read the plain directory behind the mountpoint — and in the in-place
//! layout that is the raw source tree, indistinguishable from a working mount
//! until something writes through it.

use std::path::Path;
use std::sync::Arc;

use parking_lot::RwLock;
use skillfs_core::{ParseConfig, SharedSkillStore, store::SkillStore};
use skillfs_fuse::{MountConfig, MountOptions, mount_background_configured};

mod common;

use common::list_dir_names;

fn load_store(source: &Path) -> SharedSkillStore {
    let mut store = SkillStore::new();
    store.load_from_directory(source, &ParseConfig::default());
    Arc::new(RwLock::new(store))
}

fn write_skill(source: &Path, name: &str) {
    let dir = source.join(name);
    std::fs::create_dir_all(&dir).expect("skill dir");
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: demo\n---\nbody\n"),
    )
    .expect("SKILL.md");
}

/// A mountpoint that cannot host a mount must surface the worker's error
/// instead of returning `Ok` for a mount that never happened.
#[test]
fn background_mount_reports_an_invalid_mountpoint() {
    let source = tempfile::tempdir().expect("source tempdir");
    write_skill(source.path(), "demo");

    // A regular file is not a mountpoint; the blocking entry point rejects it
    // with `InvalidMountPoint` before touching the kernel.
    let holder = tempfile::tempdir().expect("holder tempdir");
    let not_a_dir = holder.path().join("mnt");
    std::fs::write(&not_a_dir, "not a directory\n").expect("regular file");

    let err = match mount_background_configured(
        &not_a_dir,
        source.path(),
        load_store(source.path()),
        MountOptions::default(),
        false,
        MountConfig::default(),
    ) {
        Ok(_) => panic!("a file mountpoint must not report a successful background mount"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("mount point"),
        "the worker's validation error must reach the caller, got: {err}"
    );
}

/// When the call returns, the mount is serving: the very next read sees the
/// FUSE view, not an empty directory that is still waiting for the daemon.
#[test]
fn background_mount_is_serving_when_it_returns() {
    if !common::fuse_available() {
        eprintln!("SKIP: FUSE not available (no /dev/fuse or fusermount3)");
        return;
    }

    let source = tempfile::tempdir().expect("source tempdir");
    write_skill(source.path(), "demo");
    let mountpoint = tempfile::tempdir().expect("mount tempdir");

    let handle = mount_background_configured(
        mountpoint.path(),
        source.path(),
        load_store(source.path()),
        MountOptions::default(),
        false,
        MountConfig::default(),
    )
    .expect("background mount");

    // No sleep: the constructor's contract is that the mount is live.
    let root = list_dir_names(mountpoint.path());
    assert!(
        root.contains(&"skills".to_string()),
        "the FUSE view must be serving as soon as the mount returns, got: {root:?}"
    );
    let readme = std::fs::read_to_string(mountpoint.path().join("skills/demo/SKILL.md"))
        .expect("read a skill manifest through the mount");
    assert!(
        readme.contains("body"),
        "the mount must serve the manifest, got: {readme:?}"
    );

    handle.unmount().expect("unmount");
}
