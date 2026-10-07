//! 有损模型（08_FORMATS_AND_IR.md §4）：每次转换都要能回答“保留了什么、近似了什么、丢了什么”。

const MAX_SAMPLES: usize = 8;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LossReport {
    pub preserved: Vec<String>,
    pub approximated: Vec<String>,
    pub dropped: Vec<String>,
    pub nonstandard: Vec<String>,
    pub user_attention: Vec<String>,
    /// 同类问题可能成百上千，这里保留计数，列表只放样本。
    pub counts: Vec<(String, usize)>,
}

impl LossReport {
    pub fn new() -> LossReport {
        LossReport::default()
    }

    pub fn preserved(&mut self, what: &str) {
        push_unique(&mut self.preserved, what);
    }
    pub fn approximated(&mut self, what: &str) {
        push_unique(&mut self.approximated, what);
    }
    pub fn dropped(&mut self, what: &str) {
        self.bump("dropped", what);
    }
    pub fn nonstandard(&mut self, what: &str) {
        self.bump("nonstandard", what);
    }
    pub fn user_attention(&mut self, what: &str) {
        push_unique(&mut self.user_attention, what);
    }

    /// 计数型条目：样式/事件级别的信息可能有几百条，只留样本 + 总数。
    pub fn bump(&mut self, kind: &str, what: &str) {
        let key = format!("{kind}:{what}");
        if let Some((_, n)) = self.counts.iter_mut().find(|(k, _)| *k == key) {
            *n += 1;
            return;
        }
        self.counts.push((key, 1));
        let list = match kind {
            "dropped" => &mut self.dropped,
            "nonstandard" => &mut self.nonstandard,
            _ => &mut self.user_attention,
        };
        if list.len() < MAX_SAMPLES {
            list.push(what.to_string());
        }
    }

    pub fn is_lossless(&self) -> bool {
        self.approximated.is_empty() && self.dropped.is_empty() && self.nonstandard.is_empty()
    }

    pub fn count_for(&self, kind: &str, what: &str) -> usize {
        let key = format!("{kind}:{what}");
        self.counts
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, n)| *n)
            .unwrap_or(0)
    }

    /// 合并另一个报告的样本与计数（多输出 profile 汇总用）。
    ///
    /// 计数型列表（dropped/nonstandard）必须**样本和计数一起并**：只带计数的话，
    /// "别处丢过东西"进来了但样本是空的，`is_lossless()` 只看样本列表，
    /// 于是把有损的报告成无损（ISSUES Z8，CLI 的 `pgs encode` 实测撞到过）。
    pub fn merge(&mut self, other: &LossReport) {
        for v in &other.preserved {
            push_unique(&mut self.preserved, v);
        }
        for v in &other.approximated {
            push_unique(&mut self.approximated, v);
        }
        for v in &other.dropped {
            push_capped(&mut self.dropped, v);
        }
        for v in &other.nonstandard {
            push_capped(&mut self.nonstandard, v);
        }
        for v in &other.user_attention {
            push_unique(&mut self.user_attention, v);
        }
        for (k, n) in &other.counts {
            if let Some((_, m)) = self.counts.iter_mut().find(|(x, _)| x == k) {
                *m += n;
            } else {
                self.counts.push((k.clone(), *n));
            }
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "preserved={} approximated={} dropped={} nonstandard={} attention={}",
            self.preserved.len(),
            self.approximated.len(),
            self.dropped.len(),
            self.nonstandard.len(),
            self.user_attention.len()
        )
    }
}

fn push_unique(list: &mut Vec<String>, what: &str) {
    if !list.iter().any(|v| v == what) {
        list.push(what.to_string());
    }
}

/// 计数型列表的合并用插入：去重、且不超过样本上限（与 `bump` 的策略一致）。
fn push_capped(list: &mut Vec<String>, what: &str) {
    if !list.iter().any(|v| v == what) && list.len() < MAX_SAMPLES {
        list.push(what.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lossless_only_when_nothing_lost() {
        let mut r = LossReport::new();
        r.preserved("时间轴");
        assert!(r.is_lossless());
        r.approximated("字号取整");
        assert!(!r.is_lossless());
    }

    #[test]
    fn dedup_and_sample_cap() {
        let mut r = LossReport::new();
        for _ in 0..100 {
            r.dropped("drawing \\p");
        }
        assert_eq!(r.dropped.len(), 1);
        assert_eq!(r.count_for("dropped", "drawing \\p"), 100);
        for i in 0..30 {
            r.nonstandard(&format!("tag{i}"));
        }
        assert_eq!(r.nonstandard.len(), MAX_SAMPLES);
        assert_eq!(r.counts.len(), 1 + 30);
    }

    #[test]
    fn merge_adds_counts() {
        let mut a = LossReport::new();
        a.dropped("style");
        let mut b = LossReport::new();
        b.dropped("style");
        b.user_attention("编码不确定");
        a.merge(&b);
        assert_eq!(a.count_for("dropped", "style"), 2);
        assert_eq!(a.user_attention.len(), 1);
        assert!(!a.summary().is_empty());
    }

    #[test]
    fn merge_cannot_turn_a_lossy_report_into_a_lossless_one() {
        // 只并计数不并样本时，is_lossless()（看样本列表）会把有损说成无损
        let mut a = LossReport::new();
        let mut b = LossReport::new();
        b.dropped("事件被时间网格挪走");
        b.nonstandard("非标准写法");
        b.preserved("按链表顺序合成");
        a.merge(&b);
        assert!(
            !a.is_lossless(),
            "别处丢过东西，合并后必须仍然报告有损：{a:?}"
        );
        assert_eq!(a.count_for("dropped", "事件被时间网格挪走"), 1);
        assert_eq!(a.dropped.len(), 1);
        assert_eq!(a.nonstandard.len(), 1);
        assert_eq!(a.preserved.len(), 1, "保留项也是信息，合并时不能丢");
    }
}
