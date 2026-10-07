//! BDN XML 解析：流式单遍状态机（不收集 token、不递归），文档/节点/属性长度、事件数、
//! 嵌套深度全部由 [`Limits`] 封顶。
//!
//! 容忍（13 §7 的交换格式定位）：元素名与属性名大小写不敏感、允许命名空间前缀（按 local
//! name 匹配）、值前后空白忽略。方言差异全部映射到同一组槽位：
//! - `InTC`/`OutTC`：`<Event>` 属性或子元素；
//! - 尺寸/位置：`<Graphic>` 子元素优先，其次属性；
//! - 文件名：`<FileName>` / `<File>` 子元素，或 `FileName` / `Path` 属性，或 `<Graphic>` 裸文本；
//! - `X`/`Y` 缺失时退回 `HorizontalOffset`/`VerticalOffset`（PS3ISO 方言）。
//!
//! 严格（23 §XML）：`<!DOCTYPE>` 与任何非预定义实体（外部实体的入口）直接拒绝；裸 `&`、
//! 错配/未闭合标签由 quick-xml 报错并翻成 `STK-BDN-0052`；quick-xml 本身不解外部实体、不触网
//! （属性侧 `normalized_value` 只认 5 个预定义实体）。
//!
//! 时间：`BdnTimecode::parse` + `to_time(fps)` 反算 start/end；fps 走 `Fps::from_label`，
//! 因此 `"23.976"` 回到精确的 24000/1001，而不是十进制的 2997/125。

use core_model::canvas::Canvas;
use core_model::error::{Domain, Result, StkError};
use core_model::time::{BdnTimecode, Fps};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::XmlVersion;

use crate::limits::Limits;
use crate::model::{BdnDocument, BdnEvent, BdnGraphic, BdnMeta};

/// 生产默认上限（与 `crate::DEFAULT_LIMITS` 同值；写成 const 才能按引用交给状态机）。
const DEFAULTS: Limits = Limits {
    doc_bytes: crate::limits::DOC_BYTES_HARD_MAX,
    name_bytes: crate::limits::NAME_LEN_HARD_MAX,
    text_bytes: crate::limits::TEXT_LEN_HARD_MAX,
    events: crate::limits::EVENT_COUNT_HARD_MAX,
    depth: crate::limits::DEPTH_HARD_MAX,
};

/// 解析 BDN XML 文本。
pub fn parse(xml: &str) -> Result<BdnDocument> {
    parse_with(xml, &DEFAULTS)
}

/// 字节入口：非 UTF-8 必须是 `STK-BDN-0058`，而不是 panic 或乱码继续（fuzz 前提）。
pub fn parse_bytes(bytes: &[u8]) -> Result<BdnDocument> {
    let xml = std::str::from_utf8(bytes).map_err(|e| {
        StkError::simple(Domain::Bdn, 58, "BDN XML 不是合法 UTF-8")
            .with_detail(format!("有效前缀 {} 字节：{e}", e.valid_up_to()))
            .with_action("BDN 交换格式固定 UTF-8（13 §7）；请先转码")
    })?;
    parse(xml)
}

/// 可注入上限的入口：测试用小额度证明"超限 → `STK-BDN-0050`"，不必构造 16 MB 输入。
pub fn parse_with(xml: &str, lim: &Limits) -> Result<BdnDocument> {
    lim.check_doc_bytes(xml.len())?;
    let mut reader = Reader::from_str(xml);
    // 严格模式：未闭合/错配标签与裸 `&` 必须成为错误，不能被宽容吞掉。
    reader.config_mut().check_end_names = true;
    reader.config_mut().allow_unmatched_ends = false;
    reader.config_mut().allow_dangling_amp = false;
    reader.config_mut().expand_empty_elements = false;
    reader.config_mut().trim_text(false);

    let mut st = State::new(lim);
    loop {
        let ev = reader
            .read_event()
            .map_err(|e| st.syntax_error(&e, reader.error_position()))?;
        let pos = reader.buffer_position();
        match ev {
            Event::Eof => break,
            Event::DocType(d) => return Err(st.forbidden("DOCTYPE/DTD 声明", d.as_ref(), pos)),
            Event::GeneralRef(r) => {
                let name = r.as_ref().to_string();
                if r.is_char_ref() {
                    let ch = r
                        .resolve_char_ref()
                        .map_err(|e| st.value_error("字符引用", &name, &e.to_string(), pos))?;
                    if let Some(c) = ch {
                        st.push_text(&c.to_string(), pos)?;
                    }
                } else if let Some(pre) = predefined_entity(&name) {
                    st.push_text(pre, pos)?;
                } else {
                    // 非预定义实体是外部实体的入口；XML 安全要求禁止外部实体/网络解析。
                    return Err(st.forbidden("未定义实体（可能是外部实体）", &name, pos));
                }
            }
            Event::Text(t) => st.push_text(t.as_ref(), pos)?,
            Event::CData(t) => st.push_text(t.as_ref(), pos)?,
            Event::Start(s) => st.open(&s, pos)?,
            Event::Empty(s) => {
                st.open(&s, pos)?;
                let name = s.local_name();
                st.close(name.as_ref(), pos)?;
            }
            Event::End(e) => {
                let name = e.local_name();
                st.close(name.as_ref(), pos)?;
            }
            // Decl / Comment / PI 在 BDN 里没有语义：忽略，也不解析其内容。
            Event::Decl(_) | Event::Comment(_) | Event::PI(_) => {}
        }
    }
    st.finish()
}

/// 只认 5 个预定义实体（quick-xml 把实体拆成独立事件，这里负责还原）。
fn predefined_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => return None,
    })
}

/// 每个元素知道自己文本落进哪个槽；槽位互不相同，绝不靠"按出现顺序猜字段"。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    Ignored,
    VideoFormat,
    FrameRate,
    DropFrame,
    Name,
    Language,
    CanvasWidth,
    CanvasHeight,
    InTc,
    OutTc,
    FileName,
    GraphicWidth,
    GraphicHeight,
    GraphicX,
    GraphicY,
    GraphicOffsetX,
    GraphicOffsetY,
    /// `<Graphic>0001.png</Graphic>` 方言：元素文本本身就是文件名。
    GraphicText,
}

/// 一个事件的九个字段的收集槽（属性侧与子元素侧各一份，`build_event` 里按"子元素 > 属性"合并）。
#[derive(Default, Clone)]
struct Slots {
    in_tc: Option<String>,
    out_tc: Option<String>,
    file: Option<String>,
    width: Option<String>,
    height: Option<String>,
    x: Option<String>,
    y: Option<String>,
    offset_x: Option<String>,
    offset_y: Option<String>,
}

impl Slots {
    /// 同名槽位重复出现保留第一个：静默覆盖等于"最后一份尺寸赢"，正是尺寸漂移的温床。
    fn put(&mut self, slot: Slot, value: Option<String>) {
        let Some(value) = value else { return };
        let cell = match slot {
            Slot::InTc => &mut self.in_tc,
            Slot::OutTc => &mut self.out_tc,
            Slot::FileName => &mut self.file,
            Slot::GraphicWidth => &mut self.width,
            Slot::GraphicHeight => &mut self.height,
            Slot::GraphicX => &mut self.x,
            Slot::GraphicY => &mut self.y,
            Slot::GraphicOffsetX => &mut self.offset_x,
            Slot::GraphicOffsetY => &mut self.offset_y,
            _ => return,
        };
        if cell.is_none() {
            *cell = Some(value);
        }
    }

    /// 子元素侧优先于属性侧。
    fn merge(primary: &Slots, secondary: &Slots) -> Slots {
        Slots {
            in_tc: primary.in_tc.clone().or_else(|| secondary.in_tc.clone()),
            out_tc: primary.out_tc.clone().or_else(|| secondary.out_tc.clone()),
            file: primary.file.clone().or_else(|| secondary.file.clone()),
            width: primary.width.clone().or_else(|| secondary.width.clone()),
            height: primary.height.clone().or_else(|| secondary.height.clone()),
            x: primary.x.clone().or_else(|| secondary.x.clone()),
            y: primary.y.clone().or_else(|| secondary.y.clone()),
            offset_x: primary
                .offset_x
                .clone()
                .or_else(|| secondary.offset_x.clone()),
            offset_y: primary
                .offset_y
                .clone()
                .or_else(|| secondary.offset_y.clone()),
        }
    }
}

#[derive(Default)]
struct EventAcc {
    attr: Slots,
    leaf: Slots,
    graphic_text: Option<String>,
    /// 已开过 `<Graphic>`；再遇到就拒绝（08 §1 的 BDN IR 一个事件一个 graphic）。
    graphic_seen: bool,
    location: String,
}

impl EventAcc {
    fn err(&self, code: u16, title: &str, detail: &str) -> StkError {
        StkError::simple(Domain::Bdn, code, title)
            .with_detail(self.location.clone())
            .with_detail(detail.to_string())
    }

    /// 属性名 → 槽位（`<Event>` 与 `<Graphic>` 共用方言名表）。
    fn attr_slot(key: &str) -> Option<Slot> {
        Some(match key {
            "intc" => Slot::InTc,
            "outtc" => Slot::OutTc,
            "filename" | "file" | "path" => Slot::FileName,
            "width" => Slot::GraphicWidth,
            "height" => Slot::GraphicHeight,
            "x" => Slot::GraphicX,
            "y" => Slot::GraphicY,
            "horizontaloffset" => Slot::GraphicOffsetX,
            "verticaloffset" => Slot::GraphicOffsetY,
            _ => return None,
        })
    }

    fn leaf_slot(name: &str) -> Slot {
        match name {
            "intc" => Slot::InTc,
            "outtc" => Slot::OutTc,
            "filename" | "file" => Slot::FileName,
            "width" => Slot::GraphicWidth,
            "height" => Slot::GraphicHeight,
            "x" => Slot::GraphicX,
            "y" => Slot::GraphicY,
            "horizontaloffset" => Slot::GraphicOffsetX,
            "verticaloffset" => Slot::GraphicOffsetY,
            _ => Slot::Ignored,
        }
    }
}

struct Frame {
    name: String,
    slot: Slot,
    text: String,
}

#[derive(Default)]
struct MetaAcc {
    version: Option<String>,
    name: Option<String>,
    language: Option<String>,
    video_format: Option<String>,
    frame_rate: Option<String>,
    drop_frame: Option<String>,
    width: Option<String>,
    height: Option<String>,
}

/// 流式状态。`path` 栈顶是当前**正在打开的父元素**：`open` 在压栈前取父名、`close` 在弹栈后
/// 取父名，两处的 enclosing 语义因此一致。
struct State<'a> {
    lim: &'a Limits,
    stack: Vec<Frame>,
    /// 小写元素名，用于结构判定
    path: Vec<String>,
    /// 原样元素名，只用于诊断信息（XML 元素名大小写敏感，用户需要看到写下来的样子）
    raw_path: Vec<String>,
    saw_root: bool,
    root_closed: bool,
    meta: MetaAcc,
    cur: Option<EventAcc>,
    accs: Vec<EventAcc>,
}

impl<'a> State<'a> {
    fn new(lim: &'a Limits) -> State<'a> {
        State {
            lim,
            stack: Vec::new(),
            path: Vec::new(),
            raw_path: Vec::new(),
            saw_root: false,
            root_closed: false,
            meta: MetaAcc::default(),
            cur: None,
            accs: Vec::new(),
        }
    }

    fn loc(&self, pos: u64) -> String {
        if self.raw_path.is_empty() {
            format!("文档字节偏移 {pos}")
        } else {
            format!("元素路径 {}｜文档字节偏移 {pos}", self.raw_path.join("/"))
        }
    }

    fn syntax_error(&self, e: &quick_xml::errors::Error, pos: u64) -> StkError {
        StkError::simple(Domain::Bdn, 52, "BDN XML 语法错误")
            .with_detail(format!("{e}"))
            .with_detail(self.loc(pos))
            .with_action("文件不是 well-formed BDN；请回到源字幕重导，别手工补标签")
    }

    fn forbidden(&self, what: &str, detail: &str, pos: u64) -> StkError {
        let title = format!("BDN XML 含被禁止的构造：{what}");
        StkError::simple(Domain::Bdn, 51, &title)
            .with_detail(format!("内容片段 {detail:?}"))
            .with_detail(self.loc(pos))
            .with_action("BDN 交换格式不需要 DTD/外部实体，请移除（23 §XML）")
    }

    fn structural(&self, code: u16, title: &str, detail: &str, pos: u64) -> StkError {
        StkError::simple(Domain::Bdn, code, title)
            .with_detail(detail.to_string())
            .with_detail(self.loc(pos))
    }

    fn value_error(&self, what: &str, value: &str, why: &str, pos: u64) -> StkError {
        let title = format!("BDN {what} 值非法");
        StkError::simple(Domain::Bdn, 56, &title)
            .with_detail(format!("值 {value:?}"))
            .with_detail(why.to_string())
            .with_detail(self.loc(pos))
    }

    fn over_limit(&self, what: &str, limit: usize, got: usize, pos: u64) -> StkError {
        StkError::simple(Domain::Bdn, 50, "BDN 输入超出安全上限")
            .with_detail(format!("{what} 上限 {limit}，实际 {got}"))
            .with_detail(self.loc(pos))
            .with_action("该文件可能不是 BDN 或已被构造；请改用原始字幕源重新导出")
    }

    /// 元素**自身**的位置：诊断必须指向真正出问题的元素，而不是它的父元素
    /// （`loc()` 在 open 时只包含父元素，因为压栈发生在结构判定之后）。
    fn loc_self(&self, raw: &str, pos: u64) -> String {
        if self.raw_path.is_empty() {
            return format!("元素 <{raw}>｜文档字节偏移 {pos}");
        }
        format!(
            "元素路径 {}/{}｜文档字节偏移 {pos}",
            self.raw_path.join("/"),
            raw
        )
    }

    fn enclosing(&self) -> &str {
        self.path.last().map(String::as_str).unwrap_or("")
    }

    fn enclosing_parent(&self) -> &str {
        let Some(i) = self.path.len().checked_sub(2) else {
            return "";
        };
        self.path.get(i).map(String::as_str).unwrap_or("")
    }

    fn push_text(&mut self, raw: &str, pos: u64) -> Result<()> {
        // 先按单段长度拦，再按累计长度拦：两段合起来也能挡住"多段拼出的超长值"。
        self.lim.check_text_bytes(raw.len(), &self.loc(pos))?;
        let (slot, have) = match self.stack.last() {
            Some(f) => (f.slot, f.text.len()),
            None => return Ok(()),
        };
        if slot == Slot::Ignored {
            return Ok(());
        }
        let want = have
            .checked_add(raw.len())
            .ok_or_else(|| self.structural(50, "BDN 节点文本长度计数溢出", "", pos))?;
        if want > self.lim.text_bytes {
            return Err(self.over_limit("节点文本长度", self.lim.text_bytes, want, pos));
        }
        if let Some(frame) = self.stack.last_mut() {
            frame.text.push_str(raw);
        }
        Ok(())
    }

    /// 取一个属性值（大小写/前缀不敏感），并做长度与实体校验。
    fn attr(&mut self, s: &BytesStart<'_>, want: &str, pos: u64) -> Result<Option<String>> {
        for a in s.attributes() {
            let a = a.map_err(|e| self.structural(52, "BDN 属性无法解析", &e.to_string(), pos))?;
            let key = a.key.local_name().as_ref().to_ascii_lowercase();
            if key != want {
                continue;
            }
            self.lim.check_name_bytes(key.len(), &self.loc(pos))?;
            self.lim.check_text_bytes(a.value.len(), &self.loc(pos))?;
            let raw = a.value.to_string();
            // normalized_value 只解预定义实体；未定义实体 / 裸 & 在这里就变成错误。
            let v = a
                .normalized_value(XmlVersion::Explicit1_0)
                .map_err(|e| self.value_error("属性", &raw, &format!("{key}: {e}"), pos))?;
            return Ok(nonempty(v.into_owned()));
        }
        Ok(None)
    }

    /// 把元素上所有"认识的"属性灌进槽位集合。
    fn read_attr_slots(&mut self, s: &BytesStart<'_>, out: &mut Slots, pos: u64) -> Result<()> {
        let mut keys: Vec<String> = Vec::new();
        for a in s.attributes() {
            let a = a.map_err(|e| self.structural(52, "BDN 属性无法解析", &e.to_string(), pos))?;
            let key = a.key.local_name().as_ref().to_ascii_lowercase();
            if EventAcc::attr_slot(&key).is_some() {
                keys.push(key);
            }
        }
        for key in keys {
            let slot = EventAcc::attr_slot(&key);
            let value = self.attr(s, &key, pos)?;
            if let Some(slot) = slot {
                out.put(slot, value);
            }
        }
        Ok(())
    }

    fn open(&mut self, s: &BytesStart<'_>, pos: u64) -> Result<()> {
        let binding = s.local_name();
        let raw = binding.as_ref();
        self.lim.check_name_bytes(raw.len(), &self.loc(pos))?;
        let name = raw.to_ascii_lowercase();
        let depth = self
            .stack
            .len()
            .checked_add(1)
            .ok_or_else(|| self.structural(50, "BDN 元素深度计数溢出", "", pos))?;
        if depth > self.lim.depth {
            return Err(self.over_limit("元素嵌套深度", self.lim.depth, depth, pos));
        }

        if !self.saw_root {
            if name != "bdn" {
                return Err(StkError::simple(Domain::Bdn, 53, "根元素不是 <BDN>")
                    .with_detail(format!("实际 <{name}>"))
                    .with_detail(self.loc(pos))
                    .with_action("这不是 BDN 交换文件（03 §格式探测）"));
            }
            self.saw_root = true;
            self.meta.version = self.attr(s, "version", pos)?;
            for key in ["lang", "languagecode", "xml:lang"] {
                if let Some(v) = self.attr(s, key, pos)? {
                    self.meta.language = Some(v);
                    break;
                }
            }
            if let Some(v) = self.attr(s, "name", pos)? {
                self.meta.name = Some(v);
            }
        }

        let slot = self.slot_of(&name, raw, s, pos)?;
        self.path.push(name.clone());
        self.raw_path.push(raw.to_owned());
        self.stack.push(Frame {
            name,
            slot,
            text: String::new(),
        });
        Ok(())
    }

    /// 元素语义 + 它身上的属性；属性只在根 / `<Event>` / `<Graphic>` / `<Style>` 上有意义。
    fn slot_of(&mut self, name: &str, raw: &str, s: &BytesStart<'_>, pos: u64) -> Result<Slot> {
        let enc = self.enclosing().to_string();
        let enc_parent = self.enclosing_parent().to_string();

        if name == "event" {
            if enc != "bdnsubtitledata" {
                return Err(self.structural(
                    54,
                    "BDN Event 不在 BDNSubtitleData 下",
                    &format!("父元素 <{enc}>"),
                    pos,
                ));
            }
            if self.cur.is_some() {
                return Err(self.structural(
                    54,
                    "BDN 事件嵌套事件",
                    "Event 必须平铺在 BDNSubtitleData 下",
                    pos,
                ));
            }
            let count = self
                .accs
                .len()
                .checked_add(1)
                .ok_or_else(|| self.structural(50, "BDN 事件计数溢出", "", pos))?;
            self.lim.check_event_count(count, &self.loc(pos))?;
            let mut acc = EventAcc {
                location: self.loc_self(raw, pos),
                ..EventAcc::default()
            };
            self.read_attr_slots(s, &mut acc.attr, pos)?;
            self.cur = Some(acc);
            return Ok(Slot::Ignored);
        }

        if name == "graphic" {
            if enc != "event" {
                return Err(self.structural(
                    54,
                    "BDN Graphic 不在 Event 下",
                    &format!("父元素 <{enc}>"),
                    pos,
                ));
            }
            let loc = self.loc_self(raw, pos);
            // 先读属性到局部槽位，再合并：避免在借用了 self.cur 的同时再改 self。
            let mut tmp = Slots::default();
            self.read_attr_slots(s, &mut tmp, pos)?;
            let Some(acc) = self.cur.as_mut() else {
                return Ok(Slot::GraphicText);
            };
            if acc.graphic_seen {
                let title = "一个 BDN Event 含多个 Graphic";
                return Err(StkError::simple(Domain::Bdn, 57, title)
                    .with_detail(acc.location.clone())
                    .with_detail(loc)
                    .with_action(
                        "BDN IR（08 §1）一个事件只挂一个对象；多对象请拆成同时间窗的多个事件",
                    ));
            }
            acc.graphic_seen = true;
            // Graphic 自己的属性比 Event 上的同名属性更贴近对象，合并时优先。
            acc.attr = Slots::merge(&tmp, &acc.attr);
            return Ok(Slot::GraphicText);
        }

        if name == "style" || name == "bdnsubtitledata" {
            for key in ["languagecode", "lang"] {
                if let Some(v) = self.attr(s, key, pos)? {
                    if self.meta.language.is_none() {
                        self.meta.language = Some(v);
                    }
                    break;
                }
            }
            return Ok(Slot::Ignored);
        }

        if enc == "event" || enc == "graphic" {
            return Ok(EventAcc::leaf_slot(name));
        }
        if enc == "displayresolution" {
            return Ok(match name {
                "width" => Slot::CanvasWidth,
                "height" => Slot::CanvasHeight,
                _ => Slot::Ignored,
            });
        }
        if enc == "bdnformat" || (enc == "bdn" && enc_parent.is_empty()) {
            return Ok(match name {
                "videoformat" => Slot::VideoFormat,
                "framerate" => Slot::FrameRate,
                "dropframe" => Slot::DropFrame,
                "name" => Slot::Name,
                "languagecode" | "language" => Slot::Language,
                "width" => Slot::CanvasWidth,
                "height" => Slot::CanvasHeight,
                _ => Slot::Ignored,
            });
        }
        Ok(Slot::Ignored)
    }

    fn close(&mut self, end_name: &str, pos: u64) -> Result<()> {
        let Some(frame) = self.stack.pop() else {
            return Err(self.structural(52, "BDN 出现多余的结束标签", "", pos));
        };
        let opened_raw = self.raw_path.last().cloned().unwrap_or_default();
        self.path.pop();
        self.raw_path.pop();
        // 良构性检查：结束标签必须与栈顶元素同名。旧实现只看位置不看名字，
        // 于是 `</Event>` 缺失时会把父标签当成 Event 收掉，错误被推迟成误导性的结构错误。
        if !frame.name.eq_ignore_ascii_case(end_name) {
            return Err(
                StkError::simple(Domain::Bdn, 52, "BDN 结束标签与打开的元素不匹配")
                    .with_detail(format!("打开 <{opened_raw}>，却遇到 </{end_name}>"))
                    .with_detail(self.loc(pos))
                    .with_action("XML 必须良构（13 §7）；请修正结束标签或重新从原始字幕导出"),
            );
        }
        let value = nonempty(frame.text.trim().to_string());
        if frame.name == "bdn" && self.path.is_empty() {
            self.root_closed = true;
        }

        match frame.slot {
            Slot::Ignored => {}
            Slot::VideoFormat => self.meta.video_format = value,
            Slot::FrameRate => self.meta.frame_rate = value,
            Slot::DropFrame => self.meta.drop_frame = value,
            Slot::Name => {
                if self.meta.name.is_none() {
                    self.meta.name = value;
                }
            }
            Slot::Language => {
                if self.meta.language.is_none() {
                    self.meta.language = value;
                }
            }
            Slot::CanvasWidth => self.meta.width = value,
            Slot::CanvasHeight => self.meta.height = value,
            Slot::GraphicText => {
                // 裸文本只在没有别的文件名来源时使用（子元素 / 属性都优先）。
                if let Some(acc) = self.cur.as_mut() {
                    if acc.graphic_text.is_none() {
                        acc.graphic_text = value;
                    }
                }
            }
            other => {
                if let Some(acc) = self.cur.as_mut() {
                    acc.leaf.put(other, value);
                }
            }
        }

        if frame.name == "event" {
            if let Some(acc) = self.cur.take() {
                self.accs.push(acc);
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<BdnDocument> {
        if !self.saw_root {
            return Err(
                StkError::simple(Domain::Bdn, 53, "BDN 文档缺少根元素 <BDN>")
                    .with_detail("读到结尾仍未见到 BDN 根"),
            );
        }
        if !self.root_closed || !self.path.is_empty() {
            return Err(StkError::simple(Domain::Bdn, 52, "BDN 文档有未闭合的元素")
                .with_detail(format!("仍打开：{}", self.raw_path.join("/"))));
        }

        let fps = match self.meta.frame_rate.as_deref() {
            Some(raw) => Fps::from_label(raw).map_err(|e| {
                StkError::simple(Domain::Bdn, 56, "BDN FrameRate 无法解析为精确帧率")
                    .with_detail(format!("值 {raw:?}"))
                    .with_detail(format!("{e}"))
            })?,
            // 缺 FrameRate 不能猜 24：23.976 与 24 在长片上是两条不同的时间轴。
            None => Fps::NTSC_23976,
        };
        let drop_frame = match self.meta.drop_frame.as_deref() {
            Some(v) => parse_bool(v).ok_or_else(|| {
                StkError::simple(Domain::Bdn, 56, "BDN DropFrame 值非法")
                    .with_detail(format!("值 {v:?}"))
            })?,
            None => false,
        };
        let (width, height) = self.canvas_size()?;
        let canvas = Canvas::new(width, height).map_err(|_| {
            StkError::simple(Domain::Bdn, 64, "BDN DisplayResolution 非法")
                .with_detail(format!("{width}x{height}"))
        })?;
        let meta = BdnMeta {
            version: self.meta.version.unwrap_or_else(|| "0.93".to_string()),
            name: self.meta.name.unwrap_or_else(|| "BDN".to_string()),
            language: self.meta.language.unwrap_or_else(|| "eng".to_string()),
            video_format: self
                .meta
                .video_format
                .unwrap_or_else(|| video_format_label(width, height).to_string()),
            frame_rate: fps.as_rational(),
            drop_frame,
            width,
            height,
        };

        let mut events = Vec::with_capacity(self.accs.len());
        for acc in &self.accs {
            events.push(build_event(acc, fps, canvas)?);
        }
        Ok(BdnDocument::new(meta, events))
    }

    fn canvas_size(&self) -> Result<(u32, u32)> {
        let w = self.meta.width.as_deref().map(parse_u32);
        let h = self.meta.height.as_deref().map(parse_u32);
        if let (Some(Some(wv)), Some(Some(hv))) = (w, h) {
            return Ok((wv, hv));
        }
        if let Some(None) = w {
            return Err(
                StkError::simple(Domain::Bdn, 56, "BDN DisplayResolution Width 值非法")
                    .with_detail(format!("值 {:?}", self.meta.width)),
            );
        }
        if let Some(None) = h {
            return Err(
                StkError::simple(Domain::Bdn, 56, "BDN DisplayResolution Height 值非法")
                    .with_detail(format!("值 {:?}", self.meta.height)),
            );
        }
        // 单边缺失就整体退回 VideoFormat 预设，不臆造文件里没有的分辨率组合。
        match self
            .meta
            .video_format
            .as_deref()
            .and_then(video_format_size)
        {
            Some((wv, hv)) => Ok((wv, hv)),
            None => Ok((1920, 1080)),
        }
    }
}

/// 由一个事件的槽位造出 `BdnEvent`；所有缺失/非法都带元素路径与序号。
fn build_event(acc: &EventAcc, fps: Fps, canvas: Canvas) -> Result<BdnEvent> {
    let mut s = Slots::merge(&acc.leaf, &acc.attr);
    if s.file.is_none() {
        s.file = acc.graphic_text.clone();
    }
    let need = |slot: &Option<String>, what: &str| -> Result<String> {
        slot.clone().ok_or_else(|| {
            let title = format!("BDN 事件缺少 {what}");
            StkError::simple(Domain::Bdn, 55, &title)
                .with_detail(acc.location.clone())
                .with_action("BDN 交换文件必须自带完整时间码与几何；请重导")
        })
    };
    let in_raw = need(&s.in_tc, "InTC")?;
    let out_raw = need(&s.out_tc, "OutTC")?;
    let in_tc = BdnTimecode::parse(&in_raw)
        .map_err(|e| acc.err(56, "BDN InTC 时间码非法", &format!("{in_raw:?}: {e}")))?;
    let out_tc = BdnTimecode::parse(&out_raw)
        .map_err(|e| acc.err(56, "BDN OutTC 时间码非法", &format!("{out_raw:?}: {e}")))?;
    let file_name = need(&s.file, "FileName")?;
    let width = parse_u32(&need(&s.width, "Width")?)
        .ok_or_else(|| acc.err(56, "BDN Graphic Width 值非法", &file_name))?;
    let height = parse_u32(&need(&s.height, "Height")?)
        .ok_or_else(|| acc.err(56, "BDN Graphic Height 值非法", &file_name))?;
    // X/Y 优先，缺失才接受 HorizontalOffset/VerticalOffset 方言。
    let x_raw = s.x.clone().or(s.offset_x.clone());
    let y_raw = s.y.clone().or(s.offset_y.clone());
    let x = parse_i32(&need(&x_raw, "X")?)
        .ok_or_else(|| acc.err(56, "BDN Graphic X 值非法", &format!("{x_raw:?}")))?;
    let y = parse_i32(&need(&y_raw, "Y")?)
        .ok_or_else(|| acc.err(56, "BDN Graphic Y 值非法", &format!("{y_raw:?}")))?;

    let graphic = BdnGraphic::new(file_name, width, height, x, y)
        .map_err(|e| acc.err(64, "BDN Graphic 尺寸非法", &e.to_string()))?;
    // AT-BDN-001 的读侧守卫：声明尺寸必须放得进声明画布，越界文件拒绝并给位置。
    graphic.fits_in(canvas).map_err(|e| {
        StkError::simple(Domain::Bdn, 63, "BDN Graphic 越出 DisplayResolution 画布")
            .with_detail(acc.location.clone())
            .with_detail(format!("{e}"))
    })?;
    let ev = BdnEvent::from_timecodes(in_tc, out_tc, graphic, fps)
        .map_err(|e| acc.err(56, "BDN 事件时间码无法反算", &e.to_string()))?;
    if ev.end <= ev.start {
        return Err(acc.err(
            59,
            "BDN 事件结束不晚于开始",
            &format!("{} -> {}", ev.in_tc, ev.out_tc),
        ));
    }
    Ok(ev)
}

fn nonempty(v: String) -> Option<String> {
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

fn parse_u32(s: &str) -> Option<u32> {
    s.trim().parse::<u32>().ok()
}

fn parse_i32(s: &str) -> Option<i32> {
    s.trim().parse::<i32>().ok()
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Some(true),
        "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

fn video_format_size(label: &str) -> Option<(u32, u32)> {
    match label.trim().to_ascii_lowercase().as_str() {
        "720p" => Some((1280, 720)),
        "1080p" | "1080i" => Some((1920, 1080)),
        "2160p" | "4k" => Some((3840, 2160)),
        _ => None,
    }
}

fn video_format_label(w: u32, h: u32) -> &'static str {
    match (w, h) {
        (1280, 720) => "720p",
        (3840, 2160) => "2160p",
        (1920, 1080) => "1080p",
        _ => "custom",
    }
}
