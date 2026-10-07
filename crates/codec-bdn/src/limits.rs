//! 23 §XML / §文件 的资源上限。BDN 是"外部不可信输入"（用户从网上下的字幕包），
//! 所以上限必须在分配之前生效，而不是靠 OOM 兜底。
//!
//! 取值依据：真实 BDN 的 XML 一般在几百 KB ~ 3 MB（4 万事件、每事件十几个元素），
//! 所以文档上限留了约 5 倍余量；单节点/属性上限只需覆盖"最长 Windows 路径 + 时间码 + 名称"，
//! 不需要更大——超过这个量级的字符串不可能是合法 BDN 数据，只可能是攻击载荷。

use core_model::error::{Domain, Result, StkError};

/// 元素名（含前缀）的字节上限；合法 BDN 最长的是 `SecondaryGraphicParameter`（24）。
pub const NAME_LEN_HARD_MAX: usize = 64;
/// 属性值/文本节点字节上限；必须容纳 UNC/长路径的 `FileName`（260+ 中文路径 ≈ 780 字节）。
pub const TEXT_LEN_HARD_MAX: usize = 4096;
/// 事件数上限：一张超长片的 BDN 事件也远小于此（13 §7 的去重后更少）。
pub const EVENT_COUNT_HARD_MAX: usize = 200_000;
/// 文档字节上限。
pub const DOC_BYTES_HARD_MAX: usize = 16 * 1024 * 1024;
/// 元素嵌套深度上限：防递归下降爆栈（fuzz 前提）。
pub const DEPTH_HARD_MAX: usize = 32;

/// 可注入的上限集合：测试用小值证明"超限报 `STK-BDN-00xx`"，而不必构造 16 MB 输入。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub doc_bytes: usize,
    pub name_bytes: usize,
    pub text_bytes: usize,
    pub events: usize,
    pub depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            doc_bytes: DOC_BYTES_HARD_MAX,
            name_bytes: NAME_LEN_HARD_MAX,
            text_bytes: TEXT_LEN_HARD_MAX,
            events: EVENT_COUNT_HARD_MAX,
            depth: DEPTH_HARD_MAX,
        }
    }
}

/// 生产解析使用的上限（`parse` 走这一组）。
pub const DEFAULT_LIMITS: Limits = Limits {
    doc_bytes: DOC_BYTES_HARD_MAX,
    name_bytes: NAME_LEN_HARD_MAX,
    text_bytes: TEXT_LEN_HARD_MAX,
    events: EVENT_COUNT_HARD_MAX,
    depth: DEPTH_HARD_MAX,
};

/// 超限一律是 0051（"输入过大"），与语法错误区分开，便于 CLI 给出"文件损坏/不是 BDN"的提示。
fn too_large(what: &str, limit: usize, got: usize, pos: String) -> StkError {
    StkError::simple(Domain::Bdn, 50, "BDN 输入超出安全上限")
        .with_detail(format!("{what} 上限 {limit}，实际 {got}"))
        .with_detail(pos)
        .with_action("该文件可能不是 BDN 或已被损坏/构造；请改用原始字幕源重新导出")
}

impl Limits {
    pub(crate) fn check_doc_bytes(&self, got: usize) -> Result<()> {
        if got > self.doc_bytes {
            return Err(too_large(
                "文档字节数",
                self.doc_bytes,
                got,
                "位置：输入起点".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn check_name_bytes(&self, got: usize, pos: &str) -> Result<()> {
        if got > self.name_bytes {
            return Err(too_large(
                "元素/属性名长度",
                self.name_bytes,
                got,
                pos.to_string(),
            ));
        }
        Ok(())
    }

    pub(crate) fn check_text_bytes(&self, got: usize, pos: &str) -> Result<()> {
        if got > self.text_bytes {
            return Err(too_large(
                "节点文本/属性值长度",
                self.text_bytes,
                got,
                pos.to_string(),
            ));
        }
        Ok(())
    }

    pub(crate) fn check_event_count(&self, got: usize, pos: &str) -> Result<()> {
        if got > self.events {
            return Err(too_large("Event 数量", self.events, got, pos.to_string()));
        }
        Ok(())
    }
}
