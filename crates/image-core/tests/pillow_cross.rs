//! 第三方交叉验证（24 §QA / 25 §AT：不能只用自家实现自证）。
//!
//! 方向一：我们的 writer → Pillow reader。
//! 方向二：Pillow writer → 我们的 reader（覆盖 color type 0/2/3/4/6 与子字节位深）。
//!
//! 依赖 `python` + Pillow；不存在时**明确 SKIP 并打印原因**（不假装通过），
//! 需要强制时用环境变量 `STK_REQUIRE_PILLOW=1`（CI 等价脚本会设置它）。

use image_core::{encode, header, Level};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tools")
        .join("png_pillow_check.py")
}

fn run(python: &str, dir: &Path, verb: &str) -> (bool, String) {
    let out = Command::new(python)
        .arg(script())
        .arg(verb)
        .arg(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    match out {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).to_string();
            text.push_str(&String::from_utf8_lossy(&o.stderr));
            (o.status.success(), text)
        }
        Err(e) => (false, format!("无法启动 {python}：{e}")),
    }
}

fn tmpdir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("stk-pillow-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn checker() -> Option<String> {
    if let Ok(p) = std::env::var("STK_PYTHON") {
        return Some(p);
    }
    for cand in ["python", "python3", "py"] {
        if Command::new(cand)
            .arg("-c")
            .arg("import PIL")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return Some(cand.to_string());
        }
    }
    None
}

fn content(kind: u8, w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            match kind {
                0 => v.extend_from_slice(&[0, 0, 0, 0]),
                1 => v.extend_from_slice(&[((x * 3) as u8), 40, 200, 255]),
                2 => {
                    let edge = x < 3 || y < 3;
                    v.extend_from_slice(if edge {
                        &[17, 17, 17, 128]
                    } else {
                        &[250, 250, 250, 255]
                    });
                }
                _ => {
                    let s = (x * y) % 256;
                    v.extend_from_slice(&[s as u8, 255 - s as u8, (s * 7) as u8, 255]);
                }
            }
        }
    }
    v
}

#[test]
fn pillow_reads_our_pngs_and_we_read_pillow_pngs() {
    let dir = tmpdir("x");
    let Some(python) = checker() else {
        if std::env::var("STK_REQUIRE_PILLOW").is_ok() {
            panic!("STK_REQUIRE_PILLOW=1 但找不到带 Pillow 的 python");
        }
        eprintln!("SKIP pillow 交叉验证：本机没有 python + Pillow（本测试在 CI 等价脚本中启用）");
        return;
    };

    // 方向一：我们写，Pillow 读
    let mut written = 0usize;
    for (i, (kind, w, h, level)) in [
        (0u8, 64u32, 4u32, Level::Max),
        (1, 37, 12, Level::Balanced),
        (2, 16, 16, Level::Fast),
        (3, 5, 9, Level::Balanced),
    ]
    .iter()
    .enumerate()
    {
        let rgba = content(*kind, *w, *h);
        let png = encode(&rgba, *w, *h, *level).unwrap();
        std::fs::write(dir.join(format!("ours_{i:02}.png")), &png).unwrap();
        std::fs::write(dir.join(format!("ours_{i:02}.rgba")), &rgba).unwrap();
        written += 1;
    }
    let (ok, log) = run(&python, &dir, "verify");
    assert!(ok, "Pillow 复核我们的 PNG 失败：{log}");
    assert!(
        log.contains(&format!("VERIFY OK {written}")),
        "Pillow 没有核对全部文件：{log}"
    );

    // 方向二：Pillow 写，我们读
    let (ok, log) = run(&python, &dir, "emit");
    assert!(ok, "Pillow 生成参考文件失败：{log}");
    let mut refs = 0usize;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        if !name.starts_with("ref_") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let hdr = header(&bytes).unwrap();
        let expected =
            std::fs::read(dir.join(format!("{name}.rgba"))).expect("缺少 Pillow 期望像素文件");
        let img = image_core::decode(&bytes)
            .unwrap_or_else(|e| panic!("读不回 Pillow 写的 {}: {e}", path.display()));
        assert_eq!(
            (img.width, img.height),
            (hdr.width, hdr.height),
            "{name} 尺寸不符"
        );
        assert_eq!(
            img.rgba, expected,
            "{name}（color type {}）像素与 Pillow 不符",
            hdr.color_type
        );
        refs += 1;
    }
    assert!(refs >= 6, "Pillow 只产出了 {refs} 个参考文件，覆盖不足");
    let _ = std::fs::remove_dir_all(&dir);
}
