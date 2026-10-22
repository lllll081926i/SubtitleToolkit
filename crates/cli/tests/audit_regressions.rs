//! 2026-10-22 审计回归：文本路径不依赖 libass，可在 Windows CI 上直接运行。
use std::path::{Path, PathBuf};
use std::process::Command;

const ASS: &str = "[Script Info]\nScriptType: v4.00+\nPlayResX: 1280\nPlayResY: 720\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, Outline, Shadow, MarginV\nStyle: Default,Arial,48,&H00FFFFFF,3,2,50\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,demo\n";

fn setup(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("stk-audit-{}-{}", label, std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}
fn execute(dir: &Path, spec: serde_json::Value) -> serde_json::Value {
    let spec_path = dir.join("audit-spec.json");
    std::fs::write(&spec_path, serde_json::to_vec(&spec).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_subtitletool-cli"))
        .args(["batch", "--spec", spec_path.to_str().unwrap(), "--json"])
        .output().expect("CLI should start");
    assert!(output.status.success(), "stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr), String::from_utf8_lossy(&output.stdout));
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn exact_pairs_do_not_expand_to_cross_product() {
    let dir = setup("only-pairs");
    std::fs::write(dir.join("a.ass"), ASS).unwrap();
    std::fs::write(dir.join("b.ass"), ASS).unwrap();
    let data = execute(&dir, serde_json::json!({
        "id":"exact", "roots":["a.ass","b.ass"], "video_association":false,
        "profiles":[
            {"name":"sub","format":"srt","canvas":"1280x720"},
            {"name":"native","format":"ass","canvas":"1280x720"}],
        "only_pairs":[
            {"input":"a.ass","profile":"sub"},
            {"input":"b.ass","profile":"native"}],
        "output":{"dir":"out","template":"{stem}.{profile}.{ext}"}
    }));
    assert_eq!(data["data"]["report"]["details"].as_array().unwrap().len(), 2);
    assert!(dir.join("out/a.sub.srt").exists());
    assert!(dir.join("out/b.native.ass").exists());
    assert!(!dir.join("out/a.native.ass").exists());
    assert!(!dir.join("out/b.sub.srt").exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn offset_and_hdr_are_written_to_the_delivered_ass() {
    let dir = setup("profile-transform");
    std::fs::write(dir.join("a.ass"), ASS).unwrap();
    execute(&dir, serde_json::json!({
        "id":"transform", "roots":["a.ass"], "video_association":false,
        "profiles":[{"name":"fixed","format":"ass","canvas":"original",
            "offset_ms":1000,"hdr_percent":50}],
        "output":{"dir":"out","template":"{stem}.{profile}.{ext}"}
    }));
    let result = std::fs::read_to_string(dir.join("out/a.fixed.ass")).unwrap();
    assert!(result.contains("PlayResX: 1280"));
    assert!(result.contains("PlayResY: 720"));
    assert!(result.contains("0:00:02.00,0:00:03.00"), "offset was not applied: {result}");
    assert!(!result.contains("&H00FFFFFF"), "HDR was not applied: {result}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn equal_stems_from_different_directories_do_not_share_names() {
    let dir = setup("same-stem");
    std::fs::create_dir_all(dir.join("season1")).unwrap();
    std::fs::create_dir_all(dir.join("season2")).unwrap();
    std::fs::write(dir.join("season1/episode.ass"), ASS).unwrap();
    std::fs::write(dir.join("season2/episode.ass"), ASS.replace("demo", "different")).unwrap();
    execute(&dir, serde_json::json!({
        "id":"names","roots":["season1/episode.ass","season2/episode.ass"],
        "video_association":false,
        "profiles":[{"name":"srt","format":"srt"}],
        "output":{"dir":"out","template":"{stem}.{profile}.{ext}"}
    }));
    let names: Vec<_> = std::fs::read_dir(dir.join("out")).unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".srt"))
        .collect();
    assert_eq!(names.len(), 2, "same stem must produce independent files: {names:?}");
    assert_ne!(names[0], names[1]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn broken_jobstate_is_quarantined_before_recovery() {
    let dir = setup("corrupt-state");
    std::fs::write(dir.join("a.ass"), ASS).unwrap();
    let spec = serde_json::json!({
        "id":"corrupt","roots":["a.ass"],"video_association":false,
        "profiles":[{"name":"plain","format":"srt"}],
        "output":{"dir":"out","template":"{stem}.{profile}.{ext}"}
    });
    execute(&dir, spec.clone());
    std::fs::write(dir.join("out/.jobstate.json"), b"{broken").unwrap();
    execute(&dir, spec);
    assert!(dir.join("out/.jobstate.jobstate.corrupt").exists());
    std::fs::remove_dir_all(&dir).ok();
}
