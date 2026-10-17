//! 渲染缓存 key 的"字体环境"那一维（17 §7 的 `render cache key 包含所有实际 font bytes
//! hash + resolver policy + renderer version`）。renderer 版本已经由 provider 哈希进去了，
//! 这里补的是前两项。
//!
//! 一次进程内只建一次库：批量跑一百个输入时，字体库的内容是同一份，
//! 变的只是"这个脚本请求了哪些 face"。建库要扫真字体目录（本机 ~10 秒），
//! 每个输入再扫一次是白付一百倍代价。
//!
//! 只有真的会渲染的命令才付这个钱：纯文本转换（ass→srt）不建库，也不该被字体环境绑住。
//!
//! 缓存的账（AI5）：整座库重扫 release 档 ~2.7 秒、debug 档 ~45 秒。命中时不重读没用的文件，
//! 但**被用到的每个 face 都会在本次被重读并复核内容哈希**（`evaluate_or_rescan` 与
//! `FontDb::missing_glyphs`），所以对错不靠指纹、只靠使用点。指纹骗人时（同长度同 mtime
//! 的原地覆盖）最坏是多付一次全量重扫；`--no-font-cache` 是"根本不认这份缓存"的那条路。

use std::path::{Path, PathBuf};
use std::time::Instant;

use core_model::error::Result;
use font_service::attachments::Attachment;
use font_service::db::{CacheMode, CacheState, FontDb};
use font_service::report::FontMatch;
use font_service::usage::scan_text;
use font_service::{env::FontEnv, evaluate_or_rescan, identify, POLICY_VERSION};

/// 默认字体来源：系统目录 + 用户注册字体目录 + 两张注册表键（与 `font scan` 同一套）。
pub(crate) fn default_sources() -> Vec<PathBuf> {
    crate::font::default_font_dirs()
}

/// 字体缓存落在哪：`doctor` 要能回答"东西写到哪去了"。
/// 返回 `(路径, 来源说明)`；拿不到路径时第一个元素是 `None`。
pub(crate) fn cache_path_report() -> serde_json::Value {
    let explicit = std::env::var("STK_FONTCACHE")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let path = font_service::cache::cache_path();
    serde_json::json!({
        "path": path.as_ref().map(|p| p.display().to_string()),
        "source": if explicit.is_some() {
            "STK_FONTCACHE"
        } else {
            "data-dir"
        },
        "env_override": "STK_FONTCACHE",
        "disable_switch": "--no-font-cache",
    })
}

/// 一次调用要用的字体来源：目录 + 注册表指到的文件。
/// 建库与"缓存撒谎后重来"都要同一份来源，所以它们必须成对存下来。
struct Sources {
    dirs: Vec<PathBuf>,
    registered: Vec<PathBuf>,
}

/// 建一次库、对多个输入分别出身份。
pub(crate) struct Scanner {
    db: Option<FontDb>,
    sources: Option<Sources>,
    mode: CacheMode,
    build_ms: u128,
}

impl Scanner {
    /// `--no-font-cache` 走 `CacheMode::Off`：完全不读也不写磁盘缓存。
    pub(crate) fn with_cache(mode: CacheMode) -> Scanner {
        Scanner {
            db: None,
            sources: None,
            mode,
            build_ms: 0,
        }
    }

    /// 懒建库。第一次调用会扫目录与注册表，之后所有输入复用同一份。
    pub(crate) fn env_for(&mut self, ass_text: &str) -> Result<(FontEnv, u128)> {
        let (env, ms, _, _) = self.resolve(ass_text)?;
        Ok((env, ms))
    }

    /// 17 §7：这份脚本要交给渲染器的字体清单。
    ///
    /// 只列**真的解析到的** face（Missing/Ambiguous 没有 face 可交）。注册名用 face 自己表里的
    /// 家族名：实测 libass 是按字体内部名匹配内存字体的，按请求名注册不会命中
    /// （`(StkProbeFamily, 400, 0) -> ArialMT` 那次就是栽在这上面）。
    pub(crate) fn inject_plan(&mut self, ass_text: &str) -> Result<Vec<crate::front::InjectFace>> {
        let (_env, _ms, matches, embedded) = self.resolve(ass_text)?;
        // 脚本自带的 `[Fonts]` 排在台账前面：这只是**列出顺序**，不是优先级。
        // "先注册的内存字体就赢"是我们没取证过的说法，实测已否定它——同一家族有两份
        // 字节时，两种注册顺序画出同一份像素（AK8，`registration_order_is_not_a_tiebreaker_for_the_same_family`）。
        // 因此这里不承诺"作者声明的字体一定覆盖系统里同名的字体"；能承诺的是：
        // 赢家是交出去的那几批字节之一，且这批字节全部进了渲染身份摘要。
        let mut out: Vec<crate::front::InjectFace> = Vec::new();
        let mut seen_sha: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for a in embedded {
            let Some(family) = a.family.clone() else {
                continue;
            };
            // 同一批字节交两次没有意义：libass 按内部家族名匹配，第二份不会改变结果，
            // 只会让台账看上去比实际做的事多（去重按内容哈希，不按名字猜）
            if !seen_sha.insert(a.sha256.clone()) {
                continue;
            }
            out.push(crate::front::InjectFace {
                family,
                file: std::path::PathBuf::from(a.declared_name.clone()),
                expect_sha: a.sha256.clone(),
                data: Some(a.data),
                origin: crate::front::InjectOrigin::Embedded,
            });
        }
        let mut seen_face: std::collections::BTreeSet<(String, String)> =
            std::collections::BTreeSet::new();
        for m in matches {
            let Some(face) = m.face else { continue };
            if !seen_face.insert((face.family.clone(), face.path.clone())) {
                continue; // 同一个 face 被多条样式引用，只交一次
            }
            if seen_sha.contains(&face.source_sha256) {
                continue; // 脚本内嵌的就是这个文件的内容，不必再交一遍
            }
            out.push(crate::front::InjectFace {
                family: face.family.clone(),
                file: std::path::PathBuf::from(&face.path),
                expect_sha: face.source_sha256.clone(),
                data: None,
                origin: crate::front::InjectOrigin::Library,
            });
        }
        Ok(out)
    }

    pub(crate) fn resolve(
        &mut self,
        ass_text: &str,
    ) -> Result<(FontEnv, u128, Vec<FontMatch>, Vec<Attachment>)> {
        let started = Instant::now();
        if self.db.is_none() {
            let dirs = default_sources();
            if !dirs.is_empty() {
                let reg = font_service::registry::enumerate();
                let refs: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
                // 走带磁盘缓存的入口：整座库重扫一次在 release 档 ~2.7 秒、debug 档 ~45 秒，
                // 而字体在两次调用之间通常没变（AI5）。命中时索引不重读，但**被用到的每个
                // face 的字节都会在本次重读并复核哈希** —— 见 `evaluate_or_rescan`。
                let db = FontDb::scan_cached(&refs, &reg.font_files(), self.mode);
                self.build_ms = started.elapsed().as_millis();
                self.sources = Some(Sources {
                    dirs,
                    registered: reg.font_files(),
                });
                self.db = Some(db);
            } else {
                // 渲染不能因为"这台机器读不到字体目录"就停摆：给一个明确标成**不完整**的
                // 身份，让 key 与真实环境不同，并在报告里说清缺了什么。
                // （`font scan` 不一样：它就是来回答"这台机器有什么字体"的，缺来源必须报错。）
                self.db = Some(FontDb::scan(&[]));
            }
        }
        if self.db.as_ref().expect("上面刚建好").roots.is_empty() {
            return Ok((incomplete_env(), self.build_ms, Vec::new(), Vec::new()));
        }
        let requests = scan_text(ass_text)?;
        let db = self.db.as_mut().expect("上面刚建好");
        let matches: Vec<FontMatch> = match self.sources.as_ref() {
            // 有来源：解析，缓存与文件现状不符就重建一次再解析（同一进程内只重建一次，
            // 因为重建后的库 `cache != Hit`，同一进程里不会再触发第二次）。
            Some(s) => {
                let refs: Vec<&Path> = s.dirs.iter().map(|p| p.as_path()).collect();
                evaluate_or_rescan(db, &refs, &s.registered, &requests)?
            }
            None => font_service::evaluate(db, &requests)?,
        };
        let attachments = font_service::parse_font_attachments(ass_text)?;
        Ok((
            identify(db, &matches, &attachments),
            self.build_ms,
            matches,
            attachments,
        ))
    }
}

/// 读不到任何字体来源时的身份：必须是**另一个**值，不能与"库里有来源但什么都没有"
/// 撞成同一个摘要——那两者对交付的含义完全不同。
fn incomplete_env() -> FontEnv {
    FontEnv {
        policy: format!("{POLICY_VERSION}|no-font-sources"),
        entries: vec!["nosources".to_string()],
        digest: format!("{POLICY_VERSION}|nosources"),
        faces_used: 0,
        faces_missing: 0,
        attachments: 0,
        cache: CacheState::Scanned,
    }
}

/// 注入这块的报告。"没开"与"开了但一条都没注入"是两件事，必须能分开读：
/// `enabled=false` 时台账为空是正常的；`enabled=true` 而台账为空，说明这份脚本
/// 没解析到任何 face —— 那不能算"注入成功"。
pub(crate) fn injection_json(
    injected: &[render_libass::InjectedFont],
    plan: &[crate::front::InjectFace],
    enabled: bool,
) -> serde_json::Value {
    serde_json::json!({
        "enabled": enabled,
        "requested": plan.len(),
        "embedded": plan.iter().filter(|f| f.origin == crate::front::InjectOrigin::Embedded).count(),
        "from_library": plan.iter().filter(|f| f.origin == crate::front::InjectOrigin::Library).count(),
        "injected": injected.iter().map(|f| serde_json::json!({
            "name": f.name, "bytes": f.bytes, "sha256": f.sha256,
        })).collect::<Vec<_>>(),
        "meaning": if enabled {
            "这些字节是本次直接交给 libass 的内存字体；渲染缓存 key 里的 face 哈希就是它们的哈希（17 §7）"
        } else {
            "未开启注入：libass 按系统字体选择渲染，key 里的哈希是我们按名字判断的字体环境身份"
        },
    })
}

/// 命令行有没有说"不要缓存"。渲染链路与 `font scan` 共用这一个判据。
pub(crate) fn cache_mode(inv: &crate::args::Invocation) -> CacheMode {
    if inv.no_font_cache {
        CacheMode::Off
    } else {
        CacheMode::Use
    }
}

/// 进信封/报告的那一小块：只给结论与规模，完整 entries 由 `font scan` 的报告负责。
pub(crate) fn env_json(env: &FontEnv, build_ms: u128) -> serde_json::Value {
    serde_json::json!({
        "policy": env.policy,
        "digest": env.digest,
        "complete": !env.entries.iter().any(|e| e == "nosources"),
        "faces_used": env.faces_used,
        "faces_missing": env.faces_missing,
        "embedded_fonts": env.attachments,
        "scan_ms": build_ms,
        "cache": env.cache.label(),
        "cache_meaning": env.cache.meaning(),
        "roots": env.entries.iter().filter(|e| e.starts_with("root|")).collect::<Vec<_>>(),
        "meaning": "这份渲染的身份包含当时实际用到的每个 face 的字节哈希；字体被升级、卸载或换目录都会改变它（17 §7）",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASS: &str = "[V4+ Styles]\r\nFormat: Name, Fontname\r\nStyle: D,Arial\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,D,你好\r\n";

    /// `STK_FONTCACHE` 是进程级的，改它的测试必须与本模块里其它建库测试错开。
    fn env_lock() -> &'static std::sync::Mutex<()> {
        static L: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        L.get_or_init(|| std::sync::Mutex::new(()))
    }

    /// 创建包含极少量字体的临时目录，避免单元测试全量扫描系统数千个字体导致卡死。
    fn with_test_fonts<F: FnOnce()>(f: F) {
        let dir = std::env::temp_dir().join(format!("stk-cli-testfonts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(w) = std::env::var("WINDIR") {
            let sys = Path::new(&w).join("Fonts");
            if let Ok(rd) = std::fs::read_dir(&sys) {
                for ent in rd.flatten() {
                    let p = ent.path();
                    if p.extension().and_then(|s| s.to_str()) == Some("ttf") {
                        let _ = std::fs::copy(&p, dir.join(p.file_name().unwrap()));
                        break;
                    }
                }
            }
        }
        let old = std::env::var("STK_FONTS_DIR").ok();
        std::env::set_var("STK_FONTS_DIR", &dir);
        f();
        match old {
            Some(o) => std::env::set_var("STK_FONTS_DIR", o),
            None => std::env::remove_var("STK_FONTS_DIR"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 库只建一次：两次请求之间第二次不该再花建库的时间。
    #[test]
    fn the_library_is_built_once_per_scanner() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        with_test_fonts(|| {
            let mut s = Scanner::with_cache(CacheMode::Use);
            let (a, ms_a) = s.env_for(ASS).expect("测试字体目录该可用");
            let (b, ms_b) = s.env_for(ASS).expect("同上");
            assert_eq!(a.digest, b.digest, "同一份脚本、同一个环境必须给同一个身份");
            assert_eq!(ms_a, ms_b, "建库耗时只算一次，之后是缓存值");
            assert_eq!(a.faces_used + a.faces_missing, 1, "{a:?}");
        });
    }

    /// 读不到字体来源时不能停摆，但要被标成"不完整"，且不与"空库"同值。
    #[test]
    fn an_incomplete_environment_is_labeled_as_such() {
        let env = incomplete_env();
        let j = env_json(&env, 0);
        assert_eq!(j["complete"], false, "{j}");
        let empty_library = identify(&FontDb::scan(&[]), &[], &[]);
        assert_ne!(
            env.digest, empty_library.digest,
            "\"没有来源\"与\"来源里什么都没有\"必须是两个身份"
        );
    }

    /// 换个请求就等于换身份 —— 这正是补这条之前会漏的复用。
    #[test]
    fn a_different_requested_family_gives_a_different_identity() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        with_test_fonts(|| {
            let other = ASS.replace("Arial", "Times New Roman");
            let a = Scanner::with_cache(CacheMode::Use).env_for(ASS).unwrap().0;
            let b = Scanner::with_cache(CacheMode::Use)
                .env_for(&other)
                .unwrap()
                .0;
            assert_ne!(a.digest, b.digest, "{a:?} {b:?}");
        });
    }

    /// CLI 这一层的端到端：缓存被写成"指纹对得上、内容哈希是假的"那种过期状态时，
    /// `Scanner` 必须照样给出身份（不是报错），而且给出的身份与干净那条路**逐字节相同**。
    /// 这条就是 AI5 那笔性能债允许存在的前提。
    #[test]
    fn a_stale_font_cache_still_gives_the_same_identity() {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        with_test_fonts(|| {
            let dir =
                std::env::temp_dir().join(format!("stk-cli-fontcache-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let cpath = dir.join("cache.json");
            std::env::set_var("STK_FONTCACHE", &cpath);

            let clean = Scanner::with_cache(CacheMode::Use).env_for(ASS);
            std::env::remove_var("STK_FONTCACHE");
            let (clean_env, _) = clean.expect("第一次建库该成功");
            let raw = std::fs::read_to_string(&cpath).expect("建库该留下缓存");
            let stale = zero_out_hashes(&raw);
            assert_ne!(stale, raw, "没改到哈希，这条测试就没测到东西");
            std::fs::write(&cpath, &stale).unwrap();
            std::env::set_var("STK_FONTCACHE", &cpath);

            let (rescanned, _) = Scanner::with_cache(CacheMode::Use)
                .env_for(ASS)
                .expect("缓存过期不该让命令失败");
            std::env::remove_var("STK_FONTCACHE");

            assert_eq!(
                clean_env.digest, rescanned.digest,
                "重扫之后的身份必须与干净那条路一模一样，否则缓存就是在改答案"
            );
            if clean_env.faces_used > 0 {
                assert_eq!(
                    rescanned.cache,
                    CacheState::Refreshed,
                    "用到了 face 却没触发重扫，说明使用点复核根本没跑：{:?}",
                    rescanned.cache
                );
            }
            let j = env_json(&rescanned, 0);
            assert_eq!(j["cache"], "stale-refreshed", "{j}");
            std::fs::remove_dir_all(&dir).ok();
        });
    }

    /// 把 `"sha256":"<64 位十六进制>"` 的值换成全 0：指纹字段一个都不动。
    fn zero_out_hashes(json: &str) -> String {
        let key = "\"sha256\":\"";
        let mut out = String::with_capacity(json.len());
        let mut rest = json;
        while let Some(i) = rest.find(key) {
            let start = i + key.len();
            out.push_str(&rest[..start]);
            let tail = &rest[start..];
            let hex_len = tail.find('"').unwrap_or(0);
            if hex_len == 64 {
                out.push_str(&"0".repeat(64));
                rest = &tail[hex_len..];
            } else {
                rest = tail;
            }
        }
        out.push_str(rest);
        out
    }
}
