#[test]
fn linked_sqlite_includes_the_wal_reset_corruption_fix() {
    // APR supports the patched 3.51 series and later. This also catches builds
    // that unexpectedly select an older system library instead of the bundle.
    // https://www.sqlite.org/wal.html#walreset
    assert!(
        rusqlite::version_number() >= 3_051_003,
        "APR requires SQLite 3.51.3 or newer; linked {}",
        rusqlite::version()
    );
}
