use nexa::{
    digest::Digest,
    memory::{Store, seal_source},
    value::Identifier,
};
use std::{fs, path::Path};
use tempfile::TempDir;

fn write_memory(root: &Path, name: &str, module: &str, version: &str, entry: &str, source: &str) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("memory.json"),
        format!(
            r#"{{
  "schema": "nexa.memory-source/0.3",
  "name": "{name}",
  "version": "{version}",
  "module": "{module}",
  "sources": ["src/main.nexa"],
  "entry": "{entry}",
  "kernel": "std.slice0@0.3.0",
  "dependencies": []
}}"#
        ),
    )
    .unwrap();
    fs::write(root.join("src/main.nexa"), source).unwrap();
}

fn greet_source(prefix: &str) -> String {
    format!(
        "nexa 0.3\nmodule local.test.greeter\n\
         export fn greet(name is String) -> String {{\n\
           return \"{prefix}\" + name\n\
         }}\n"
    )
}

fn temp_store() -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    let store = Store::open(temp.path().join("store")).unwrap();
    (temp, store)
}

fn test_memory(temp: &TempDir, suffix: &str, source: &str) -> std::path::PathBuf {
    let path = temp.path().join(suffix);
    write_memory(
        &path,
        "local.test.greeter",
        "local.test.greeter",
        "1.0.0",
        "greet",
        source,
    );
    path
}

#[test]
fn seals_and_reloads_a_source_memory_after_reopening_the_store() {
    let (temp, store) = temp_store();
    let source = greet_source("Hello, ");
    let directory = test_memory(&temp, "memory", &source);
    let digest = seal_source(directory, &store).unwrap();

    let record = store.load_memory(&digest).unwrap();
    assert_eq!(record.metadata.name, "local.test.greeter");
    assert_eq!(record.exports[0].export_name.as_str(), "greet");
    assert_eq!(record.kernel, *store.kernel_digest());
    assert_eq!(store.resolve("local.test.greeter@1.0.0").unwrap(), digest);

    let reopened = Store::open(temp.path().join("store")).unwrap();
    let loaded = reopened.load_memory(&digest).unwrap();
    assert_eq!(loaded.exports[0].result_type, record.exports[0].result_type);
    let checked = reopened.load_source_module(&digest).unwrap();
    assert_eq!(checked.module_name(), "local.test.greeter");
}

#[test]
fn source_manifest_accepts_the_exact_installed_kernel_digest() {
    let (temp, store) = temp_store();
    let directory = test_memory(&temp, "memory", &greet_source("Hello, "));
    let manifest_path = directory.join("memory.json");
    let manifest = fs::read_to_string(&manifest_path)
        .unwrap()
        .replace("std.slice0@0.3.0", store.kernel_digest().as_str());
    fs::write(manifest_path, manifest).unwrap();

    let digest = seal_source(directory, &store).unwrap();
    let record = store.load_memory(&digest).unwrap();
    assert_eq!(record.kernel, *store.kernel_digest());
}

#[test]
fn every_blob_load_checks_its_digest() {
    let (temp, store) = temp_store();
    let directory = test_memory(&temp, "memory", &greet_source("Hello, "));
    let digest = seal_source(directory, &store).unwrap();
    let record = store.load_memory(&digest).unwrap();
    let blob = &record.source_map[0].blob;
    fs::write(store.object_path(blob), b"tampered source bytes").unwrap();

    let error = store.load_memory(&digest).unwrap_err();
    assert_eq!(error.diagnostic.code, "E_INTEGRITY");
}

#[test]
fn alias_moves_do_not_change_existing_locks_but_new_resolution_does() {
    let (temp, store) = temp_store();
    let first_dir = test_memory(&temp, "first", &greet_source("Hello, "));
    let first = seal_source(first_dir, &store).unwrap();
    let selector = "local.test.greeter@1.0.0".to_owned();
    let export = Identifier::new("greet").unwrap();
    let lock = store
        .resolve_requests(&[(selector.clone(), export.clone())])
        .unwrap();
    assert_eq!(
        store.resolve_locked(&lock, &selector, &export).unwrap(),
        first
    );

    let second_dir = test_memory(&temp, "second", &greet_source("Hi, "));
    let second = seal_source(second_dir, &store).unwrap();
    assert_ne!(first, second);
    assert_eq!(store.resolve(&selector).unwrap(), second);
    assert_eq!(
        store.resolve_locked(&lock, &selector, &export).unwrap(),
        first
    );

    let next_lock = store
        .resolve_requests(&[(selector.clone(), export.clone())])
        .unwrap();
    assert_eq!(
        store
            .resolve_locked(&next_lock, &selector, &export)
            .unwrap(),
        second
    );
}

#[test]
fn manifest_module_must_match_checked_source() {
    let (temp, store) = temp_store();
    let directory = temp.path().join("memory");
    write_memory(
        &directory,
        "local.test.greeter",
        "local.test.wrong",
        "1.0.0",
        "greet",
        &greet_source("Hello, "),
    );

    let error = seal_source(directory, &store).unwrap_err();
    assert_eq!(error.diagnostic.code, "E_MODULE");
}

#[test]
fn source_bundle_rejects_multiple_files_instead_of_ignoring_one() {
    let (temp, store) = temp_store();
    let directory = test_memory(&temp, "memory", &greet_source("Hello, "));
    let manifest_path = directory.join("memory.json");
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    let manifest = manifest.replace(
        "\"sources\": [\"src/main.nexa\"]",
        "\"sources\": [\"src/main.nexa\", \"src/helper.nexa\"]",
    );
    fs::write(manifest_path, manifest).unwrap();

    let error = seal_source(directory, &store).unwrap_err();
    assert_eq!(error.diagnostic.code, "E_SOURCE_BUNDLE");
}

#[cfg(unix)]
#[test]
fn source_symlink_cannot_escape_memory_directory() {
    use std::os::unix::fs::symlink;

    let (temp, store) = temp_store();
    let directory = temp.path().join("memory");
    fs::create_dir_all(directory.join("src")).unwrap();
    let outside = temp.path().join("outside.nexa");
    fs::write(&outside, greet_source("Hello, ")).unwrap();
    symlink(&outside, directory.join("src/main.nexa")).unwrap();
    fs::write(
        directory.join("memory.json"),
        r#"{
  "schema": "nexa.memory-source/0.3",
  "name": "local.test.greeter",
  "version": "1.0.0",
  "module": "local.test.greeter",
  "sources": ["src/main.nexa"],
  "entry": "greet",
  "kernel": "std.slice0@0.3.0",
  "dependencies": []
}"#,
    )
    .unwrap();

    let error = seal_source(directory, &store).unwrap_err();
    assert_eq!(error.diagnostic.code, "E_PATH");
}

#[test]
fn source_effects_cannot_be_hidden_by_an_empty_declaration() {
    let (temp, store) = temp_store();
    let source = "nexa 0.3\nmodule local.test.writer\n\
        export action write(message is String) -> Unit effects {} {\n\
          return std.console.write_line(message)\n\
        }\n";
    let directory = temp.path().join("memory");
    write_memory(
        &directory,
        "local.test.writer",
        "local.test.writer",
        "1.0.0",
        "write",
        source,
    );

    let error = seal_source(directory, &store).unwrap_err();
    assert_eq!(error.diagnostic.code, "E_EFFECT_SET");
}

#[test]
fn sealing_a_valid_action_does_not_execute_it() {
    let (temp, store) = temp_store();
    let source = "nexa 0.3\nmodule local.test.writer\n\
        export action write(message is String) -> Unit effects {io.stdout} {\n\
          return std.console.write_line(message)\n\
        }\n";
    let directory = temp.path().join("memory");
    write_memory(
        &directory,
        "local.test.writer",
        "local.test.writer",
        "1.0.0",
        "write",
        source,
    );

    let digest = seal_source(directory, &store).unwrap();
    let record = store.load_memory(&digest).unwrap();
    assert_eq!(record.exports[0].effects, vec!["io.stdout".to_owned()]);
}

#[test]
fn raw_digest_addresses_are_validated_on_load() {
    let (_temp, store) = temp_store();
    let digest = store.put_raw(b"raw source").unwrap();
    assert_eq!(store.get_raw(&digest).unwrap(), b"raw source");
    let parsed: Digest = digest.as_str().parse().unwrap();
    assert_eq!(parsed, digest);
}
