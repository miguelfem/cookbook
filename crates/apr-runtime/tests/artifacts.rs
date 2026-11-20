use apr_runtime::artifacts::ArtifactStore;

#[test]
fn imported_files_are_immutable_snapshots_with_known_hashes() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    std::fs::write(&source, b"abc").unwrap();
    let state = directory.path().join("state");
    let store = ArtifactStore::open(&state).unwrap();
    let reference = store.import(&source, "text/plain", 1024).unwrap();
    assert_eq!(
        reference.sha256,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        reference.artifact_id,
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(reference.bytes, 3);
    std::fs::write(&source, b"changed source").unwrap();
    drop(store);
    let reopened = ArtifactStore::open(&state).unwrap();
    assert_eq!(reopened.read(&reference, 1024).unwrap(), b"abc");
    assert_eq!(reopened.put(b"abc", "text/plain", 1024).unwrap(), reference);
}

#[test]
fn bundles_have_stable_order_metadata_and_content_across_restarts() {
    use std::io::Read;
    let directory = tempfile::tempdir().unwrap();
    let store = ArtifactStore::open(directory.path()).unwrap();
    let first = store.put(b"abc", "text/plain", 4096).unwrap();
    let second = store.put(b"", "application/octet-stream", 4096).unwrap();
    let report = serde_json::json!({"approved": true});
    let bundle = store
        .bundle(&[first.clone(), second.clone()], Some(&report), 4096)
        .unwrap();
    drop(store);
    let store = ArtifactStore::open(directory.path()).unwrap();
    let repeated = store
        .bundle(
            &[second.clone(), first.clone(), first.clone()],
            Some(&report),
            4096,
        )
        .unwrap();
    assert_eq!(bundle, repeated);
    let bytes = store.read(&bundle, 4096).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    assert_eq!(zip.len(), 3);
    let mut manifest = String::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_string(&mut manifest)
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(manifest["format"], "apr-artifact-bundle-v1");
    assert_eq!(manifest["report"], report);
    assert_eq!(manifest["artifacts"][0]["sha256"], first.sha256);
    assert_eq!(manifest["artifacts"][1]["sha256"], second.sha256);
    let mut payload = Vec::new();
    let mut entry = zip.by_name(&format!("objects/{}", first.sha256)).unwrap();
    assert_eq!(entry.last_modified().unwrap().year(), 1980);
    assert_eq!(entry.unix_mode().unwrap() & 0o777, 0o644);
    entry.read_to_end(&mut payload).unwrap();
    assert_eq!(payload, b"abc");
}

#[test]
fn corrupted_or_missing_objects_are_never_reused_or_silently_repaired() {
    use apr_runtime::artifacts::ArtifactError;
    let directory = tempfile::tempdir().unwrap();
    let store = ArtifactStore::open(directory.path()).unwrap();
    let reference = store.put(b"abc", "text/plain", 1024).unwrap();
    let object = directory
        .path()
        .join("objects/sha256")
        .join(&reference.sha256);
    std::fs::write(&object, b"xyz").unwrap();
    assert!(matches!(
        store.read(&reference, 1024),
        Err(ArtifactError::Corrupt { .. })
    ));
    assert!(matches!(
        store.put(b"abc", "text/plain", 1024),
        Err(ArtifactError::Corrupt { .. })
    ));
    assert_eq!(std::fs::read(&object).unwrap(), b"xyz");
    std::fs::remove_file(&object).unwrap();
    assert!(store.verify(&reference, 1024).is_err());
}

#[test]
fn size_limits_cover_imports_reads_and_complete_zip_overhead() {
    use apr_runtime::artifacts::ArtifactError;
    let directory = tempfile::tempdir().unwrap();
    let store = ArtifactStore::open(directory.path()).unwrap();
    let reference = store.put(b"abc", "text/plain", 3).unwrap();
    assert!(matches!(
        store.put(b"abc", "text/plain", 2),
        Err(ArtifactError::TooLarge { limit: 2 })
    ));
    assert!(matches!(
        store.read(&reference, 2),
        Err(ArtifactError::TooLarge { limit: 2 })
    ));
    let source = directory.path().join("oversized");
    std::fs::write(&source, b"abcd").unwrap();
    assert!(matches!(
        store.import(&source, "text/plain", 3),
        Err(ArtifactError::TooLarge { limit: 3 })
    ));
    assert!(matches!(
        store.bundle(&[reference], None, 100),
        Err(ArtifactError::TooLarge { limit: 100 })
    ));
    let leftovers = std::fs::read_dir(directory.path().join("objects/sha256"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('.'))
        .collect::<Vec<_>>();
    assert!(leftovers.is_empty());
}

#[test]
fn malformed_references_and_conflicting_metadata_cannot_select_objects() {
    use apr_runtime::artifacts::ArtifactError;
    let directory = tempfile::tempdir().unwrap();
    let store = ArtifactStore::open(directory.path()).unwrap();
    let reference = store.put(b"abc", "text/plain", 1024).unwrap();
    let mut forged = reference.clone();
    forged.sha256 = "../../outside".into();
    forged.artifact_id = format!("sha256:{}", forged.sha256);
    assert!(matches!(
        store.read(&forged, 1024),
        Err(ArtifactError::InvalidReference(_))
    ));
    let mut wrong_size = reference.clone();
    wrong_size.bytes = 2;
    assert!(matches!(
        store.verify(&wrong_size, 1024),
        Err(ArtifactError::Corrupt { .. })
    ));
    assert!(matches!(
        store.put(b"abc", "text/plain\r\nInjected: yes", 1024),
        Err(ArtifactError::InvalidMediaType(_))
    ));
    let mut other_type = reference.clone();
    other_type.media_type = "application/json".into();
    assert!(matches!(
        store.bundle(&[reference, other_type], None, 4096),
        Err(ArtifactError::InvalidReference(_))
    ));
}

#[test]
fn symlinks_cannot_redirect_imports_or_object_publication() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let outside = directory.path().join("outside");
    std::fs::write(&outside, b"outside").unwrap();
    let source_link = directory.path().join("source-link");
    symlink(&outside, &source_link).unwrap();
    let state = directory.path().join("state");
    let store = ArtifactStore::open(&state).unwrap();
    assert!(store.import(&source_link, "text/plain", 1024).is_err());
    let object = state
        .join("objects/sha256/ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    symlink(&outside, &object).unwrap();
    assert!(store.put(b"abc", "text/plain", 1024).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside");
}

#[test]
fn open_store_keeps_objects_pinned_when_a_directory_is_replaced_with_a_symlink() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let outside = directory.path().join("outside");
    let moved = directory.path().join("original-objects");
    std::fs::create_dir(&outside).unwrap();
    let store = ArtifactStore::open(&state).unwrap();
    let reference = store.put(b"abc", "text/plain", 1024).unwrap();
    std::fs::rename(state.join("objects/sha256"), &moved).unwrap();
    symlink(&outside, state.join("objects/sha256")).unwrap();
    assert_eq!(store.read(&reference, 1024).unwrap(), b"abc");
    let additional = store.put(b"another object", "text/plain", 1024).unwrap();
    assert_eq!(store.read(&additional, 1024).unwrap(), b"another object");
    assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
    assert!(ArtifactStore::open(&state).is_err());
}

#[test]
fn concurrent_writers_publish_one_complete_object_and_readers_recheck_it() {
    use std::sync::{Arc, Barrier};
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(ArtifactStore::open(directory.path()).unwrap());
    let barrier = Arc::new(Barrier::new(8));
    let payload = Arc::new(vec![b'x'; 256 * 1024]);
    let jobs = (0..8)
        .map(|_| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            let payload = Arc::clone(&payload);
            std::thread::spawn(move || {
                barrier.wait();
                let reference = store
                    .put(&payload, "application/octet-stream", 512 * 1024)
                    .unwrap();
                for _ in 0..4 {
                    assert_eq!(store.read(&reference, 512 * 1024).unwrap(), *payload);
                }
                reference
            })
        })
        .collect::<Vec<_>>();
    let references = jobs
        .into_iter()
        .map(|job| job.join().unwrap())
        .collect::<Vec<_>>();
    assert!(
        references
            .iter()
            .all(|reference| reference == &references[0])
    );
}

#[test]
fn bundle_limit_accepts_exact_size_and_rejects_one_byte_less() {
    use apr_runtime::artifacts::ArtifactError;
    let directory = tempfile::tempdir().unwrap();
    let store = ArtifactStore::open(directory.path()).unwrap();
    let reference = store.put(b"abc", "text/plain", 4096).unwrap();
    let bundle = store
        .bundle(std::slice::from_ref(&reference), None, 4096)
        .unwrap();
    assert_eq!(
        store
            .bundle(std::slice::from_ref(&reference), None, bundle.bytes)
            .unwrap(),
        bundle
    );
    assert!(matches!(
        store.bundle(&[reference], None, bundle.bytes - 1),
        Err(ArtifactError::TooLarge { .. })
    ));
}
